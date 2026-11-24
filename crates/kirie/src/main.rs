mod annotate;
mod canvas;
mod capture;
mod cli;
mod desktop;
mod editor;
mod overlay;
mod runtime;
mod storage;
mod update;

use anyhow::Context;
use clap::Parser;

use cli::{Cli, Command, Debug, GnomeExtension, Shortcuts};
use desktop::{activation, clipboard, extension, ipc, shortcuts, tray};
use runtime::block_on;

pub const APP_ID: &str = "dev.samuelba.Kirie";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        // stdout may carry a picture, with `capture -o -`.
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Some(Command::Capture(args)) => capture::run(args),
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
