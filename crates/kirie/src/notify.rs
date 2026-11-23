//! A desktop notification once the overlay has copied or saved a picture.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use iced::futures::StreamExt as _;
use zbus::Variant;

const ICON: &str = crate::APP_ID;
/// How long the notification stays, in milliseconds.
const TIMEOUT: i32 = 4000;
/// How long a notification with buttons is waited on at most, should the desktop never say that
/// it closed.
const WAIT: Duration = Duration::from_secs(60);

/// Clicking the notification itself.
const DEFAULT: &str = "default";
const OPEN: &str = "open";
const SHOW: &str = "show";

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Variant<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

fn hints() -> HashMap<&'static str, Variant<'static>> {
    HashMap::from([
        ("desktop-entry", Variant::from(crate::APP_ID)),
        ("transient", Variant::from(true)),
    ])
}

/// Shows `summary` and `body`; failures are logged, never shown.
pub async fn show(summary: &str, body: &str) {
    let sent = async {
        let connection = zbus::Connection::session().await?;
        NotificationsProxy::new(&connection)
            .await?
            .notify("Kirie", 0, ICON, summary, body, &[], hints(), TIMEOUT)
            .await?;
        Ok::<_, zbus::Error>(())
    };
    if let Err(error) = sent.await {
        tracing::debug!(%error, "cannot show a notification");
    }
}

/// Tells that the picture was saved to `path`, with buttons that open it or show it in the file
/// manager, and waits until one is pressed or the notification goes away. Failures are logged,
/// never shown.
pub async fn saved(path: &Path) {
    let summary = "Screenshot saved";
    let body = crate::save::pretty(path);
    let shown = async {
        let connection = zbus::Connection::session().await?;
        let proxy = NotificationsProxy::new(&connection).await?;
        let buttons = proxy
            .get_capabilities()
            .await
            .is_ok_and(|capabilities| capabilities.iter().any(|one| one == "actions"));
        if !buttons {
            proxy
                .notify("Kirie", 0, ICON, summary, &body, &[], hints(), TIMEOUT)
                .await?;
            return Ok(None);
        }
        // Listening before the notification shows, so no answer is missed.
        let mut invoked = proxy.receive_action_invoked().await?;
        let mut closed = proxy.receive_notification_closed().await?;
        let actions = [DEFAULT, "Open", OPEN, "Open", SHOW, "Show in Files"];
        let id = proxy
            .notify("Kirie", 0, ICON, summary, &body, &actions, hints(), TIMEOUT)
            .await?;
        let answer = async {
            loop {
                tokio::select! {
                    Some(signal) = invoked.next() => {
                        let Ok(args) = signal.args() else { continue };
                        if args.id == id {
                            return Some(args.action_key);
                        }
                    }
                    Some(signal) = closed.next() => {
                        if signal.args().is_ok_and(|args| args.id == id) {
                            return None;
                        }
                    }
                    else => return None,
                }
            }
        };
        Ok::<_, zbus::Error>(tokio::time::timeout(WAIT, answer).await.ok().flatten())
    };
    match shown.await {
        Ok(Some(action)) if action == SHOW => crate::launch::show_in_files(path).await,
        Ok(Some(action)) if action == OPEN || action == DEFAULT => crate::launch::open(path),
        Ok(_) => {}
        Err(error) => tracing::debug!(%error, "cannot show a notification"),
    }
}
