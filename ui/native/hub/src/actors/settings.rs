use std::collections::BTreeMap;

use async_trait::async_trait;
use claude_bridge_core::{browser, config, egress, families, routing, settings};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::JoinSet;

use crate::configuration::KNOWN_KEYS;
use crate::i18n::{self, tr};
use crate::signals::{BrowserInfo, ErrorSeverity, ErrorSignal, SaveSettings, SettingsState};

/// The .env settings file and UI choices.
pub struct SettingsActor {
    _owned_tasks: JoinSet<()>,
}

impl Actor for SettingsActor {}

/// Keys that need no proxy restart: the UI's own, and those the proxy re-reads on every request.
const UI_ONLY_KEYS: &[&str] = &[
    "CLAUDE_BRIDGE_AUTOSTART",
    "CLAUDE_BRIDGE_BIN",
    browser::KEY,
    browser::PROFILE_KEY,
    browser::FINGERPRINT_KEY,
    families::DISABLED_KEY,
    i18n::LANG_KEY,
];

impl SettingsActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<SaveSettings, _>(self_addr));
        Self::send_state();
        Self {
            _owned_tasks: owned_tasks,
        }
    }

    pub fn send_state() {
        let values = settings::read();
        let from_environment = KNOWN_KEYS
            .iter()
            .filter(|k| !values.contains_key(**k) && config::env(k).is_some())
            .map(|k| k.to_string())
            .collect();
        SettingsState {
            values,
            from_environment,
            file_path: settings::env_file().display().to_string(),
            efforts: routing::EFFORTS
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
            browsers: browser::installed()
                .into_iter()
                .map(|b| BrowserInfo {
                    id: b.id,
                    name: b.name,
                })
                .collect(),
        }
        .send_signal_to_dart();
    }
}

#[async_trait]
impl Notifiable<SaveSettings> for SettingsActor {
    async fn notify(&mut self, msg: SaveSettings, _: &Context<Self>) {
        let affects_proxy = msg
            .values
            .keys()
            .any(|k| !UI_ONLY_KEYS.contains(&k.as_str()));
        let updates: BTreeMap<String, Option<String>> = msg
            .values
            .into_iter()
            .map(|(k, v)| (k, Some(v.trim().to_string()).filter(|v| !v.is_empty())))
            .collect();
        let value = |key: &str| match updates.get(key) {
            Some(new) => new.clone(),
            None => crate::configuration::setting(key),
        };
        if updates.contains_key("CLAUDE_BRIDGE_PORT") || updates.contains_key(egress::PORT_KEY) {
            let proxy_port = match value("CLAUDE_BRIDGE_PORT") {
                Some(v) => v.parse::<u16>().ok(),
                None => Some(config::DEFAULT_PORT),
            };
            let checked = match proxy_port {
                Some(port) => egress::validate_ports(port, value(egress::PORT_KEY).as_deref()),
                None => Err(anyhow::anyhow!("CLAUDE_BRIDGE_PORT: not a port number")),
            };
            if let Err(e) = checked {
                ErrorSignal::error(&e);
                Self::send_state();
                return;
            }
        }
        match settings::write(&updates) {
            Ok(()) if affects_proxy => ErrorSignal::send(
                ErrorSeverity::Info,
                tr(
                    "Настройки сохранены. Перезапустите прокси, чтобы они применились",
                    "Settings saved. Restart the proxy to apply them",
                ),
            ),
            Ok(()) => {}
            Err(e) => ErrorSignal::error(&e),
        }
        Self::send_state();
    }
}
