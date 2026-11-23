//! Kirie's GNOME Shell extension (`dev.samuelba.Kirie.Shell`): it starts Kirie as a trusted
//! client and hands it the screens straight from GNOME Shell, with no permission dialog.

use image::{RgbaImage, imageops};
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
    let folder = shell_folder();
    for (.., path) in &screens {
        // Only the extension's own files: anything else could be a squatter on its bus name
        // pointing at files to read and delete.
        if !is_shell_picture(std::path::Path::new(path), folder.as_deref()) {
            return Err(CaptureError::Shell(format!("unexpected picture {path}")));
        }
    }
    // The extension sends each monitor in strips (older versions: whole); decoding them side by
    // side keeps this quick.
    let pieces = std::thread::scope(|scope| {
        let decoding: Vec<_> = screens
            .iter()
            .map(|(index, x, y, width, height, path)| {
                scope.spawn(move || {
                    let image = image::open(path)
                        .map_err(|error| CaptureError::Shell(format!("{path}: {error}")))
                        .map(|image| image.to_rgba8());
                    let _ = std::fs::remove_file(path);
                    image.map(|image| (*index, Rect::new(*x, *y, *width, *height), image))
                })
            })
            .collect();
        decoding
            .into_iter()
            .map(|decoding| {
                decoding
                    .join()
                    .unwrap_or_else(|_| Err(CaptureError::Shell("decoding panicked".into())))
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    Ok(stitch(pieces)?
        .into_iter()
        .map(|(index, logical, image)| Frame {
            output: output(index, logical, known),
            image,
        })
        .collect())
}

/// Joins the strips of each monitor, top to bottom, into one picture of the monitor, keeping
/// the monitors in the order they first appear.
fn stitch(
    pieces: Vec<(u32, Rect, RgbaImage)>,
) -> Result<Vec<(u32, Rect, RgbaImage)>, CaptureError> {
    let mut monitors: Vec<(u32, Vec<(Rect, RgbaImage)>)> = Vec::new();
    for (index, rect, image) in pieces {
        match monitors.iter_mut().find(|(known, _)| *known == index) {
            Some((_, strips)) => strips.push((rect, image)),
            None => monitors.push((index, vec![(rect, image)])),
        }
    }
    monitors
        .into_iter()
        .map(|(index, mut strips)| {
            strips.sort_by_key(|(rect, _)| rect.y);
            let broken =
                || CaptureError::Shell(format!("the strips of monitor {index} do not fit"));
            let (first, _) = strips.first().ok_or_else(broken)?;
            let (mut logical, width) = (*first, strips[0].1.width());
            for pair in strips.windows(2) {
                let ((above, _), (below, image)) = (&pair[0], &pair[1]);
                if below.x != logical.x
                    || below.width != logical.width
                    || below.y != above.y + above.height
                    || image.width() != width
                {
                    return Err(broken());
                }
                logical.height += below.height;
            }
            if strips.len() == 1 {
                let (_, image) = strips.pop().ok_or_else(broken)?;
                return Ok((index, logical, image));
            }
            let height = strips.iter().map(|(_, image)| image.height()).sum();
            let mut whole = RgbaImage::new(width, height);
            let mut top = 0;
            for (_, image) in &strips {
                imageops::replace(&mut whole, image, 0, i64::from(top));
                top += image.height();
            }
            Ok((index, logical, whole))
        })
        .collect()
}

/// Where the extension writes its pictures.
fn shell_folder() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|dir| std::path::PathBuf::from(dir).join("kirie"))
}

/// A `shell-<monitor>.png` or `shell-<monitor>-<strip>.png` straight inside the extension's
/// folder.
fn is_shell_picture(path: &std::path::Path, folder: Option<&std::path::Path>) -> bool {
    let number = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("shell-"))
        .and_then(|name| name.strip_suffix(".png"))
        .is_some_and(|name| match name.split_once('-') {
            Some((monitor, strip)) => number(monitor) && number(strip),
            None => number(name),
        });
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
        assert!(ok("/run/user/1000/kirie/shell-1-7.png"));
        assert!(!ok("/run/user/1000/kirie/shell-1-.png"));
        assert!(!ok("/run/user/1000/kirie/shell--7.png"));
        assert!(!ok("/run/user/1000/kirie/shell-1-2-3.png"));
        assert!(!ok("/run/user/1000/kirie/../../../home/me/photo.png"));
        assert!(!ok("/home/me/shell-0.png"));
        assert!(!ok("/run/user/1000/kirie/shell-.png"));
        assert!(!ok("/run/user/1000/kirie/shell-a.png"));
        assert!(!is_shell_picture(
            std::path::Path::new("/run/user/1000/kirie/shell-0.png"),
            None
        ));
    }

    fn strip(index: u32, rect: Rect, rows: u32, shade: u8) -> (u32, Rect, RgbaImage) {
        let width = u32::try_from(rect.width * 2).unwrap();
        (
            index,
            rect,
            RgbaImage::from_pixel(width, rows, image::Rgba([shade, 0, 0, 255])),
        )
    }

    #[test]
    fn strips_join_into_monitors() {
        // Out of order, interleaved, and at twice the logical size.
        let pieces = vec![
            strip(1, Rect::new(100, 0, 50, 10), 20, 3),
            strip(0, Rect::new(0, 8, 100, 4), 8, 2),
            strip(1, Rect::new(100, 10, 50, 10), 20, 4),
            strip(0, Rect::new(0, 0, 100, 8), 16, 1),
        ];
        let monitors = stitch(pieces).unwrap();
        assert_eq!(monitors.len(), 2);
        let (index, logical, image) = &monitors[0];
        assert_eq!((*index, *logical), (1, Rect::new(100, 0, 50, 20)));
        assert_eq!(image.dimensions(), (100, 40));
        assert_eq!(image.get_pixel(0, 19)[0], 3);
        assert_eq!(image.get_pixel(0, 20)[0], 4);
        let (index, logical, image) = &monitors[1];
        assert_eq!((*index, *logical), (0, Rect::new(0, 0, 100, 12)));
        assert_eq!(image.dimensions(), (200, 24));
        assert_eq!(image.get_pixel(5, 15)[0], 1);
        assert_eq!(image.get_pixel(5, 16)[0], 2);
    }

    #[test]
    fn whole_monitors_pass_through() {
        let monitors = stitch(vec![strip(0, Rect::new(0, 0, 10, 10), 20, 9)]).unwrap();
        assert_eq!(monitors[0].1, Rect::new(0, 0, 10, 10));
        assert_eq!(monitors[0].2.dimensions(), (20, 20));
    }

    #[test]
    fn strips_that_do_not_fit_are_refused() {
        let gap = vec![
            strip(0, Rect::new(0, 0, 10, 4), 8, 0),
            strip(0, Rect::new(0, 5, 10, 4), 8, 0),
        ];
        assert!(stitch(gap).is_err());
        let shifted = vec![
            strip(0, Rect::new(0, 0, 10, 4), 8, 0),
            strip(0, Rect::new(1, 4, 10, 4), 8, 0),
        ];
        assert!(stitch(shifted).is_err());
        let mut narrow = strip(0, Rect::new(0, 4, 10, 4), 8, 0);
        narrow.2 = RgbaImage::new(19, 8);
        assert!(stitch(vec![strip(0, Rect::new(0, 0, 10, 4), 8, 0), narrow]).is_err());
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
