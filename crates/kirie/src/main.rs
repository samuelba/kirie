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
mod notify;
mod overlay;
mod paint;
mod picker;
mod region;
mod save;
mod selection;
mod shortcuts;
mod store;
mod swatches;
mod typing;
mod update;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use kirie_capture::{Cursor, geometry};

pub const APP_ID: &str = "dev.samuelba.Kirie";

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
        /// Write the screenshot to this file instead of showing the overlay.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Draw the mouse pointer into the screenshot.
        #[arg(long)]
        cursor: bool,
        /// Enter opens the selection in the editor instead of copying it.
        #[arg(long)]
        edit: bool,
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
    /// Offers a PNG from stdin on the clipboard until something else is copied.
    #[command(name = "serve-clipboard", hide = true)]
    ServeClipboard,
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
        .init();
    match Cli::parse().command {
        Some(Command::Capture {
            mode: Mode::All,
            output: Some(path),
            cursor,
            ..
        }) => capture_to_file(&path, cursor),
        Some(Command::Capture {
            mode: Mode::Region,
            output: None,
            cursor,
            edit,
        }) => capture_region(cursor, edit),
        Some(Command::Debug(Debug::Globals)) => print_globals(),
        Some(Command::ServeClipboard) => clipboard::serve(),
        Some(Command::Shortcuts(Shortcuts::Install)) => shortcuts::install(),
        Some(Command::SelfUpdate) => update::run(),
        Some(Command::GnomeExtension(GnomeExtension::Install)) => extension::install(),
        Some(Command::GnomeExtension(GnomeExtension::Uninstall)) => extension::uninstall(),
        None => edit(None),
        Some(Command::Edit { path }) => edit(path.as_deref()),
        Some(Command::Capture { .. }) => {
            anyhow::bail!("this capture mode is not implemented yet")
        }
    }
}

fn capture_to_file(path: &std::path::Path, cursor: bool) -> anyhow::Result<()> {
    // A Kirie the GNOME Shell extension started may read the screens without asking; it only
    // ever shows them in the overlay, never writes them where its arguments say.
    anyhow::ensure!(
        !kirie_capture::shell::launched(),
        "capturing to a file is not available from the GNOME Shell extension"
    );
    let cursor = if cursor {
        Cursor::Shown
    } else {
        Cursor::Hidden
    };
    let (protocol, frames) = block_on(kirie_capture::capture(cursor))??;
    for frame in &frames {
        tracing::info!(
            output = frame.output.name,
            logical = ?frame.output.logical,
            pixels = ?frame.image.dimensions(),
            scale = frame.scale(),
            "captured"
        );
    }
    let area = geometry::bounds(&frames).context("no screens")?;
    let image = geometry::crop(&frames, area).context("nothing captured")?;
    image
        .save(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    eprintln!(
        "Saved {}×{} from {} screen(s) via {protocol:?} to {}",
        image.width(),
        image.height(),
        frames.len(),
        path.display()
    );
    Ok(())
}

/// Opens `path` (or an empty editor) in the running editor, or in a new one.
fn edit(path: Option<&std::path::Path>) -> anyhow::Result<()> {
    let path = path
        .map(std::path::absolute)
        .transpose()
        .context("cannot find the file")?;
    if block_on(ipc::hand_over(path.as_deref()))? {
        return Ok(());
    }
    let loaded = path
        .as_deref()
        .map(editor::load)
        .transpose()
        .map_err(anyhow::Error::msg)?;
    editor::run(loaded).map_err(anyhow::Error::from)
}

/// Runs a future to completion before any UI event loop exists.
fn block_on<T>(future: impl Future<Output = T>) -> anyhow::Result<T> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future))
}

fn capture_region(cursor: bool, edit: bool) -> anyhow::Result<()> {
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
        overlay::Outcome::Saved(path) => {
            println!("{}", path.display());
            block_on(notify::show("Screenshot saved", &save::pretty(&path)))?;
            Ok(())
        }
        overlay::Outcome::Copied => {
            block_on(notify::show(
                "Screenshot copied",
                "Paste it anywhere with Ctrl+V.",
            ))?;
            Ok(())
        }
        overlay::Outcome::Canceled => Ok(()),
        overlay::Outcome::Edit(path) => {
            if !block_on(ipc::hand_over(Some(&path)))? {
                editor_command(&std::env::current_exe()?, &path)
                    .spawn()
                    .context("cannot open the editor")?;
            }
            Ok(())
        }
        overlay::Outcome::Failed(error) => anyhow::bail!(error),
    }
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
