//! Screenshots with the overlay, where the user picks a region, a window or a screen.

use std::path::Path;

use kirie_capture::Cursor;

use crate::desktop::{dialog, ipc, notify, tray};
use crate::overlay;
use crate::runtime::block_on;
use crate::storage::save;

/// Takes a screenshot with the overlay, and starts the tray icon beside it if none is shown.
pub fn run(cursor: bool, edit: bool, delay: u64) -> anyhow::Result<()> {
    let tray = std::thread::spawn(|| block_on(tray::ensure()));
    super::wait(delay);
    let taken = take(cursor, edit);
    let _ = tray.join();
    taken
}

fn take(cursor: bool, edit: bool) -> anyhow::Result<()> {
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
    let outcome = overlay::run(frames, windows, primary, host());
    if edit && !matches!(outcome, Ok(overlay::Outcome::Edit(_))) {
        block_on(ipc::resume())?;
    }
    deliver(outcome.map_err(anyhow::Error::msg)?)
}

/// Where the overlay shows: in the GNOME Shell extension's window, on a layer above every
/// window, or in a plain window where the compositor has no layers or `KIRIE_OVERLAY=window`.
fn host() -> overlay::Host {
    if kirie_capture::shell::launched() {
        overlay::Host::Shell
    } else if kirie_capture::wayland::globals().is_ok_and(|globals| globals.layer_shell)
        && std::env::var("KIRIE_OVERLAY").as_deref() != Ok("window")
    {
        overlay::Host::Layer
    } else {
        overlay::Host::Window
    }
}

/// Finishes what the overlay was closed with.
fn deliver(outcome: overlay::Outcome) -> anyhow::Result<()> {
    match outcome {
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
        overlay::Outcome::Edit(path) => super::handoff::open(&path),
        overlay::Outcome::Failed(error) => anyhow::bail!(error),
    }
}

/// Tells where the overlay's picture was saved.
fn saved(path: &Path, copied: bool) -> anyhow::Result<()> {
    println!("{}", path.display());
    block_on(notify::saved(path, copied))?;
    Ok(())
}
