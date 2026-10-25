use std::collections::BTreeMap;

use async_trait::async_trait;
use claude_bridge_core::{config, families, routing, settings};
use messages::prelude::{Actor, Address, Context, Notifiable};
use rinf::RustSignal;
use tokio::task::JoinSet;

use crate::configuration::KNOWN_KEYS;
use crate::i18n::{self, tr};
use crate::signals::{ErrorSeverity, ErrorSignal, PresetInfo, SaveSettings, SettingsState};

/// The .env settings file, plus the model presets the UI offers.
pub struct SettingsActor {
    _owned_tasks: JoinSet<()>,
}

impl Actor for SettingsActor {}

/// Keys that need no proxy restart: the UI's own, and those the proxy re-reads on every request.
const UI_ONLY_KEYS: &[&str] = &["CLAUDE_BRIDGE_AUTOSTART", "CLAUDE_BRIDGE_BIN", families::DISABLED_KEY, i18n::LANG_KEY];

impl SettingsActor {
    pub fn new(self_addr: Address<Self>) -> Self {
        let mut owned_tasks = JoinSet::new();
        owned_tasks.spawn(super::forward::<SaveSettings, _>(self_addr));
        Self::send_state();
        Self { _owned_tasks: owned_tasks }
    }

    pub fn send_state() {
        let values = settings::read();
        let from_environment = KNOWN_KEYS
            .iter()
            .filter(|k| !values.contains_key(**k) && config::env(k).is_some())
            .map(|k| k.to_string())
            .collect();
        let presets = routing::PRESETS
            .iter()
            .map(|p| {
                let target = routing::resolve(p.alias, None).expect("every preset must resolve");
                PresetInfo {
                    alias: p.alias.into(),
                    provider: target.provider.as_str().into(),
                    model: target.model,
                    reasoning: target.reasoning.map(|r| r.as_str().into()),
                    description: p.description.into(),
                }
            })
            .collect();
        SettingsState {
            values,
            from_environment,
            file_path: settings::env_file().display().to_string(),
            presets,
            efforts: routing::EFFORTS.iter().map(|r| r.as_str().to_string()).collect(),
        }
        .send_signal_to_dart();
    }
}

#[async_trait]
impl Notifiable<SaveSettings> for SettingsActor {
    async fn notify(&mut self, msg: SaveSettings, _: &Context<Self>) {
        let affects_proxy = msg.values.keys().any(|k| !UI_ONLY_KEYS.contains(&k.as_str()));
        let updates: BTreeMap<String, Option<String>> =
            msg.values.into_iter().map(|(k, v)| (k, Some(v.trim().to_string()).filter(|v| !v.is_empty()))).collect();
        match settings::write(&updates) {
            Ok(()) if affects_proxy => {
                ErrorSignal::send(
                    ErrorSeverity::Info,
                    tr("Настройки сохранены. Перезапустите прокси, чтобы они применились", "Settings saved. Restart the proxy to apply them"),
                )
            }
            Ok(()) => {}
            Err(e) => ErrorSignal::error(&e),
        }
        Self::send_state();
    }
}
