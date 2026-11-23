mod activation;
mod appearance;
mod clipboard;
mod controls;
mod cursor;
mod dialog;
mod drawing;
mod drops;
mod editor;
mod extension;
mod ipc;
mod launch;
mod loupe;
mod marks;
mod notify;
mod overlay;
mod paint;
mod picker;
mod region;
mod release;
mod save;
mod script;
mod selection;
mod settings;
mod shortcuts;
mod store;
mod swatches;
mod tray;
mod typing;
mod update;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use kirie_capture::Cursor;

pub const APP_ID: &str = "dev.samuelba.Kirie";
/// The longest wait before a screenshot, in seconds.
const MAX_DELAY: u64 = 60;

#[derive(Debug, Parser)]
#[command(
    name = "kirie",
    version,
    about = "Cut a piece out of your screen and mark it up"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Take a screenshot.
    Capture {
        #[arg(value_enum, default_value_t = Mode::Region)]
        mode: Mode,
        /// Write the screenshot to this file, or to stdout for -, instead of the clipboard.
        /// Needs `all`, --region or --window.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Take this region without the overlay, in desktop coordinates: `X,Y,W,H`, or `X,Y WxH`
        /// as slurp prints it.
        #[arg(long, value_name = "X,Y,W,H", value_parser = script::parse_region, conflicts_with = "window", allow_hyphen_values = true)]
        region: Option<kirie_capture::geometry::Rect>,
        /// Take the topmost window without the overlay, where Kirie can see the windows.
        #[arg(long)]
        window: bool,
        /// Draw the mouse pointer into the screenshot.
        #[arg(long)]
        cursor: bool,
        /// Open the screenshot in the editor. With the overlay, Enter opens the selection there
        /// instead of copying it.
        #[arg(long, conflicts_with = "output")]
        edit: bool,
        /// Wait this many seconds before taking the screenshot, to open a menu or hover first.
        #[arg(long, value_name = "SECONDS", default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=MAX_DELAY))]
        delay: u64,
    },
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

#[derive(Debug, Subcommand)]
enum Shortcuts {
    /// Bind Shift+Print to `kirie capture` on this desktop.
    Install,
}

#[derive(Debug, Subcommand)]
enum GnomeExtension {
    /// Install and enable it for this user.
    Install,
    /// Disable and remove it.
    Uninstall,
}

#[derive(Debug, Subcommand)]
enum Debug {
    /// The compositor's Wayland globals and which Kirie uses.
    Globals,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    /// Pick a region, a window or a screen.
    Region,
    /// Every screen together.
    All,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // stdout may carry a picture, with `capture -o -`.
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Some(Command::Capture {
            mode,
            output,
            region,
            window,
            cursor,
            edit,
            delay,
        }) => match area(mode, region, window) {
            Some(area) => {
                wait(delay);
                script::capture(area, &script::Target::of(output, edit), cursor)
            }
            None if output.is_some() => {
                anyhow::bail!("-o needs `all`, --region or --window: the overlay keeps no file")
            }
            None => capture_region(cursor, edit, delay),
        },
        Some(Command::Debug(Debug::Globals)) => print_globals(),
        Some(Command::ServeClipboard { kind }) => clipboard::serve(&kind),
        Some(Command::Tray) => tray::run(),
        Some(Command::About) => show(editor::Card::About),
        Some(Command::Settings) => show(editor::Card::Settings),
        Some(Command::Shortcuts(Shortcuts::Install)) => shortcuts::install(),
        Some(Command::SelfUpdate) => update::run(),
        Some(Command::GnomeExtension(GnomeExtension::Install)) => extension::install(),
        Some(Command::GnomeExtension(GnomeExtension::Uninstall)) => extension::uninstall(),
        None => edit(None),
        Some(Command::Edit { path }) => edit(path.as_deref()),
    }
}

/// The part of the screens a capture takes without the overlay, if it is one of those.
fn area(
    mode: Mode,
    region: Option<kirie_capture::geometry::Rect>,
    window: bool,
) -> Option<script::Area> {
    match (region, window, mode) {
        (Some(region), _, _) => Some(script::Area::Region(region)),
        (None, true, _) => Some(script::Area::Window),
        (None, false, Mode::All) => Some(script::Area::All),
        (None, false, Mode::Region) => None,
    }
}

