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
            let mut notification = Notification::new();
            notification.appname("Claude Bridge").summary(&title).body(&body);
            // Cinnamon and GNOME hide normal banners over a fullscreen window and only play the
            // sound. These warnings are rare and need a reaction, so they are critical; they then
            // stay on screen until dismissed.
            #[cfg(all(unix, not(target_os = "macos")))]
            notification.urgency(notify_rust::Urgency::Critical);
            if let Err(error) = notification.show() {
                tracing::warn!("desktop notification failed: {error}");
            }
        });
    }
}
