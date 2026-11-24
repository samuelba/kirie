//! The desktop's light or dark preference, from the Settings portal, as it changes.

use iced::futures::{SinkExt as _, Stream, StreamExt as _};
use kirie_ui::Scheme;
use zbus::OwnedVariant;

const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The scheme for a portal `color-scheme` value: 1 prefers dark, 2 light, anything else neither.
pub fn scheme(value: u32) -> Option<Scheme> {
    match value {
        1 => Some(Scheme::Dark),
        2 => Some(Scheme::Light),
        _ => None,
    }
}

fn number(value: &OwnedVariant) -> Option<u32> {
    u32::try_from(value.clone()).ok()
}

/// The preference now and every change after, while the stream is polled.
pub fn changes() -> impl Stream<Item = Scheme> {
    iced::stream::channel(4, async |mut output| {
        let watched = async {
            let connection = zbus::Connection::session().await?;
            let settings = zbus::Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Settings",
            )
            .await?;
            let mut changes = settings.receive_signal("SettingChanged").await?;
            let now: OwnedVariant = settings.call("ReadOne", &(NAMESPACE, KEY)).await?;
            if let Some(scheme) = number(&now).and_then(scheme) {
                let _ = output.send(scheme).await;
            }
            while let Some(message) = changes.next().await {
                let Ok((namespace, key, value)) = message
                    .body()
                    .deserialize::<(String, String, OwnedVariant)>()
                else {
                    continue;
                };
                if namespace == NAMESPACE
                    && key == KEY
                    && let Some(scheme) = number(&value).and_then(scheme)
                    && output.send(scheme).await.is_err()
                {
                    break;
                }
            }
            Ok::<_, zbus::Error>(())
        };
        if let Err(error) = watched.await {
            tracing::debug!(%error, "no color scheme from the Settings portal");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_values_map_to_schemes() {
        assert_eq!(scheme(1), Some(Scheme::Dark));
        assert_eq!(scheme(2), Some(Scheme::Light));
        assert_eq!(scheme(0), None);
        assert_eq!(scheme(7), None);
    }

    #[test]
    fn only_numbers_are_read() {
        let plain = OwnedVariant::from(2u32);
        assert_eq!(number(&plain), Some(2));
        let text = OwnedVariant::try_from(zbus::Variant::from("dark")).unwrap();
        assert_eq!(number(&text), None);
    }
}