/// Opens `path` (or an empty editor) in the running editor, or in a new one.
fn edit(path: Option<&std::path::Path>) -> anyhow::Result<()> {
    let path = path
        .map(std::path::absolute)
        .transpose()
        .context("cannot find the file")?;
    block_on(tray::ensure())?;
    let token = activation::from_env();
    if block_on(ipc::hand_over(path.as_deref(), token.as_deref()))? {
        return Ok(());
    }
    let loaded = path
        .as_deref()
        .map(editor::load)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    editor::run(loaded, None, token).map_err(anyhow::Error::from)
}

/// Shows `card` in the running editor, or in a new one.
fn show(card: editor::Card) -> anyhow::Result<()> {
    let token = activation::from_env();
    if block_on(ipc::show(card, token.as_deref()))? {
        return Ok(());
    }
    editor::run(None, Some(card), token).map_err(anyhow::Error::from)
}

/// Runs a future to completion before any UI event loop exists.
fn block_on<T>(future: impl Future<Output = T>) -> anyhow::Result<T> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future))
}

/// Takes a screenshot with the overlay, and starts the tray icon beside it if none is shown.
fn capture_region(cursor: bool, edit: bool, delay: u64) -> anyhow::Result<()> {
    let tray = std::thread::spawn(|| block_on(tray::ensure()));
    wait(delay);
    let taken = take_region(cursor, edit);
    let _ = tray.join();
    taken
}

/// Waits `delay` seconds before a screenshot.
fn wait(delay: u64) {
    if delay > 0 {
        tracing::info!(delay, "waiting before the screenshot");
        std::thread::sleep(std::time::Duration::from_secs(delay));
    }
}

fn take_region(cursor: bool, edit: bool) -> anyhow::Result<()> {
    // With Kirie's GNOME Shell extension, the extension starts a trusted Kirie for the capture.
    if !kirie_capture::shell::launched() && block_on(kirie_capture::shell::capture(edit, cursor))? {
        return Ok(());
    }
    let cursor = if cursor {
        Cursor::Shown
    } else {
        Cursor::Hidden
    };
    let lookup = std::thread::spawn(kirie_capture::windows::start);
    let (_, frames) = block_on(kirie_capture::capture(cursor))??;
    let lookup = lookup.join().unwrap_or_else(|_| {
        tracing::warn!("finding the windows failed; clicks take whole screens");
        kirie_capture::windows::Lookup::Found(Vec::new())
    });
    let windows = block_on(lookup.finish(&frames))?;
    let primary = if edit {
        overlay::Action::Edit
    } else {
        overlay::Action::Copy
    };
    let host = if kirie_capture::shell::launched() {
        overlay::Host::Shell
    } else if kirie_capture::wayland::globals().is_ok_and(|globals| globals.layer_shell)
        && std::env::var("KIRIE_OVERLAY").as_deref() != Ok("window")
    {
        overlay::Host::Layer
    } else {
        overlay::Host::Window
    };
    let outcome = overlay::run(frames, windows, primary, host);
    if edit && !matches!(outcome, Ok(overlay::Outcome::Edit(_))) {
        block_on(ipc::resume())?;
    }
    match outcome.map_err(anyhow::Error::msg)? {
        overlay::Outcome::Saved { path, copied } => saved(&path, copied),
        overlay::Outcome::SaveAs(project) => {
            let name = format!("{}.png", save::stem(&jiff::Zoned::now()));
            let folder = save::screenshots_dir();
            let _ = std::fs::create_dir_all(&folder);
            let Some((path, format)) =
                block_on(dialog::save_as(&name, &folder, &save::Format::ALL))?
                    .map_err(anyhow::Error::msg)?
            else {
                return Ok(());
            };
            let path =
                block_on(save::save_as(*project, path, format))?.map_err(anyhow::Error::msg)?;
            saved(&path, false)
        }
        overlay::Outcome::Copied => {
            block_on(notify::show(
                "Screenshot copied",
                "Paste it anywhere with Ctrl+V.",
            ))?;
            Ok(())
        }
        overlay::Outcome::Canceled => Ok(()),
        overlay::Outcome::Edit(path) => open_handoff(&path),
        overlay::Outcome::Failed(error) => anyhow::bail!(error),
    }
}

