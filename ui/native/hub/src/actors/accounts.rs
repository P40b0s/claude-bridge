use std::time::Duration;

use async_trait::async_trait;

use claude_bridge_core::{auth, families, usage};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::process::Command;
use tokio::task::{JoinHandle, JoinSet};

use crate::i18n::tr;
use crate::signals::{
    AccountsState, ActivateAccount, AuthProvider, CancelLogin, ChatGptAccount, ClaudeCodeAccount,
    ErrorSeverity, ErrorSignal, LoginPhase, LoginRequest, LoginState, LogoutRequest,
    OpenClaudeUsage, RefreshUsage, Usage, UsageReset, UsageState, UsageWindow, UseLimitReset,
};

/// Token expiry shown in the UI moves; resend now and then.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// The ChatGPT OAuth login, Claude Code's account, and subscription limits.
pub struct AccountsActor {
    self_addr: Address<Self>,
    /// Limits being read; a tick does not start another while one runs.
    usage: Option<JoinHandle<()>>,
    /// The browser login in progress, if any. Only one callback server can run at a time.
    login: Option<Login>,
    /// Tells a finished login from one that was replaced or cancelled meanwhile.
    login_generation: u64,
    _owned_tasks: JoinSet<()>,
}

impl Actor for AccountsActor {}

struct Login {
    provider: AuthProvider,
    profile: String,
    generation: u64,
    task: JoinHandle<()>,
}

impl Login {
    /// Stops the login and waits until its cleanup (restoring the previous Claude account) is done,
    /// so the next login never races with it over the shared Claude Code files.
    async fn stop(self) -> (AuthProvider, String) {
        self.task.abort();
        let _ = self.task.await;
        (self.provider, self.profile)
    }
}

pub struct Refresh;

/// Internal: a login task ended.
pub struct LoginFinished {
    generation: u64,
    provider: AuthProvider,
    profile: String,
    result: Result<String, String>,
}

fn login_state(
    provider: AuthProvider,
    profile: &str,
    phase: LoginPhase,
    url: Option<String>,
    message: Option<String>,
) {
    LoginState {
        provider,
        profile: profile.into(),
        phase,
        url,
        message,
    }
    .send_signal_to_dart();
}

impl AccountsActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<LoginRequest, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<CancelLogin, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<LogoutRequest, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<ActivateAccount, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<RefreshUsage, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<UseLimitReset, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<OpenClaudeUsage, _>(self_addr.clone()));
        owned_tasks.spawn(Self::refresh_timer(self_addr.clone()));
        Self::send_state();
        Self {
            self_addr,
            usage: None,
            login: None,
            login_generation: 0,
            _owned_tasks: owned_tasks,
        }
    }

    async fn refresh_timer(mut self_addr: Address<Self>) {
        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        loop {
            interval.tick().await;
            if self_addr.notify(Refresh).await.is_err() {
                break;
            }
        }
    }

    fn send_state() {
        let mut chatgpt: Vec<_> = auth::openai::profiles()
            .into_iter()
            .map(|p| ChatGptAccount {
                id: p.id,
                active: p.active,
                subscription_until: p.tokens.subscription_until(),
                email: p.tokens.email,
                plan: p.tokens.plan,
                source: "claude-bridge".into(),
                expires_at: p.tokens.expires_at,
            })
            .collect();
        if chatgpt.is_empty()
            && let Some((t, source)) = auth::openai::load()
            && source == auth::openai::Source::CodexCli
        {
            chatgpt.push(ChatGptAccount {
                id: "codex-cli".into(),
                active: true,
                subscription_until: t.subscription_until(),
                email: t.email,
                plan: t.plan,
                source: source.as_str().into(),
                expires_at: t.expires_at,
            });
        }
        let claude_code = auth::claude_code::profiles()
            .into_iter()
            .map(|p| ClaudeCodeAccount {
                id: p.id,
                active: p.active,
                email: p.account.email,
                organization: p.account.organization,
                plan: p.account.plan,
                billing: p.account.billing,
                subscription_since: p.account.subscription_since,
            })
            .collect();
        AccountsState {
            claude_code,
            chatgpt,
        }
        .send_signal_to_dart();
    }

    fn refresh_usage(&mut self) {
        if self.usage.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        self.usage = Some(tokio::spawn(async move {
            // Disabled providers are not asked: a broken login there should stay out of sight.
            let codex = async {
                if families::is_disabled("codex") {
                    None
                } else {
                    usage::codex().await
                }
            };
            let codex = codex.await;
            let claude = if families::is_disabled("anthropic") {
                None
            } else {
                usage::load_claude()
            };
            UsageState {
                claude: claude.map(usage_signal),
                codex: codex.map(usage_signal),
            }
            .send_signal_to_dart();
        }));
    }

    async fn start_login(&mut self, provider: AuthProvider, profile: String) {
        if let Some(login) = self.login.take() {
            login.stop().await;
        }
        self.login_generation += 1;
        let generation = self.login_generation;
        let mut addr = self.self_addr.clone();
        let task_profile = profile.clone();
        let task = tokio::spawn(async move {
            let result = match provider {
                AuthProvider::ChatGpt => login_chatgpt(provider, &task_profile).await,
                AuthProvider::Claude => login_claude(&task_profile).await,
            }
            .map_err(|e| format!("{e:#}"));
            let _ = addr
                .notify(LoginFinished {
                    generation,
                    provider,
                    profile: task_profile,
                    result,
                })
                .await;
        });
        self.login = Some(Login {
            provider,
            profile,
            generation,
            task,
        });
    }
}

