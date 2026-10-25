//! Rust side of the claude-bridge UI: a control panel for the background proxy, logins
//! and settings, built on `claude-bridge-core`.

mod actors;
mod configuration;
mod i18n;
mod signals;

use actors::create_actors;
use rinf::{dart_shutdown, write_interface};
use tokio::spawn;

write_interface!();

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();
    spawn(create_actors());
    dart_shutdown().await;
}
