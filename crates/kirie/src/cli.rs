use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use kirie_capture::geometry::Rect;

use crate::desktop::clipboard;

/// The longest wait before a screenshot, in seconds.
const MAX_DELAY: u64 = 60;

#[derive(Debug, Parser)]
#[command(
    name = "kirie",
    version,
    about = "Cut a piece out of your screen and mark it up"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Take a screenshot.
    Capture(CaptureArgs),
    /// Open the editor, with an image if one is given.
    Edit { path: Option<PathBuf> },
    /// Install the latest release, after checking its signature.
    #[command(name = "self-update")]
    SelfUpdate,
    /// Set up a keyboard shortcut for screenshots.
    #[command(subcommand)]
    Shortcuts(Shortcuts),
    /// Install or remove the companion GNOME Shell extension.
    #[command(subcommand, name = "gnome-extension")]
    GnomeExtension(GnomeExtension),
    /// Information for bug reports.
    #[command(subcommand, hide = true)]
    Debug(Debug),
    /// Offers a PNG, or marks, from stdin on the clipboard until something else is copied.
    #[command(name = "serve-clipboard", hide = true)]
    ServeClipboard {
        #[arg(long, default_value = clipboard::PNG)]
        kind: String,
    },
    /// Shows Kirie's tray icon until Quit is picked from its menu.
    #[command(hide = true)]
    Tray,
    /// Opens the editor with the about card.
    #[command(hide = true)]
    About,
    /// Opens the editor with the settings.
    #[command(hide = true)]
    Settings,
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    #[arg(value_enum, default_value_t = Mode::Region)]
    pub mode: Mode,
    /// Write the screenshot to this file, or to stdout for -, instead of the clipboard.
    /// Needs `all`, --region or --window.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Take this region without the overlay, in desktop coordinates: `X,Y,W,H`, or `X,Y WxH`
    /// as slurp prints it.
    #[arg(long, value_name = "X,Y,W,H", value_parser = parse_region, conflicts_with = "window", allow_hyphen_values = true)]
    pub region: Option<Rect>,
    /// Take the topmost window without the overlay, where Kirie can see the windows.
    #[arg(long)]
    pub window: bool,
    /// Draw the mouse pointer into the screenshot.
    #[arg(long)]
    pub cursor: bool,
    /// Open the screenshot in the editor. With the overlay, Enter opens the selection there
    /// instead of copying it.
    #[arg(long, conflicts_with = "output")]
    pub edit: bool,
    /// Wait this many seconds before taking the screenshot, to open a menu or hover first.
    #[arg(long, value_name = "SECONDS", default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=MAX_DELAY))]
    pub delay: u64,
}

/// A region as `X,Y,W,H`, or as slurp prints it, `X,Y WxH`.
fn parse_region(text: &str) -> Result<Rect, String> {
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
    let rect = Rect::new(number(x)?, number(y)?, number(width)?, number(height)?);
    if rect.width <= 0 || rect.height <= 0 {
        return Err("the region must be at least one pixel wide and tall".to_owned());
    }
    if rect.x.checked_add(rect.width).is_none() || rect.y.checked_add(rect.height).is_none() {
        return Err("the region reaches too far".to_owned());
    }
    Ok(rect)
}

#[derive(Debug, Subcommand)]
pub enum Shortcuts {
    /// Bind Shift+Print to `kirie capture` on this desktop.
    Install,
}

#[derive(Debug, Subcommand)]
pub enum GnomeExtension {
    /// Install and enable it for this user.
    Install,
    /// Disable and remove it.
    Uninstall,
}

#[derive(Debug, Subcommand)]
pub enum Debug {
    /// The compositor's Wayland globals and which Kirie uses.
    Globals,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Mode {
    /// Pick a region, a window or a screen.
    Region,
    /// Every screen together.
    All,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command line the desktop entry starts, as a launcher runs it with no files.
    fn desktop_commands() -> Vec<Vec<&'static str>> {
        include_str!("../../../packaging/dev.samuelba.Kirie.desktop")
            .lines()
            .filter_map(|line| line.strip_prefix("Exec="))
            .map(|exec| {
                exec.split_whitespace()
                    .filter(|word| !word.starts_with('%'))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_desktop_entry_starts_kirie() {
        let commands = desktop_commands();
        assert!(!commands.is_empty());
        for command in commands {
            assert!(
                Cli::try_parse_from(&command).is_ok(),
                "the desktop entry runs {command:?}"
            );
        }
    }

    #[test]
    fn the_desktop_entry_opens_the_project_file_type() {
        let mime = include_str!("../../../packaging/dev.samuelba.Kirie.mime.xml");
        let desktop = include_str!("../../../packaging/dev.samuelba.Kirie.desktop");
        let kind = mime
            .split("<mime-type type=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        let opened = desktop
            .lines()
            .find_map(|line| line.strip_prefix("MimeType="))
            .unwrap();
        assert!(opened.split(';').any(|listed| listed == kind));
        let glob = format!(
            "<glob pattern=\"*.{}\"/>",
            kirie_annotate::project::EXTENSION
        );
        assert!(mime.contains(&glob));
    }

    #[test]
    fn capture_waits_up_to_a_minute() {
        let delay = |args: &[&str]| match Cli::try_parse_from(args).map(|cli| cli.command) {
            Ok(Some(Command::Capture(capture))) => Some(capture.delay),
            _ => None,
        };
        assert_eq!(delay(&["kirie", "capture"]), Some(0));
        assert_eq!(delay(&["kirie", "capture", "--delay", "5"]), Some(5));
        assert_eq!(
            delay(&["kirie", "capture", "all", "--delay", "60"]),
            Some(60)
        );
        assert_eq!(delay(&["kirie", "capture", "--delay", "61"]), None);
        assert_eq!(delay(&["kirie", "capture", "--delay", "-1"]), None);
    }

    #[test]
    fn the_clipboard_service_offers_pictures_or_marks() {
        let kind = |args: &[&str]| match Cli::parse_from(args).command {
            Some(Command::ServeClipboard { kind }) => kind,
            other => panic!("{other:?}"),
        };
        assert_eq!(kind(&["kirie", clipboard::SERVE]), clipboard::PNG);
        let marks = kind(&[
            "kirie",
            clipboard::SERVE,
            "--kind",
            kirie_annotate::marks::MIME,
        ]);
        assert!(clipboard::KINDS.contains(&marks.as_str()));
        assert!(clipboard::serve("text/html").is_err());
    }

    #[test]
    fn regions_are_read_as_kirie_or_slurp_writes_them() {
        assert_eq!(
            parse_region("10,20,300,200"),
            Ok(Rect::new(10, 20, 300, 200))
        );
        assert_eq!(
            parse_region("-5,20 300x200"),
            Ok(Rect::new(-5, 20, 300, 200))
        );
        assert!(parse_region("10,20,300").is_err());
        assert!(parse_region("10,20,0,5").is_err());
        assert!(parse_region("a,b,c,d").is_err());
        assert!(parse_region("1,2,3,4,5").is_err());
        assert!(parse_region("2147483647,0,5,5").is_err());
    }

    #[test]
    fn edit_takes_an_optional_file() {
        assert!(matches!(
            Cli::parse_from(["kirie", "edit"]).command,
            Some(Command::Edit { path: None })
        ));
        assert!(matches!(
            Cli::parse_from(["kirie", "edit", "a.png"]).command,
            Some(Command::Edit { path: Some(_) })
        ));
    }
}