/// Opens a project from the hand-off folder in the running editor, or in a new one.
fn open_handoff(path: &std::path::Path) -> anyhow::Result<()> {
    if !block_on(ipc::hand_over(Some(path), None))? {
        editor_command(&launch::exe()?, path)
            .spawn()
            .context("cannot open the editor")?;
    }
    Ok(())
}

/// Tells where the overlay's picture was saved.
fn saved(path: &std::path::Path, copied: bool) -> anyhow::Result<()> {
    println!("{}", path.display());
    block_on(notify::saved(path, copied))?;
    Ok(())
}

/// The editor, for a picture the overlay hands over. The extension trusts only the overlay it
/// started, so the editor must not pass for it; its captures go through the extension again.
fn editor_command(exe: &std::path::Path, path: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .arg("edit")
        .arg(path)
        .env_remove(kirie_capture::shell::LAUNCHED_ENV);
    command
}

fn print_globals() -> anyhow::Result<()> {
    let globals = kirie_capture::wayland::globals()?;
    println!("ext-image-copy-capture: {}", globals.ext_image_copy_capture);
    println!("wlr-screencopy:         {}", globals.wlr_screencopy);
    println!("layer-shell:            {}", globals.layer_shell);
    println!();
    for interface in globals.interfaces {
        println!("{interface}");
    }
    Ok(())
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
    fn an_editor_the_overlay_opens_is_not_trusted_by_the_extension() {
        let command = editor_command(
            std::path::Path::new("kirie"),
            std::path::Path::new("a.kirie"),
        );
        assert!(
            command.get_envs().any(|(key, value)| {
                key == kirie_capture::shell::LAUNCHED_ENV && value.is_none()
            })
        );
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["edit", "a.kirie"]);
    }

    #[test]
    fn capture_waits_up_to_a_minute() {
        let delay = |args: &[&str]| match Cli::try_parse_from(args).map(|cli| cli.command) {
            Ok(Some(Command::Capture { delay, .. })) => Some(delay),
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
        let marks = kind(&["kirie", clipboard::SERVE, "--kind", marks::MIME]);
        assert!(clipboard::KINDS.contains(&marks.as_str()));
        assert!(clipboard::serve("text/html").is_err());
    }

    #[test]
    fn scripts_say_which_part_to_take() {
        let parsed = |args: &[&str]| match Cli::try_parse_from(args).map(|cli| cli.command) {
            Ok(Some(Command::Capture {
                mode,
                region,
                window,
                ..
            })) => Ok(area(mode, region, window)),
            Ok(other) => panic!("{other:?}"),
            Err(error) => Err(error.kind()),
        };
        assert_eq!(parsed(&["kirie", "capture"]), Ok(None));
        assert_eq!(
            parsed(&["kirie", "capture", "all", "-o", "-"]),
            Ok(Some(script::Area::All))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--region", "1,2 30x40"]),
            Ok(Some(script::Area::Region(
                kirie_capture::geometry::Rect::new(1, 2, 30, 40)
            )))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--window", "-o", "w.png"]),
            Ok(Some(script::Area::Window))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--region", "-1920,0 1920x1080"]),
            Ok(Some(script::Area::Region(
                kirie_capture::geometry::Rect::new(-1920, 0, 1920, 1080)
            ))),
            "screens left of the first one have negative coordinates"
        );
        assert!(parsed(&["kirie", "capture", "--region", "1,2,3"]).is_err());
        assert!(parsed(&["kirie", "capture", "--region", "1,2,3,4", "--window"]).is_err());
        assert_eq!(
            parsed(&["kirie", "capture", "--region", "0,0,200,100", "--edit"]),
            Ok(Some(script::Area::Region(
                kirie_capture::geometry::Rect::new(0, 0, 200, 100)
            )))
        );
        assert_eq!(
            parsed(&["kirie", "capture", "--window", "--edit"]),
            Ok(Some(script::Area::Window))
        );
        assert!(
            parsed(&[
                "kirie", "capture", "--region", "1,2,3,4", "--edit", "-o", "-"
            ])
            .is_err()
        );
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