async fn login_chatgpt(provider: AuthProvider, profile: &str) -> anyhow::Result<String> {
    auth::profiles::validate(profile)?;
    let http = claude_bridge_core::exits::outside_client("codex").await?;
    let browser_profile = format!("chatgpt-{profile}");
    let on_url = move |url: &str| {
        if let Err(e) =
            claude_bridge_core::browser::open_with_profile(url, "codex", Some(&browser_profile))
        {
            ErrorSignal::error(&e);
        }
        login_state(
            provider,
            profile,
            LoginPhase::Waiting,
            Some(url.to_string()),
            None,
        );
    };
    let tokens = auth::openai::login_profile(&http, profile, on_url).await?;
    Ok(tokens
        .email
        .unwrap_or_else(|| tr("неизвестный аккаунт", "unknown account").into()))
}

struct RestoreClaudeLogin(Option<String>);

impl Drop for RestoreClaudeLogin {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            let _ = auth::claude_code::restore_after_failed_login(Some(&previous));
        }
    }
}

async fn login_claude(profile: &str) -> anyhow::Result<String> {
    auth::profiles::validate(profile)?;
    let mut restore = RestoreClaudeLogin(auth::claude_code::prepare_login(profile)?);
    login_state(
        AuthProvider::Claude,
        profile,
        LoginPhase::Waiting,
        None,
        None,
    );
    let mut command = Command::new("claude");
    command.kill_on_drop(true).args(["auth", "login"]);
    // A running bridge may still have the placeholder inherited from before an OAuth login.
    // Do not let it shadow the credentials that `claude auth login` is about to create.
    if std::env::var("ANTHROPIC_AUTH_TOKEN").as_deref()
        == Ok(claude_bridge_core::providers::anthropic::PLACEHOLDER_TOKEN)
    {
        command.env_remove("ANTHROPIC_AUTH_TOKEN");
    }
    for (key, value) in claude_bridge_core::claude_settings::managed_env() {
        command.env(key, value);
    }
    command.env(
        claude_bridge_core::browser::PROFILE_OVERRIDE_KEY,
        format!("claude-{profile}"),
    );
    let output = command.output().await;
    let result = match output {
        Ok(output) if output.status.success() => auth::claude_code::finish_login(profile),
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(anyhow::anyhow!(if error.is_empty() {
                format!("claude auth login exited with {}", output.status)
            } else {
                error
            }))
        }
        Err(e) => Err(anyhow::Error::new(e).context("starting `claude auth login`")),
    };
    result?;
    restore.0 = None;
    Ok(auth::claude_code::account()
        .and_then(|a| a.email)
        .unwrap_or_else(|| tr("неизвестный аккаунт", "unknown account").into()))
}

