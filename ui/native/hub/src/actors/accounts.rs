use std::time::Duration;

use async_trait::async_trait;
use std::sync::Arc;

use claude_bridge_core::auth::google::GoogleAuth;
use claude_bridge_core::{auth, families, server, usage};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::{JoinHandle, JoinSet};

use crate::i18n::tr;
use crate::signals::{
    AccountsState, AuthProvider, CancelLogin, ChatGptAccount, ClaudeCodeAccount, ErrorSeverity,
    ErrorSignal, GoogleAccount, LoginPhase, LoginRequest, LoginState, LogoutRequest, RefreshUsage,
    Usage, UsageState, UsageWindow,
};

/// Token expiry shown in the UI moves; resend now and then.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// OAuth logins (ChatGPT, Google 1 and 2), their status and subscription limits.
pub struct AccountsActor {
    self_addr: Address<Self>,
    google_auth: Arc<GoogleAuth>,
    /// Limits being read; a tick does not start another while one runs.
    usage: Option<JoinHandle<()>>,
    /// The browser login in progress, if any. Only one callback server can run at a time.
    login: Option<(AuthProvider, i32, JoinHandle<()>)>,
    _owned_tasks: JoinSet<()>,
}

impl Actor for AccountsActor {}

pub struct Refresh;

/// Internal: a login task ended.
pub struct LoginFinished {
    provider: AuthProvider,
    account: i32,
    result: Result<String, String>,
}

fn login_state(
    provider: AuthProvider,
    account: i32,
    phase: LoginPhase,
    url: Option<String>,
    message: Option<String>,
) {
    LoginState {
        provider,
        account,
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
            google_auth: Arc::default(),
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
        let google = (1..=2u8)
            .filter_map(|account| {
                auth::google::load(account).map(|t| GoogleAccount {
                    account: i32::from(account),
                    email: t.email,
                    project_id: t.project_id,
                    expires_at: t.expires_at,
                })
            })
            .collect();
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
            google,
        }
        .send_signal_to_dart();
    }

    fn refresh_usage(&mut self) {
        if self.usage.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        let google_auth = self.google_auth.clone();
        self.usage = Some(tokio::spawn(async move {
            let Ok(http) = server::http_client() else {
                return;
            };
            // Disabled providers are not asked: a broken login there should stay out of sight.
            let gemini = futures_util::future::join_all(
                (1..=2u8)
                    .filter(|&account| {
                        !families::is_disabled("gemini") && auth::google::load(account).is_some()
                    })
                    .map(|account| usage::gemini(&http, &google_auth, account)),
            );
            let codex = async {
                if families::is_disabled("codex") {
                    None
                } else {
                    usage::codex(&http).await
                }
            };
            let (codex, gemini) = tokio::join!(codex, gemini);
            let claude = if families::is_disabled("anthropic") {
                None
            } else {
                usage::load_claude()
            };
            // Reading Gemini limits may have refreshed a Google token; show its new expiry.
            Self::send_state();
            UsageState {
                claude: claude.map(usage_signal),
                codex: codex.map(usage_signal),
                gemini: gemini.into_iter().map(usage_signal).collect(),
            }
            .send_signal_to_dart();
        }));
    }

    fn start_login(&mut self, provider: AuthProvider, account: i32) {
        if let Some((_, _, task)) = self.login.take() {
            task.abort();
        }
        let mut addr = self.self_addr.clone();
        let task = tokio::spawn(async move {
            let on_url = move |url: &str| {
                auth::open_browser(url);
                login_state(
                    provider,
                    account,
                    LoginPhase::Waiting,
                    Some(url.to_string()),
                    None,
                );
            };
            let result = async {
                let http = server::http_client()?;
                let who = match provider {
                    AuthProvider::ChatGpt => auth::openai::login(&http, on_url).await?.email,
                    AuthProvider::Google => {
                        let slot = u8::try_from(account).unwrap_or(1);
                        auth::google::login(&http, slot, on_url).await?.email
                    }
                };
                anyhow::Ok(
                    who.unwrap_or_else(|| tr("неизвестный аккаунт", "unknown account").into()),
                )
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = addr
                .notify(LoginFinished {
                    provider,
                    account,
                    result,
                })
                .await;
        });
        self.login = Some((provider, account, task));
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
        self.start_login(msg.provider, msg.account);
    }
}

#[async_trait]
impl Notifiable<CancelLogin> for AccountsActor {
    async fn notify(&mut self, _: CancelLogin, _: &Context<Self>) {
        if let Some((provider, account, task)) = self.login.take() {
            task.abort();
            login_state(provider, account, LoginPhase::Cancelled, None, None);
        }
    }
}

#[async_trait]
impl Notifiable<LoginFinished> for AccountsActor {
    async fn notify(&mut self, msg: LoginFinished, _: &Context<Self>) {
        self.login = None;
        match msg.result {
            Ok(who) => {
                login_state(
                    msg.provider,
                    msg.account,
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
                    msg.account,
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
        let removed = match msg.provider {
            AuthProvider::ChatGpt => auth::openai::logout(),
            AuthProvider::Google => auth::google::logout(u8::try_from(msg.account).unwrap_or(1)),
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
        account: u.account.map(i32::from),
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
