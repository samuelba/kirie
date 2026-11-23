//! Handing files and links to the desktop: the default app for a file or a link, and the file
//! manager with a file selected.

use std::path::Path;
use std::process::{Command, Stdio};

/// Opens `target`, a link or a file, in the app the desktop picks for it.
pub fn open(target: impl AsRef<std::ffi::OsStr>) {
    let mut command = Command::new("xdg-open");
    command.arg(target);
    spawn(command);
}

/// Shows `path` selected in the file manager, or its folder when no file manager takes the
/// request.
pub async fn show_in_files(path: &Path) {
    let uri = file_uri(path);
    let shown = async {
        let connection = zbus::Connection::session().await?;
        connection
            .call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
            .await?;
        Ok::<_, zbus::Error>(())
    };
    if let Err(error) = shown.await {
        tracing::debug!(%error, "no file manager to show the file in");
        if let Some(folder) = path.parent() {
            open(folder);
        }
    }
}

/// Starts `command` without waiting for it, and reaps it once it ends. It gets no pipes of
/// Kirie's, which a reader would wait on, and a process group of its own, so that Ctrl+C in the
/// terminal Kirie runs in leaves it be.
pub fn spawn(mut command: Command) {
    use std::os::unix::process::CommandExt as _;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(error) => tracing::warn!(%error, "cannot start {:?}", command.get_program()),
    }
}

/// `file://` and the path, with every byte but unreserved ones and `/` escaped.
pub fn file_uri(path: &Path) -> String {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt as _;

    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_escape_what_is_not_plain() {
        assert_eq!(
            file_uri(Path::new("/home/me/Shot one#2é.png")),
            "file:///home/me/Shot%20one%232%C3%A9.png"
        );
    }
}
