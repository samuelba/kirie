//! Handing files and links to the desktop.

use std::process::{Command, Stdio};

/// Opens `target`, a link or a file, in the app the desktop picks for it.
pub fn open(target: impl AsRef<std::ffi::OsStr>) {
    let mut command = Command::new("xdg-open");
    command.arg(target);
    spawn(command);
}

/// Starts `command` without waiting for it, and reaps it once it ends.
fn spawn(mut command: Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => tracing::warn!(%error, "cannot start {:?}", command.get_program()),
    }
}
