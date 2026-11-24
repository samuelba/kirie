//! Starting things: Kirie itself again, from the binary on disk, and files and links in the
//! desktop's apps, with the file manager showing a file selected.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// What Linux appends to the running binary's path once an update replaced the file.
const REPLACED: &str = " (deleted)";

/// The binary Kirie runs from, to start Kirie again. After an update this is the new binary,
/// at the path the old one had.
pub fn exe() -> std::io::Result<PathBuf> {
    std::env::current_exe().map(|path| without_replaced(&path))
}

/// The path of the running binary once an update replaced it on disk, where the new one is.
pub fn replaced_binary() -> Option<PathBuf> {
    let running = std::env::current_exe().ok()?;
    let path = without_replaced(&running);
    (path != running).then_some(path)
}

fn without_replaced(path: &Path) -> PathBuf {
    path.to_str()
        .and_then(|path| path.strip_suffix(REPLACED))
        .map_or_else(|| path.to_owned(), PathBuf::from)
}

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
        Ok(child) => reap(child),
        Err(error) => tracing::warn!(%error, "cannot start {:?}", command.get_program()),
    }
}

/// Waits for `child` in the background, so that it leaves no zombie once it ends.
pub fn reap(mut child: Child) {
    std::thread::spawn(move || child.wait());
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
    fn a_replaced_binary_is_started_from_its_path() {
        assert_eq!(
            without_replaced(Path::new("/home/me/.local/bin/kirie (deleted)")),
            Path::new("/home/me/.local/bin/kirie")
        );
        assert_eq!(
            without_replaced(Path::new("/usr/bin/kirie")),
            Path::new("/usr/bin/kirie")
        );
    }

    #[test]
    fn a_reaped_child_leaves_no_zombie() {
        let child = Command::new("true").spawn().unwrap();
        let entry = PathBuf::from(format!("/proc/{}", child.id()));
        assert!(entry.exists(), "{entry:?} is missing before the reap");
        reap(child);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while entry.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "{entry:?} is a zombie"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn file_uris_escape_what_is_not_plain() {
        assert_eq!(
            file_uri(Path::new("/home/me/Shot one#2é.png")),
            "file:///home/me/Shot%20one%232%C3%A9.png"
        );
    }
}
