//! Kirie's GNOME Shell extension (`dev.samuelba.Kirie.Shell`): it starts Kirie as a trusted
//! client and hands it the screens straight from GNOME Shell, with no permission dialog.

use zbus::proxy;

use crate::CaptureError;
use crate::geometry::{Frame, Output, Rect};

pub const BUS_NAME: &str = "dev.samuelba.Kirie.Shell";
/// Set by the extension in the environment of the Kirie it starts.
pub const LAUNCHED_ENV: &str = "KIRIE_SHELL_LAUNCHED";

#[proxy(
    interface = "dev.samuelba.Kirie.Shell",
    default_service = "dev.samuelba.Kirie.Shell",
    default_path = "/dev/samuelba/Kirie/Shell"
)]
trait Shell {
    fn capture(&self, args: &[&str]) -> zbus::Result<()>;
    fn screens(&self) -> zbus::Result<Vec<(u32, i32, i32, i32, i32, String)>>;
    fn set_clipboard(&self, mime: &str, data: &[u8]) -> zbus::Result<()>;
}

/// Puts a PNG on the clipboard through GNOME Shell, which keeps it after Kirie exits.
pub async fn set_clipboard(png: &[u8]) -> Result<(), CaptureError> {
    let failed = |error: zbus::Error| CaptureError::Shell(error.to_string());
    let connection = zbus::Connection::session().await.map_err(failed)?;
    ShellProxy::new(&connection)
        .await
        .map_err(failed)?
        .set_clipboard("image/png", png)
        .await
        .map_err(failed)
}

/// Whether this Kirie was started by the extension.
pub fn launched() -> bool {
    std::env::var_os(LAUNCHED_ENV).is_some()
}

/// Asks the extension to start a capture with `args`; false when there is no extension.
pub async fn capture(args: &[&str]) -> bool {
    let attempt = async {
        let connection = zbus::Connection::session().await?;
        let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
        if !dbus.name_has_owner(BUS_NAME.try_into()?).await? {
            return Ok::<_, zbus::Error>(false);
        }
        ShellProxy::new(&connection).await?.capture(args).await?;
        Ok(true)
    };
    attempt
        .await
        .inspect_err(|error| tracing::debug!(%error, "the GNOME Shell extension is not there"))
        .unwrap_or(false)
}

/// Every screen, captured by GNOME Shell, in the order of its monitor index.
pub async fn screens(known: &[Output]) -> Result<Vec<Frame>, CaptureError> {
    let failed = |error: zbus::Error| CaptureError::Shell(error.to_string());
    let connection = zbus::Connection::session().await.map_err(failed)?;
    let screens = ShellProxy::new(&connection)
        .await
        .map_err(failed)?
        .screens()
        .await
        .map_err(failed)?;
    let mut frames = Vec::with_capacity(screens.len());
    for (index, x, y, width, height, path) in screens {
        let image = image::open(&path)
            .map_err(|error| CaptureError::Shell(format!("{path}: {error}")))?
            .to_rgba8();
        let _ = std::fs::remove_file(&path);
        frames.push(Frame {
            output: output(index, Rect::new(x, y, width, height), known),
            image,
        });
    }
    Ok(frames)
}

/// The Wayland output at `logical`, or a stand-in named after the monitor index.
fn output(index: u32, logical: Rect, known: &[Output]) -> Output {
    known
        .iter()
        .find(|output| output.logical == logical)
        .cloned()
        .unwrap_or_else(|| Output {
            name: format!("monitor-{index}"),
            description: String::new(),
            logical,
            transform: crate::Transform::Normal,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitors_match_outputs_by_layout() {
        let known = [Output {
            name: "DP-1".into(),
            logical: Rect::new(1920, 0, 2560, 1440),
            ..Output::default()
        }];
        assert_eq!(
            output(1, Rect::new(1920, 0, 2560, 1440), &known).name,
            "DP-1"
        );
        let stand_in = output(0, Rect::new(0, 0, 1920, 1080), &known);
        assert_eq!(stand_in.name, "monitor-0");
        assert_eq!(stand_in.logical, Rect::new(0, 0, 1920, 1080));
    }
}
