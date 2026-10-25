//! Desktop notifications Dart asks for (the tray has no balloon of its own on Linux).

use notify_rust::Notification;
use rinf::DartSignal;

use crate::signals::ShowNotification;

pub async fn run() {
    let receiver = ShowNotification::get_dart_signal_receiver();
    while let Some(pack) = receiver.recv().await {
        let ShowNotification { title, body } = pack.message;
        // Sending blocks on D-Bus (Linux) or the OS API, so keep it off the async workers.
        tokio::task::spawn_blocking(move || {
            if let Err(error) = Notification::new().appname("Claude Bridge").summary(&title).body(&body).show() {
                tracing::warn!("desktop notification failed: {error}");
            }
        });
    }
}
