use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use kirie_capture::{Cursor, geometry};

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
    },
    /// Open an image in the editor.
    Edit { path: PathBuf },
    /// Information for bug reports.
    #[command(subcommand, hide = true)]
    Debug(Debug),
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
        }) => capture_to_file(&path, cursor),
        Some(Command::Debug(Debug::Globals)) => print_globals(),
        None | Some(Command::Capture { .. } | Command::Edit { .. }) => {
            anyhow::bail!("not implemented yet")
        }
    }
}

fn capture_to_file(path: &std::path::Path, cursor: bool) -> anyhow::Result<()> {
    let cursor = if cursor { Cursor::Shown } else { Cursor::Hidden };
    let (protocol, frames) = kirie_capture::wayland::capture(cursor)?;
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
