//! Copying a picture, or marks, to the clipboard. The clipboard is served by a small background
//! copy of Kirie, so what is copied stays pasteable after the overlay closes.

use std::io::Write as _;
use std::process::{Command, Stdio};

use image::RgbaImage;

/// The hidden subcommand the background copy runs.
pub const SERVE: &str = "serve-clipboard";
pub const PNG: &str = "image/png";
/// What the background copy offers.
pub const KINDS: [&str; 2] = [PNG, crate::marks::MIME];
/// The most bytes read from the clipboard.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Encodes `image` as PNG and hands it to a background process that offers it on the clipboard.
pub async fn copy(image: RgbaImage) -> Result<(), String> {
    let png = tokio::task::spawn_blocking(move || encode_png(&image))
        .await
        .map_err(|error| error.to_string())??;
    if kirie_capture::shell::launched() {
        return kirie_capture::shell::set_clipboard(&png)
            .await
            .map_err(|error| error.to_string());
    }
    offer(png, PNG).await
}

/// Hands `bytes` to a background process that offers them on the clipboard as `kind`, one of
/// [`KINDS`].
pub async fn offer(bytes: Vec<u8>, kind: &'static str) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(exe)
            .args([SERVE, "--kind", kind])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("cannot start the clipboard service: {error}"))?;
        child
            .stdin
            .take()
            .ok_or("the clipboard service has no input")?
            .write_all(&bytes)
            .map_err(|error| format!("cannot hand it to the clipboard: {error}"))?;
        // A service that cannot take the clipboard, as where the compositor lets no program
        // do so, gives up at once; one that took it keeps running.
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    return Err("the clipboard cannot be reached here".to_owned());
                }
                Ok(Some(_)) => return Ok(()),
                Ok(None) => {}
                Err(error) => return Err(format!("cannot follow the clipboard service: {error}")),
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| format!("cannot encode the picture: {error}"))?;
    Ok(png)
}

/// What is on the clipboard as the first of `kinds` it is offered as, with that kind, read
/// through the clipboard protocol of wlroots compositors.
pub async fn paste(kinds: &'static [&'static str]) -> Result<(String, Vec<u8>), String> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read as _;
        use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents, get_mime_types};

        let offered = get_mime_types(ClipboardType::Regular, Seat::Unspecified)
            .map_err(|error| format!("there is nothing to paste ({error})"))?;
        let kind = kinds
            .iter()
            .find(|kind| offered.contains(**kind))
            .ok_or("there is no image on the clipboard")?;
        let (pipe, _) = get_contents(
            ClipboardType::Regular,
            Seat::Unspecified,
            MimeType::Specific(kind),
        )
        .map_err(|error| format!("cannot read the clipboard ({error})"))?;
        let mut bytes = Vec::new();
        pipe.take(MAX_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read the clipboard: {error}"))?;
        Ok(((*kind).to_owned(), bytes))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Offers bytes from stdin on the clipboard as `kind`, one of [`KINDS`], until something else
/// is copied.
pub fn serve(kind: &str) -> anyhow::Result<()> {
    use std::io::Read as _;
    use wl_clipboard_rs::copy::{MimeType, Options, Source};

    anyhow::ensure!(KINDS.contains(&kind), "Kirie does not copy {kind}");
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    anyhow::ensure!(!bytes.is_empty(), "nothing to copy");
    let mut options = Options::new();
    options.foreground(true);
    options.copy(
        Source::Bytes(bytes.into_boxed_slice()),
        MimeType::Specific(kind.to_owned()),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_encode_as_png() {
        let png = encode_png(&RgbaImage::new(2, 2)).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
