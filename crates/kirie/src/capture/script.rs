//! Screenshots without the overlay, for scripts: every screen, a region given on the command
//! line, or the topmost window, written to a file, to stdout or to the clipboard, or opened in
//! the editor.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::Context as _;
use kirie_capture::{Cursor, geometry};

use crate::runtime::block_on;

/// Which part of the screens to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    All,
    /// A region in desktop coordinates, as the compositor lays the screens out.
    Region(geometry::Rect),
    /// The topmost window.
    Window,
}

/// Where the picture goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File(PathBuf),
    Stdout,
    Clipboard,
    Editor,
}

impl Target {
    /// The editor when `edit`, else `-` for stdout, a path for a file, nothing for the clipboard.
    pub fn of(output: Option<PathBuf>, edit: bool) -> Self {
        match output {
            _ if edit => Self::Editor,
            Some(path) if path.as_os_str() == "-" => Self::Stdout,
            Some(path) => Self::File(path),
            None => Self::Clipboard,
        }
    }
}

/// Takes `area` of the screens and sends it to `target`.
pub fn capture(area: Area, target: &Target, cursor: bool) -> anyhow::Result<()> {
    // A Kirie the GNOME Shell extension started may read the screens without asking; it only
    // ever shows them in the overlay, never writes them where its arguments say.
    anyhow::ensure!(
        !kirie_capture::shell::launched(),
        "capturing without the overlay is not available from the GNOME Shell extension"
    );
    let cursor = if cursor {
        Cursor::Shown
    } else {
        Cursor::Hidden
    };
    let lookup = (area == Area::Window).then(|| std::thread::spawn(kirie_capture::windows::start));
    let (protocol, frames) = block_on(kirie_capture::capture(cursor))??;
    let bounds = geometry::bounds(&frames).context("no screens")?;
    let part = match area {
        Area::All => bounds,
        Area::Region(region) => region
            .intersection(&bounds)
            .context("the region is beside every screen")?,
        Area::Window => {
            let lookup = lookup
                .and_then(|lookup| lookup.join().ok())
                .context("cannot find the windows")?;
            let windows = block_on(lookup.finish(&frames))?;
            let window = windows
                .first()
                .context("Kirie cannot see the windows on this desktop")?;
            tracing::info!(title = window.title, "taking the topmost window");
            window
                .rect
                .intersection(&bounds)
                .context("the window is beside every screen")?
        }
    };
    let image = geometry::crop(&frames, part).context("nothing captured")?;
    let (width, height) = image.dimensions();
    match target {
        Target::Editor => {
            let unit = f64::from(width) / f64::from(part.width.max(1));
            let document = crate::annotate::drawing::Drawing::load()
                .styled(kirie_annotate::Document::new(width, height, unit));
            let project = kirie_annotate::project::Project { image, document };
            let path = crate::storage::save::save_handoff(&project).map_err(anyhow::Error::msg)?;
            super::handoff::open(&path)?;
        }
        Target::File(path) => {
            image
                .save(path)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!(
                "Saved {width}×{height} via {protocol:?} to {}",
                path.display()
            );
        }
        Target::Stdout => {
            let png = crate::desktop::clipboard::encode_png(&image).map_err(anyhow::Error::msg)?;
            let mut out = std::io::stdout().lock();
            out.write_all(&png)
                .and_then(|()| out.flush())
                .context("cannot write to stdout")?;
        }
        Target::Clipboard => {
            block_on(crate::desktop::clipboard::copy(image))?.map_err(anyhow::Error::msg)?;
            eprintln!("Copied {width}×{height} to the clipboard");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dash_is_stdout_and_nothing_is_the_clipboard() {
        assert_eq!(Target::of(Some("-".into()), false), Target::Stdout);
        assert_eq!(
            Target::of(Some("a.png".into()), false),
            Target::File("a.png".into())
        );
        assert_eq!(Target::of(None, false), Target::Clipboard);
        assert_eq!(Target::of(None, true), Target::Editor);
    }
}
