//! Screenshots without the overlay, for scripts: every screen, a region given on the command
//! line, or the topmost window, written to a file, to stdout or to the clipboard, or opened in
//! the editor.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::Context as _;
use kirie_capture::{Cursor, geometry};

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

/// A region as `X,Y,W,H`, or as slurp prints it, `X,Y WxH`.
pub fn parse_region(text: &str) -> Result<geometry::Rect, String> {
    let numbers: Vec<&str> = text
        .split([',', ' ', 'x'])
        .filter(|part| !part.is_empty())
        .collect();
    let [x, y, width, height] = numbers[..] else {
        return Err(format!("{text:?} is not X,Y,W,H"));
    };
    let number = |part: &str| {
        part.trim()
            .parse::<i32>()
            .map_err(|_| format!("{part:?} is not a whole number"))
    };
    let rect = geometry::Rect::new(number(x)?, number(y)?, number(width)?, number(height)?);
    if rect.width <= 0 || rect.height <= 0 {
        return Err("the region must be at least one pixel wide and tall".to_owned());
    }
    if rect.x.checked_add(rect.width).is_none() || rect.y.checked_add(rect.height).is_none() {
        return Err("the region reaches too far".to_owned());
    }
    Ok(rect)
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
    let (protocol, frames) = crate::block_on(kirie_capture::capture(cursor))??;
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
            let windows = crate::block_on(lookup.finish(&frames))?;
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
            crate::open_handoff(&path)?;
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
            crate::block_on(crate::desktop::clipboard::copy(image))?.map_err(anyhow::Error::msg)?;
            eprintln!("Copied {width}×{height} to the clipboard");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_are_read_as_kirie_or_slurp_writes_them() {
        assert_eq!(
            parse_region("10,20,300,200"),
            Ok(geometry::Rect::new(10, 20, 300, 200))
        );
        assert_eq!(
            parse_region("-5,20 300x200"),
            Ok(geometry::Rect::new(-5, 20, 300, 200))
        );
        assert!(parse_region("10,20,300").is_err());
        assert!(parse_region("10,20,0,5").is_err());
        assert!(parse_region("a,b,c,d").is_err());
        assert!(parse_region("1,2,3,4,5").is_err());
        assert!(parse_region("2147483647,0,5,5").is_err());
    }

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
