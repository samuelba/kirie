//! A desktop notification once the overlay has copied or saved a picture.

use std::collections::HashMap;

use zbus::Variant;

const ICON: &str = crate::APP_ID;
/// How long the notification stays, in milliseconds.
const TIMEOUT: i32 = 4000;

/// Shows `summary` and `body`; failures are logged, never shown.
pub async fn show(summary: &str, body: &str) {
    let sent = async {
        let connection = zbus::Connection::session().await?;
        let hints: HashMap<&str, Variant<'_>> = HashMap::from([
            ("desktop-entry", Variant::from(crate::APP_ID)),
            ("transient", Variant::from(true)),
        ]);
        connection
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    "Kirie",
                    0u32,
                    ICON,
                    summary,
                    body,
                    Vec::<&str>::new(),
                    hints,
                    TIMEOUT,
                ),
            )
            .await?;
        Ok::<_, zbus::Error>(())
    };
    if let Err(error) = sent.await {
        tracing::debug!(%error, "cannot show a notification");
    }
}
