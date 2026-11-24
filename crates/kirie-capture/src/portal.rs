//! The xdg-desktop-portal Screenshot call, for desktops without a capture protocol (GNOME) or
//! that keep theirs private. The portal returns one image of the whole desktop, which is cut
//! back into screens with the layout from `xdg_output`.

use std::collections::HashMap;
use std::os::unix::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::{RgbaImage, imageops};
use zbus::{OwnedVariant as OwnedValue, Variant as Value};

use crate::CaptureError;
use crate::geometry::{Frame, Output, Rect};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
/// How long a screenshot may take, with time for the user to answer GNOME's permission dialog.
const ANSWER: Duration = Duration::from_secs(120);
/// How far a file's time may lag the clock and still count as written after it.
const SLACK: Duration = Duration::from_secs(2);
/// Numbers each request, so its handle token is unique within the process.
static REQUESTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A fresh handle token for a portal request.
pub fn token() -> String {
    let request = REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("kirie_{}_{request}", std::process::id())
}

/// Calls `method` on the portal `interface` and waits for its answer. `body` must carry
/// `token` as its `handle_token` option.
pub async fn request<B>(
    interface: &str,
    method: &str,
    token: &str,
    body: &B,
) -> Result<HashMap<String, OwnedValue>, CaptureError>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let portal = |error: zbus::Error| CaptureError::Portal(error.to_string());
    let connection = crate::bus::session().await.map_err(portal)?;
    let sender = connection
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .ok_or_else(|| CaptureError::Portal("no name on the session bus".into()))?;
    let request_path = format!("{PATH}/request/{sender}/{token}");
    let request = zbus::Proxy::new(
        &connection,
        DESTINATION,
        request_path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .await
    .map_err(portal)?;
    let mut responses = request.receive_signal("Response").await.map_err(portal)?;
    zbus::Proxy::new(&connection, DESTINATION, PATH, interface)
        .await
        .map_err(portal)?
        .call_method(method, body)
        .await
        .map_err(portal)?;
    let message = futures_util::StreamExt::next(&mut responses)
        .await
        .ok_or_else(|| CaptureError::Portal("the portal did not answer".into()))?;
    let (response, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|error| CaptureError::Portal(error.to_string()))?;
    match response {
        0 => Ok(results),
        1 => Err(CaptureError::Canceled),
        _ => Err(CaptureError::Portal(format!("the portal refused {method}"))),
    }
}

/// Asks the portal for a screenshot of the whole desktop. `interactive` lets the desktop show
/// its own picker instead. Gives up after [`ANSWER`], picker or not.
pub async fn screenshot(interactive: bool) -> Result<RgbaImage, CaptureError> {
    let token = token();
    let sent = SystemTime::now();
    let options: HashMap<&str, Value<'_>> = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("interactive", Value::from(interactive)),
        ("modal", Value::from(false)),
    ]);
    let body = ("", options);
    let asked = request(
        "org.freedesktop.portal.Screenshot",
        "Screenshot",
        &token,
        &body,
    );
    let results = tokio::time::timeout(ANSWER, asked).await.map_err(|_| {
        CaptureError::Portal(format!("no answer within {} seconds", ANSWER.as_secs()))
    })??;
    let uri: String = results
        .get("uri")
        .and_then(|uri| String::try_from(uri.clone()).ok())
        .ok_or_else(|| CaptureError::Portal("the portal sent no picture".into()))?;
    let path = file_path(&uri)
        .ok_or_else(|| CaptureError::Portal(format!("cannot read the picture at {uri}")))?;
    let image = image::open(&path)
        .map_err(|error| CaptureError::Portal(error.to_string()))?
        .to_rgba8();
    // The portal saved the picture for us; Kirie keeps its own copy. Only a file made for this
    // request goes: anything else could be a squatter on the portal's bus name pointing at a
    // file to delete.
    if made_since(&path, sent) {
        let _ = std::fs::remove_file(&path);
    } else {
        tracing::warn!(path = %path.display(), "keeping a picture the portal did not make now");
    }
    Ok(image)
}

