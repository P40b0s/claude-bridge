use std::time::Duration;

use async_trait::async_trait;
use claude_bridge_core::claude_settings::{self, Connection};
use claude_bridge_core::{auth, control, families};
use futures_util::StreamExt;
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::{JoinHandle, JoinSet};

use crate::configuration;
use crate::i18n::{is_russian, tr};
use crate::signals::{
    ConnectClaudeCode, ErrorSeverity, ErrorSignal, ModelInfo, PinModel, ProxyAction, ProxyCommand,
    ProxyState, RequestLogReset, RequestLogged,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Watches the background proxy, starts and stops it, pins models, follows its request
/// log and points Claude Code at it.
pub struct ProxyActor {
    last_state: Option<ProxyState>,
    busy: bool,
    /// Autostart was considered; it runs once, on the first poll.
    autostart_checked: bool,
    /// Request log subscription and the proxy PID it belongs to.
    events: Option<(u32, JoinHandle<()>)>,
    _owned_tasks: JoinSet<()>,
}

impl Actor for ProxyActor {}

/// Internal: poll the proxy now.
pub struct Poll;

impl ProxyActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<ProxyCommand, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<PinModel, _>(self_addr.clone()));
        owned_tasks.spawn(super::forward::<ConnectClaudeCode, _>(self_addr.clone()));
        owned_tasks.spawn(Self::poll_timer(self_addr));
        Self {
            last_state: None,
            busy: false,
            autostart_checked: false,
            events: None,
            _owned_tasks: owned_tasks,
        }
    }

    async fn poll_timer(mut self_addr: Address<Self>) {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        loop {
            interval.tick().await;
            if self_addr.notify(Poll).await.is_err() {
                break;
            }
        }
    }

    async fn refresh(&mut self) -> Option<control::Health> {
        let port = configuration::port();
        let health = control::health(port).await;
        self.follow_events(port, health.as_ref());
        // Logging in to Claude Code or out of it changes whether the placeholder token belongs there.
        if let Err(e) = claude_settings::sync_auth(port) {
            tracing::warn!("{e:#}");
        }
        let connection = claude_settings::connection(port);
        let state = ProxyState {
            running: health.is_some(),
            port: i32::from(port),
            pid: health.as_ref().map(|h| i64::from(h.pid)),
            started_at: health.as_ref().map(|h| h.started_at),
            active: health
                .as_ref()
                .and_then(|h| h.active.as_ref())
                .map(ModelInfo::from),
            pinned: health
                .as_ref()
                .and_then(|h| h.pinned.as_ref())
                .map(ModelInfo::from),
            cli_path: configuration::cli_path().map(|p| p.display().to_string()),
            busy: self.busy,
            claude_code_connected: connection == Connection::Connected,
            claude_code_other_url: match connection {
                Connection::Other(url) => Some(url),
                _ => None,
            },
            claude_code_settings: claude_settings::settings_file().display().to_string(),
            autostart: configuration::autostart(),
        };
        if self.last_state.as_ref() != Some(&state) {
            state.clone().send_signal_to_dart();
            self.last_state = Some(state);
        }
        health
    }

    /// Keeps exactly one request log subscription, to the proxy process currently running.
    fn follow_events(&mut self, port: u16, health: Option<&control::Health>) {
        let pid = health.map(|h| h.pid);
        let current = self
            .events
            .as_ref()
            .filter(|(_, task)| !task.is_finished())
            .map(|(pid, _)| *pid);
        if current == pid {
            return;
        }
        if let Some((_, task)) = self.events.take() {
            task.abort();
        }
        let Some(pid) = pid else { return };
        RequestLogReset {}.send_signal_to_dart();
        let task = tokio::spawn(async move {
            match control::events(port).await {
                Ok(stream) => {
                    let mut stream = std::pin::pin!(stream);
                    while let Some(record) = stream.next().await {
                        RequestLogged {
                            entry: record.into(),
                        }
                        .send_signal_to_dart();
                    }
                }
                Err(e) => tracing::warn!("request log subscription failed: {e:#}"),
            }
        });
        self.events = Some((pid, task));
    }

    /// Proxy default when the UI starts it: the first provider the user can actually use.
    fn default_model() -> String {
        let on = |family: &str| !families::is_disabled(family);
        if on("anthropic")
            && (auth::claude_code::account().is_some()
                || configuration::setting("ANTHROPIC_API_KEY").is_some())
        {
            "anthropic:claude-opus-5-5".into()
        } else if on("codex") && auth::openai::load().is_some() {
            "codex-oauth:gpt-5.6-sol".into()
        } else if on("gemini") && auth::google::load(1).is_some() {
            "gemini-oauth:gemini-3.1-pro-preview".into()
        } else if on("gemini") && configuration::setting("GEMINI_API_KEY").is_some() {
            "gemini:gemini-3.1-pro-preview".into()
        } else if on("openai-compat")
            && configuration::setting("OPENAI_COMPAT_BASE_URL").is_some()
            && let Some(model) = configuration::setting("OPENAI_COMPAT_DEFAULT_MODEL")
        {
            format!("openai-compat:{model}")
        } else {
            "openai:gpt-5.5".into()
        }
    }

    async fn run_command(&mut self, action: ProxyAction) -> anyhow::Result<()> {
        let port = configuration::port();
        match action {
            ProxyAction::Stop => {
                control::stop(port).await?;
                ErrorSignal::send(
                    ErrorSeverity::Info,
                    tr("Прокси остановлен", "Proxy stopped"),
                );
            }
            ProxyAction::Start | ProxyAction::Restart => {
                let cli = configuration::cli_path().ok_or_else(|| {
                    anyhow::anyhow!(
                        tr(
                            "Не найден бинарник claude-bridge: укажите путь в настройках",
                            "claude-bridge binary not found: set its path in the settings"
                        )
                        .to_string()
                    )
                })?;
                let restart = matches!(action, ProxyAction::Restart);
                let pinned = control::health(port).await.and_then(|h| h.pinned);
                let default_model = Self::default_model();
                let health = control::ensure_proxy(&cli, port, &default_model, restart).await?;
                // A restart should not silently drop the model the user pinned.
                if restart && let Some(pinned) = pinned {
                    let spec = format!("{}:{}", pinned.provider.as_str(), pinned.model);
                    let spec = match pinned.reasoning {
                        Some(r) => format!("{spec}@{}", r.as_str()),
                        None => spec,
                    };
                    control::pin_model(port, &spec).await?;
                }
                let text = match (is_russian(), restart) {
                    (true, true) => "Прокси перезапущен",
                    (true, false) => "Прокси запущен",
                    (false, true) => "Proxy restarted",
                    (false, false) => "Proxy started",
                };
                ErrorSignal::send(
                    ErrorSeverity::Success,
                    format!("{text} (PID {})", health.pid),
                );
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Notifiable<Poll> for ProxyActor {
    async fn notify(&mut self, _: Poll, ctx: &Context<Self>) {
        let health = self.refresh().await;
        if !self.autostart_checked {
            self.autostart_checked = true;
            // `claude` pointed at the proxy does not answer without it.
            if health.is_none() && configuration::autostart() && configuration::cli_path().is_some()
            {
                self.notify(
                    ProxyCommand {
                        action: ProxyAction::Start,
                    },
                    ctx,
                )
                .await;
            }
        }
    }
}

#[async_trait]
impl Notifiable<ProxyCommand> for ProxyActor {
    async fn notify(&mut self, msg: ProxyCommand, _: &Context<Self>) {
        self.busy = true;
        self.refresh().await;
        if let Err(e) = self.run_command(msg.action).await {
            ErrorSignal::error(&e);
        }
        self.busy = false;
        self.refresh().await;
    }
}

#[async_trait]
impl Notifiable<PinModel> for ProxyActor {
    async fn notify(&mut self, msg: PinModel, _: &Context<Self>) {
        let port = configuration::port();
        let result = match &msg.model {
            Some(model) => control::pin_model(port, model).await.map(|t| {
                let effort = t
                    .reasoning
                    .map(|r| format!(" @{}", r.as_str()))
                    .unwrap_or_default();
                format!(
                    "{} {}{effort}",
                    tr("Все сессии переключены на", "All sessions switched to"),
                    t.model
                )
            }),
            None => control::unpin_model(port).await.map(|_| {
                tr(
                    "Модель снова выбирается через /model",
                    "The model is picked with /model again",
                )
                .to_string()
            }),
        };
        match result {
            Ok(text) => ErrorSignal::send(ErrorSeverity::Success, text),
            Err(e) => ErrorSignal::error(&e),
        }
        self.refresh().await;
    }
}

#[async_trait]
impl Notifiable<ConnectClaudeCode> for ProxyActor {
    async fn notify(&mut self, msg: ConnectClaudeCode, _: &Context<Self>) {
        let result = if msg.connect {
            claude_settings::connect(configuration::port())
        } else {
            claude_settings::disconnect()
        };
        match result {
            Ok(()) if msg.connect => ErrorSignal::send(
                ErrorSeverity::Success,
                tr(
                    "Claude Code подключен к прокси. Действует на новые сессии claude",
                    "Claude Code now goes through the proxy. Applies to new claude sessions",
                ),
            ),
            Ok(()) => ErrorSignal::send(
                ErrorSeverity::Success,
                tr(
                    "Claude Code снова работает напрямую. Действует на новые сессии",
                    "Claude Code talks to Anthropic directly again. Applies to new sessions",
                ),
            ),
            Err(e) => ErrorSignal::error(&e),
        }
        self.refresh().await;
    }
}
