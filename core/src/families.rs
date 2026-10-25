//! Provider groups switched off in the UI (`CLAUDE_BRIDGE_DISABLED_PROVIDERS`, comma-separated
//! `Provider::family` names). Read on every request, so a change applies without a restart.

use crate::routing::Provider;
use crate::{config, settings};

pub const DISABLED_KEY: &str = "CLAUDE_BRIDGE_DISABLED_PROVIDERS";

pub fn disabled() -> Vec<String> {
    let value = settings::read().remove(DISABLED_KEY).or_else(|| config::env(DISABLED_KEY)).unwrap_or_default();
    value.split(',').map(|f| f.trim().to_ascii_lowercase()).filter(|f| !f.is_empty()).collect()
}

pub fn is_disabled(family: &str) -> bool {
    disabled().iter().any(|f| f == family)
}

pub fn provider_disabled(provider: Provider) -> bool {
    is_disabled(provider.family())
}
