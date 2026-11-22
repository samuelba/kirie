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
    fn capture(&self, edit: bool, cursor: bool) -> zbus::Result<()>;
    fn screens(&self) -> zbus::Result<Vec<(u32, i32, i32, i32, i32, String)>>;
    fn set_clipboard(&self, mime: &str, data: &[u8]) -> zbus::Result<()>;
    fn windows(&self) -> zbus::Result<Vec<(String, i32, i32, i32, i32)>>;
}

/// The windows on the active workspace, topmost first, from GNOME Shell.
pub async fn windows() -> Result<Vec<crate::windows::Window>, CaptureError> {
    let failed = |error: zbus::Error| CaptureError::Shell(error.to_string());
    let connection = zbus::Connection::session().await.map_err(failed)?;
    let windows = ShellProxy::new(&connection)
        .await
        .map_err(failed)?
        .windows()
        .await
        .map_err(failed)?;
    Ok(windows
        .into_iter()
        .map(|(title, x, y, width, height)| crate::windows::Window {
            title,
            rect: Rect::new(x, y, width, height),
        })
        .collect())
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

/// Asks the extension to start a capture; false when there is no extension.
pub async fn capture(edit: bool, cursor: bool) -> bool {
    let attempt = async {
        let connection = zbus::Connection::session().await?;
        let dbus = zbus::fdo::DBusProxy::new(&connection).await?;
        if !dbus.name_has_owner(BUS_NAME.try_into()?).await? {
            return Ok::<_, zbus::Error>(false);
        }
        ShellProxy::new(&connection)
            .await?
            .capture(edit, cursor)
            .await?;
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
    let folder = shell_folder();
    for (index, x, y, width, height, path) in screens {
        // Only the extension's own files: anything else could be a squatter on its bus name
        // pointing at files to read and delete.
        if !is_shell_picture(std::path::Path::new(&path), folder.as_deref()) {
            return Err(CaptureError::Shell(format!("unexpected picture {path}")));
        }
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

/// Where the extension writes its pictures.
fn shell_folder() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| std::path::PathBuf::from(dir).join("kirie"))
}

/// A `shell-N.png` straight inside the extension's folder.
fn is_shell_picture(path: &std::path::Path, folder: Option<&std::path::Path>) -> bool {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("shell-"))
        .and_then(|name| name.strip_suffix(".png"))
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()));
    named && folder.is_some_and(|folder| path.parent() == Some(folder))
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
    fn only_the_extensions_pictures_are_read() {
        let folder = std::path::Path::new("/run/user/1000/kirie");
        let ok = |path: &str| is_shell_picture(std::path::Path::new(path), Some(folder));
        assert!(ok("/run/user/1000/kirie/shell-0.png"));
        assert!(ok("/run/user/1000/kirie/shell-12.png"));
        assert!(!ok("/run/user/1000/kirie/../../../home/me/photo.png"));
        assert!(!ok("/home/me/shell-0.png"));
        assert!(!ok("/run/user/1000/kirie/shell-.png"));
        assert!(!ok("/run/user/1000/kirie/shell-a.png"));
        assert!(!is_shell_picture(
            std::path::Path::new("/run/user/1000/kirie/shell-0.png"),
            None
        ));
    }

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
