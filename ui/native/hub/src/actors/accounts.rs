use std::time::Duration;

use async_trait::async_trait;

use claude_bridge_core::{auth, families, usage};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::{JoinHandle, JoinSet};

use crate::i18n::tr;
use crate::signals::{
    AccountsState, AuthProvider, CancelLogin, ChatGptAccount, ClaudeCodeAccount, ErrorSeverity,
    ErrorSignal, LoginPhase, LoginRequest, LoginState, LogoutRequest, RefreshUsage,
    Usage, UsageState, UsageWindow,
};

/// Token expiry shown in the UI moves; resend now and then.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// The ChatGPT OAuth login, Claude Code's account, and subscription limits.
pub struct AccountsActor {
    self_addr: Address<Self>,
    /// Limits being read; a tick does not start another while one runs.
    usage: Option<JoinHandle<()>>,
    /// The browser login in progress, if any. Only one callback server can run at a time.
    login: Option<(AuthProvider, JoinHandle<()>)>,
    _owned_tasks: JoinSet<()>,
}

impl Actor for AccountsActor {}

pub struct Refresh;

/// Internal: a login task ended.
pub struct LoginFinished {
    provider: AuthProvider,
    result: Result<String, String>,
}

fn login_state(provider: AuthProvider, phase: LoginPhase, url: Option<String>, message: Option<String>) {
    LoginState {
        provider,
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
        owned_tasks.spawn(super::forward::<RefreshUsage, _>(self_addr.clone()));
        owned_tasks.spawn(Self::refresh_timer(self_addr.clone()));
        Self::send_state();
        Self {
            self_addr,
            usage: None,
            login: None,
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
        let chatgpt = auth::openai::load().map(|(t, source)| ChatGptAccount {
            subscription_until: t.subscription_until(),
            email: t.email,
            plan: t.plan,
            source: source.as_str().into(),
            expires_at: t.expires_at,
        });
        let claude_code = auth::claude_code::account().map(|a| ClaudeCodeAccount {
            email: a.email,
            organization: a.organization,
            plan: a.plan,
            billing: a.billing,
            subscription_since: a.subscription_since,
        });
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

    fn start_login(&mut self, provider: AuthProvider) {
        if let Some((_, task)) = self.login.take() {
            task.abort();
        }
        let mut addr = self.self_addr.clone();
        let task = tokio::spawn(async move {
            let on_url = move |url: &str| {
                auth::open_browser(url);
                login_state(provider, LoginPhase::Waiting, Some(url.to_string()), None);
            };
            let result = async {
                // The token exchange goes out the codex exit, like every other ChatGPT call.
                let http = claude_bridge_core::exits::outside_client("codex").await?;
                let who = match provider {
                    AuthProvider::ChatGpt => auth::openai::login(&http, on_url).await?.email,
                };
                anyhow::Ok(
                    who.unwrap_or_else(|| tr("неизвестный аккаунт", "unknown account").into()),
                )
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = addr
                .notify(LoginFinished { provider, result })
                .await;
        });
        self.login = Some((provider, task));
    }
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

#[async_trait]
impl Notifiable<LoginRequest> for AccountsActor {
    async fn notify(&mut self, msg: LoginRequest, _: &Context<Self>) {
        self.start_login(msg.provider);
    }
}

#[async_trait]
impl Notifiable<CancelLogin> for AccountsActor {
    async fn notify(&mut self, _: CancelLogin, _: &Context<Self>) {
        if let Some((provider, task)) = self.login.take() {
            task.abort();
            login_state(provider, LoginPhase::Cancelled, None, None);
        }
    }
}

#[async_trait]
impl Notifiable<LoginFinished> for AccountsActor {
    async fn notify(&mut self, msg: LoginFinished, _: &Context<Self>) {
        self.login = None;
        match msg.result {
            Ok(who) => {
                login_state(msg.provider, LoginPhase::Succeeded, None, Some(who.clone()));
                ErrorSignal::send(
                    ErrorSeverity::Success,
                    format!("{}: {who}", tr("Вход выполнен", "Logged in")),
                );
            }
            Err(e) => {
                login_state(msg.provider, LoginPhase::Failed, None, Some(e.clone()));
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
        let removed = match msg.provider {
            AuthProvider::ChatGpt => auth::openai::logout(),
        };
        if !removed && msg.provider == AuthProvider::ChatGpt && auth::openai::load().is_some() {
            ErrorSignal::send(
                ErrorSeverity::Warning,
                tr(
                    "Используются токены Codex CLI (~/.codex/auth.json); выйдите через `codex logout`",
                    "These are Codex CLI tokens (~/.codex/auth.json); log out with `codex logout`",
                ),
            );
        }
        Self::send_state();
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
    }
}