#[async_trait]
impl Notifiable<Refresh> for AccountsActor {
    async fn notify(&mut self, _: Refresh, _: &Context<Self>) {
        Self::send_state();
        self.refresh_usage();
    }
}

#[async_trait]
impl Notifiable<RefreshUsage> for AccountsActor {
    async fn notify(&mut self, _: RefreshUsage, _: &Context<Self>) {
        self.refresh_usage();
    }
}

const CLAUDE_USAGE_PAGE: &str = "https://claude.ai/settings/usage";

#[async_trait]
impl Notifiable<OpenClaudeUsage> for AccountsActor {
    async fn notify(&mut self, _: OpenClaudeUsage, _: &Context<Self>) {
        let profile = format!("claude-{}", auth::claude_code::active_profile());
        if let Err(e) = claude_bridge_core::browser::open_with_profile(
            CLAUDE_USAGE_PAGE,
            "anthropic",
            Some(&profile),
        ) {
            ErrorSignal::error(&e);
        }
    }
}

/// Internal: a reset was used; read the limits again.
pub struct ResetUsed;

#[async_trait]
impl Notifiable<ResetUsed> for AccountsActor {
    async fn notify(&mut self, _: ResetUsed, _: &Context<Self>) {
        self.refresh_usage();
    }
}

#[async_trait]
impl Notifiable<UseLimitReset> for AccountsActor {
    async fn notify(&mut self, msg: UseLimitReset, _: &Context<Self>) {
        let mut addr = self.self_addr.clone();
        tokio::spawn(async move {
            match usage::use_codex_reset(&msg.id).await {
                Ok(usage::ResetOutcome::Reset(_)) => ErrorSignal::send(
                    ErrorSeverity::Success,
                    tr("Лимиты Codex сброшены", "Codex limits reset"),
                ),
                Ok(usage::ResetOutcome::NothingToReset) => ErrorSignal::send(
                    ErrorSeverity::Warning,
                    tr(
                        "Сбрасывать нечего: лимиты не израсходованы. Сброс не потрачен",
                        "Nothing to reset: the limits are not used up. The reset was not spent",
                    ),
                ),
                Ok(usage::ResetOutcome::NoCredit) => ErrorSignal::send(
                    ErrorSeverity::Warning,
                    tr(
                        "Этого сброса больше нет",
                        "This reset is no longer available",
                    ),
                ),
                Ok(usage::ResetOutcome::AlreadyRedeemed) => ErrorSignal::send(
                    ErrorSeverity::Warning,
                    tr("Этот сброс уже использован", "This reset was already used"),
                ),
                Ok(usage::ResetOutcome::Other(reply)) => ErrorSignal::send(
                    ErrorSeverity::Warning,
                    format!(
                        "{}: {reply}",
                        tr("Непонятный ответ на сброс", "Unexpected reset reply")
                    ),
                ),
                Err(e) => ErrorSignal::send(
                    ErrorSeverity::Error,
                    format!("{}: {e:#}", tr("Сброс не удался", "Reset failed")),
                ),
            }
            let _ = addr.notify(ResetUsed).await;
        });
    }
}

#[async_trait]
impl Notifiable<LoginRequest> for AccountsActor {
    async fn notify(&mut self, msg: LoginRequest, _: &Context<Self>) {
        self.start_login(msg.provider, msg.profile).await;
    }
}

#[async_trait]
impl Notifiable<CancelLogin> for AccountsActor {
    async fn notify(&mut self, _: CancelLogin, _: &Context<Self>) {
        if let Some(login) = self.login.take() {
            let (provider, profile) = login.stop().await;
            login_state(provider, &profile, LoginPhase::Cancelled, None, None);
        }
    }
}

