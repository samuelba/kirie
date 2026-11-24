//! Copying a picture, or marks, to the clipboard. The clipboard is served by a small background
//! copy of Kirie, so what is copied stays pasteable after the overlay closes. Where the
//! compositor lets no such program take it, as on GNOME, the editor's window offers it itself.

use std::io::Write as _;
use std::process::{Child, Command, Stdio};

use image::RgbaImage;

/// The hidden subcommand the background copy runs.
pub const SERVE: &str = "serve-clipboard";
pub const PNG: &str = "image/png";
/// What the background copy offers.
pub const KINDS: [&str; 2] = [PNG, kirie_annotate::marks::MIME];
/// How long the clipboard service gets to say that it holds the clipboard.
const READY: std::time::Duration = std::time::Duration::from_secs(2);
/// The most bytes read from the clipboard.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Encodes `image` as PNG and hands it to a background process that offers it on the clipboard.
pub async fn copy(image: RgbaImage) -> Result<(), String> {
    copy_from(image, None).await
}

/// Like [`copy`], with the window of `device` offering it where no background process can.
/// GNOME keeps a PNG the window offers after the window closes.
pub async fn copy_from(
    image: RgbaImage,
    device: Option<crate::desktop::drops::Device>,
) -> Result<(), String> {
    let png = tokio::task::spawn_blocking(move || encode_png(&image))
        .await
        .map_err(|error| error.to_string())??;
    if kirie_capture::shell::launched() {
        return kirie_capture::shell::set_clipboard(&png)
            .await
            .map_err(|error| error.to_string());
    }
    offer_from(png, PNG, device).await
}

/// Like [`offer`], with the window of `device` offering the bytes where no background process
/// can.
pub async fn offer_from(
    bytes: Vec<u8>,
    kind: &'static str,
    device: Option<crate::desktop::drops::Device>,
) -> Result<(), String> {
    let Some(device) = device else {
        return offer(bytes, kind).await;
    };
    let Err(error) = offer(bytes.clone(), kind).await else {
        return Ok(());
    };
    tracing::debug!(%error, "the window offers the clipboard itself");
    tokio::task::spawn_blocking(move || device.offer(bytes, kind))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|own| format!("{error}; {own}"))
}

/// Hands `bytes` to a background process that offers them on the clipboard as `kind`, one of
/// [`KINDS`]. The process gets a process group of its own, so that Ctrl+C in the terminal Kirie
/// runs in leaves the clipboard be, and is reaped once it ends.
pub async fn offer(bytes: Vec<u8>, kind: &'static str) -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;

    tokio::task::spawn_blocking(move || {
        let exe = crate::desktop::launch::exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(exe)
            .args([SERVE, "--kind", kind])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .map_err(|error| format!("cannot start the clipboard service: {error}"))?;
        let handed = hand_over(&mut child, &bytes);
        crate::desktop::launch::reap(child);
        handed
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Writes `bytes` to the clipboard service `child` and waits until it says it holds the
/// clipboard. It gives up at once where the compositor lets no program take it; one slow to
/// start is left to it.
fn hand_over(child: &mut Child, bytes: &[u8]) -> Result<(), String> {
    child
        .stdin
        .take()
        .ok_or("the clipboard service has no input")?
        .write_all(bytes)
        .map_err(|error| format!("cannot hand it to the clipboard: {error}"))?;
    let mut said = child
        .stdout
        .take()
        .ok_or("the clipboard service has no output")?;
    let (sender, ready) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read as _;
        let mut byte = [0];
        let _ = sender.send(said.read(&mut byte).is_ok_and(|read| read == 1));
    });
    match ready.recv_timeout(READY) {
        Ok(false) => Err("the clipboard cannot be reached here".to_owned()),
        Ok(true) | Err(_) => Ok(()),
    }
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
    let prepared = options.prepare_copy(
        Source::Bytes(bytes.into_boxed_slice()),
        MimeType::Specific(kind.to_owned()),
    )?;
    // Tells the Kirie that started this one that the clipboard is taken.
    let mut said = std::io::stdout();
    said.write_all(b"\n")?;
    said.flush()?;
    prepared.serve()?;
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