/// Whether `path` is a plain file, not a link, last written no earlier than `since`. A little
/// slack allows for coarse file times.
fn made_since(path: &Path, since: SystemTime) -> bool {
    let since = since.checked_sub(SLACK).unwrap_or(SystemTime::UNIX_EPOCH);
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.modified().is_ok_and(|modified| modified >= since)
    })
}

/// The local path of a `file://` URI, with percent escapes decoded.
pub fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
}

/// Cuts a picture of the whole desktop back into one frame per screen. The picture's scale is
/// taken from how much wider it is than the desktop.
pub fn slice(image: &RgbaImage, outputs: &[Output]) -> Vec<Frame> {
    let Some(desktop) = outputs
        .iter()
        .map(|output| output.logical)
        .reduce(|all, rect| all.union(&rect))
    else {
        return Vec::new();
    };
    if desktop.width <= 0 {
        return Vec::new();
    }
    let scale = f64::from(image.width()) / f64::from(desktop.width);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let px = |value: i32| (f64::from(value) * scale).round().max(0.0) as u32;
    outputs
        .iter()
        .filter_map(|output| {
            let rect = Rect {
                x: output.logical.x - desktop.x,
                y: output.logical.y - desktop.y,
                ..output.logical
            };
            let (x, y) = (px(rect.x), px(rect.y));
            let width = px(rect.width).min(image.width().saturating_sub(x));
            let height = px(rect.height).min(image.height().saturating_sub(y));
            (width > 0 && height > 0).then(|| Frame {
                output: output.clone(),
                image: imageops::crop_imm(image, x, y, width, height).to_image(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn output(name: &str, logical: Rect) -> Output {
        Output {
            name: name.into(),
            logical,
            ..Output::default()
        }
    }

    #[test]
    fn uris_become_paths() {
        assert_eq!(
            file_path("file:///home/me/Pictures/Screenshot%20from%202026.png"),
            Some(PathBuf::from("/home/me/Pictures/Screenshot from 2026.png"))
        );
        assert_eq!(file_path("https://example.org/a.png"), None);
        assert_eq!(file_path("file:///bad%zz"), None);
        assert_eq!(file_path("file:///cut%2"), None);
    }

    #[test]
    fn only_pictures_made_for_the_request_are_deleted() {
        let dir = std::env::temp_dir().join(format!("kirie-made-since-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = dir.join("fresh.png");
        let old = dir.join("old.png");
        let link = dir.join("link.png");
        let sent = SystemTime::now();
        std::fs::write(&fresh, b"new").unwrap();
        std::fs::write(&old, b"mine").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(sent - Duration::from_secs(60))
            .unwrap();
        std::os::unix::fs::symlink(&fresh, &link).unwrap();

        assert!(made_since(&fresh, sent));
        assert!(!made_since(&old, sent));
        assert!(!made_since(&link, sent));
        assert!(!made_since(&dir, sent));
        assert!(!made_since(&dir.join("missing.png"), sent));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn slices_follow_the_layout_and_scale() {
        let mut desktop = RgbaImage::from_pixel(400, 100, Rgba([0, 0, 255, 255]));
        for x in 200..400 {
            for y in 0..100 {
                desktop.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let outputs = [
            output("left", Rect::new(-100, 0, 100, 50)),
            output("right", Rect::new(0, 0, 100, 50)),
        ];
        let frames = slice(&desktop, &outputs);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].image.dimensions(), (200, 100));
        assert_eq!(frames[0].image.get_pixel(0, 0), &Rgba([0, 0, 255, 255]));
        assert_eq!(frames[1].image.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
        assert!((frames[1].scale() - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nothing_to_slice() {
        let image = RgbaImage::new(10, 10);
        assert!(slice(&image, &[]).is_empty());
        assert!(slice(&image, &[output("zero", Rect::new(0, 0, 0, 0))]).is_empty());
        assert!(
            slice(
                &image,
                &[
                    output("off", Rect::new(0, 0, 10, 10)),
                    output("far", Rect::new(500, 0, 10, 10))
                ]
            )
            .iter()
            .all(|frame| frame.image.width() > 0)
        );
    }
}