#[async_trait]
impl Notifiable<LoginFinished> for AccountsActor {
    async fn notify(&mut self, msg: LoginFinished, _: &Context<Self>) {
        // Sent just before its task was replaced or cancelled: that login no longer counts.
        if self
            .login
            .as_ref()
            .is_none_or(|l| l.generation != msg.generation)
        {
            return;
        }
        self.login = None;
        if msg.provider == AuthProvider::Claude {
            sync_claude_settings();
        }
        match msg.result {
            Ok(who) => {
                login_state(
                    msg.provider,
                    &msg.profile,
                    LoginPhase::Succeeded,
                    None,
                    Some(who.clone()),
                );
                ErrorSignal::send(
                    ErrorSeverity::Success,
                    format!("{}: {who}", tr("Вход выполнен", "Logged in")),
                );
            }
            Err(e) => {
                login_state(
                    msg.provider,
                    &msg.profile,
                    LoginPhase::Failed,
                    None,
                    Some(e.clone()),
                );
                ErrorSignal::send(
                    ErrorSeverity::Error,
                    format!("{}: {e}", tr("Вход не удался", "Login failed")),
                );
            }
        }
        Self::send_state();
        self.refresh_usage();
    }
}

#[async_trait]
impl Notifiable<LogoutRequest> for AccountsActor {
    async fn notify(&mut self, msg: LogoutRequest, _: &Context<Self>) {
        let result = match msg.provider {
            AuthProvider::ChatGpt if msg.profile == "codex-cli" => {
                ErrorSignal::send(
                    ErrorSeverity::Warning,
                    tr(
                        "Используются токены Codex CLI (~/.codex/auth.json); выйдите через `codex logout`",
                        "These are Codex CLI tokens (~/.codex/auth.json); log out with `codex logout`",
                    ),
                );
                Ok(false)
            }
            AuthProvider::ChatGpt => auth::openai::logout_profile(&msg.profile),
            AuthProvider::Claude if auth::claude_code::active_profile() == msg.profile => {
                let output = Command::new("claude")
                    .args(["auth", "logout"])
                    .output()
                    .await;
                match output {
                    Ok(output) if output.status.success() => {
                        auth::claude_code::clear_active(&msg.profile).map(|_| true)
                    }
                    Ok(output) => Err(anyhow::anyhow!(
                        String::from_utf8_lossy(&output.stderr).trim().to_string()
                    )),
                    Err(e) => Err(anyhow::Error::new(e).context("starting `claude auth logout`")),
                }
            }
            AuthProvider::Claude => auth::claude_code::remove(&msg.profile),
        };
        if msg.provider == AuthProvider::Claude {
            sync_claude_settings();
        }
        if let Err(e) = result {
            ErrorSignal::error(&e);
        }
        Self::send_state();
    }
}

#[async_trait]
impl Notifiable<ActivateAccount> for AccountsActor {
    async fn notify(&mut self, msg: ActivateAccount, _: &Context<Self>) {
        let result = match msg.provider {
            AuthProvider::Claude => auth::claude_code::activate(&msg.profile),
            AuthProvider::ChatGpt if msg.profile == "codex-cli" => Err(anyhow::anyhow!(
                "Codex CLI fallback is active only when no claude-bridge ChatGPT profile is selected"
            )),
            AuthProvider::ChatGpt => auth::openai::activate(&msg.profile),
        };
        if msg.provider == AuthProvider::Claude {
            sync_claude_settings();
        }
        match result {
            Ok(()) => ErrorSignal::send(
                ErrorSeverity::Success,
                format!(
                    "{}: {}",
                    tr("Аккаунт активирован", "Account activated"),
                    msg.profile
                ),
            ),
            Err(e) => ErrorSignal::error(&e),
        }
        Self::send_state();
        self.refresh_usage();
    }
}

/// The placeholder token in Claude Code's settings.json belongs there only without a login; fix it
/// right away instead of on the next proxy poll, so a `claude` started meanwhile gets no 424.
fn sync_claude_settings() {
    if let Err(e) = claude_bridge_core::claude_settings::sync_auth(crate::configuration::port()) {
        tracing::warn!("{e:#}");
    }
}

fn usage_signal(u: usage::Usage) -> Usage {
    Usage {
        windows: u
            .windows
            .into_iter()
            .map(|w| UsageWindow {
                id: w.id,
                model: w.model,
                window_minutes: w.window_minutes,
                used_percent: w.used_percent,
                resets_at: w.resets_at,
            })
            .collect(),
        limited: u.limited,
        observed_at: u.observed_at,
        error: u.error,
        resets: u
            .resets
            .into_iter()
            .map(|r| UsageReset {
                id: r.id,
                title: r.title,
                expires_at: r.expires_at,
            })
            .collect(),
    }
}
