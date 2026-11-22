mod clipboard;
mod controls;
mod editor;
mod ipc;
mod overlay;
mod paint;
mod save;

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
    /// Open an image in the editor.
    Edit { path: PathBuf },
    /// Information for bug reports.
    #[command(subcommand, hide = true)]
    Debug(Debug),
    /// Offers a PNG from stdin on the clipboard until something else is copied.
    #[command(name = "serve-clipboard", hide = true)]
    ServeClipboard,
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
        None => edit(None),
        Some(Command::Edit { path }) => edit(Some(&path)),
        Some(Command::Capture { .. }) => {
            anyhow::bail!("this capture mode is not implemented yet")
        }
    }
}

fn capture_to_file(path: &std::path::Path, cursor: bool) -> anyhow::Result<()> {
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
    let cursor = if cursor {
        Cursor::Shown
    } else {
        Cursor::Hidden
    };
    let (_, frames) = block_on(kirie_capture::capture(cursor))??;
    let primary = if edit {
        overlay::Action::Edit
    } else {
        overlay::Action::Copy
    };
    let host = if kirie_capture::wayland::globals().is_ok_and(|globals| globals.layer_shell)
        && std::env::var("KIRIE_OVERLAY").as_deref() != Ok("window")
    {
        overlay::Host::Layer
    } else {
        overlay::Host::Window
    };
    match overlay::run(frames, primary, host).map_err(anyhow::Error::msg)? {
        overlay::Outcome::Saved(path) => {
            println!("{}", path.display());
            Ok(())
        }
        overlay::Outcome::Copied | overlay::Outcome::Cancelled => Ok(()),
        overlay::Outcome::Edit(path) => {
            if !block_on(ipc::hand_over(Some(&path)))? {
                std::process::Command::new(std::env::current_exe()?)
                    .arg("edit")
                    .arg(&path)
                    .spawn()
                    .context("cannot open the editor")?;
            }
            Ok(())
        }
        overlay::Outcome::Failed(error) => anyhow::bail!(error),
    }
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
