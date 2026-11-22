//! Copying a picture to the clipboard. The clipboard is served by a small background copy of
//! Kirie, so the picture stays pasteable after the overlay closes.

use std::io::Write as _;
use std::process::{Command, Stdio};

use image::RgbaImage;

/// The hidden subcommand the background copy runs.
pub const SERVE: &str = "serve-clipboard";

/// Encodes `image` as PNG and hands it to a background process that offers it on the clipboard.
pub async fn copy(image: RgbaImage) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let png = encode_png(&image)?;
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(exe)
            .arg(SERVE)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("cannot start the clipboard service: {error}"))?;
        child
            .stdin
            .take()
            .ok_or("the clipboard service has no input")?
            .write_all(&png)
            .map_err(|error| format!("cannot hand the picture to the clipboard: {error}"))?;
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

/// Offers PNG bytes from stdin on the clipboard until something else is copied.
pub fn serve() -> anyhow::Result<()> {
    use std::io::Read as _;
    use wl_clipboard_rs::copy::{MimeType, Options, Source};

    let mut png = Vec::new();
    std::io::stdin().read_to_end(&mut png)?;
    anyhow::ensure!(!png.is_empty(), "nothing to copy");
    let mut options = Options::new();
    options.foreground(true);
    options.copy(
        Source::Bytes(png.into_boxed_slice()),
        MimeType::Specific("image/png".to_owned()),
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
