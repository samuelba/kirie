//! Small RON files Kirie keeps between runs, always written whole.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// `~/.local/state/kirie/<name>`: what Kirie remembers from one run to the next. Tests get
/// none, so they never read or change what the user's Kirie remembers.
pub fn state_path(name: &str) -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let dirs = directories::BaseDirs::new()?;
    let state = dirs
        .state_dir()
        .map_or_else(|| dirs.data_local_dir().to_path_buf(), Path::to_path_buf);
    Some(state.join("kirie").join(name))
}

/// What the file at `path` holds, or `None` when there is none or it cannot be read. What a file
/// that cannot be parsed held, such as one a newer Kirie wrote, is copied to `<file>.bak` first,
/// so the next save does not lose it; a later unreadable copy replaces an earlier one.
pub fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read");
            return None;
        }
    };
    let parsed = std::str::from_utf8(&bytes)
        .map_err(|error| error.to_string())
        .and_then(|text| ron::from_str(text).map_err(|error| error.to_string()));
    parsed
        .inspect_err(|error| keep_aside(path, &bytes, error))
        .ok()
}

/// Copies an unreadable file's `bytes` to `<file>.bak`. Copying rather than moving the file
/// leaves alone whatever another Kirie saves to `path` meanwhile.
fn keep_aside(path: &Path, bytes: &[u8], error: &str) {
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bak");
    let backup = PathBuf::from(backup);
    tracing::warn!(
        path = %path.display(),
        backup = %backup.display(),
        %error,
        "cannot parse; keeping a copy and starting afresh"
    );
    if let Err(error) = write(&backup, bytes) {
        tracing::warn!(backup = %backup.display(), %error, "cannot keep a copy of the unreadable file");
    }
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())
        .map_err(|error| error.to_string())?;
    write(path, &text).map_err(|error| format!("cannot save {}: {error}", path.display()))
}

pub fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("cannot remove {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Writes through a temporary file so a crash never leaves half a file behind.
pub fn write(path: &Path, text: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    replace_with(path, |file| file.write_all(text.as_ref()))
}

/// Replaces `path` with a file that `fill` writes beside it, so a failure halfway leaves the old
/// file as it was.
pub fn replace_with(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let partial = fill_beside(path, fill)?;
    std::fs::rename(&partial, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })
}

/// Numbers the temporary files of this process.
static WRITES: AtomicU32 = AtomicU32::new(0);

/// Writes `text` to a new hidden file next to `path`, flushed to disk, and returns where.
pub fn write_beside(path: &Path, text: impl AsRef<[u8]>) -> std::io::Result<PathBuf> {
    fill_beside(path, |file| file.write_all(text.as_ref()))
}

/// A new hidden file next to `path`, written by `fill` and flushed to disk. Each write gets its
/// own file, so the editor, the overlay and the tray never write into one another's.
fn fill_beside(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
    })?;
    let mut tries = 0;
    // A name already taken is left over from an earlier Kirie with the same process id.
    let (partial, mut file) = loop {
        let write = WRITES.fetch_add(1, Ordering::Relaxed);
        let partial = path.with_file_name(format!(
            ".{}.kirie-{}-{write}",
            name.to_string_lossy(),
            std::process::id()
        ));
        match std::fs::File::create_new(&partial) {
            Ok(file) => break (partial, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && tries < 8 => {
                tries += 1;
            }
            Err(error) => return Err(error),
        }
    };
    match fill(&mut file).and_then(|()| file.sync_all()) {
        Ok(()) => Ok(partial),
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kirie-store-{name}-{}", std::process::id()))
    }

    #[test]
    fn saving_replaces_the_file_whole() {
        let folder = folder("whole");
        let path = folder.join("nested/colors.ron");
        write(&path, "one").unwrap();
        write(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_failed_replacement_leaves_the_old_file() {
        let folder = folder("failed");
        let path = folder.join("shot.png");
        write(&path, "old").unwrap();
        let failed = replace_with(&path, |file| {
            file.write_all(b"half")?;
            Err(std::io::Error::other("the encoder gave up"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn leftover_temporary_files_are_skipped_and_kept() {
        let folder = folder("leftover");
        let path = folder.join("settings.ron");
        std::fs::create_dir_all(&folder).unwrap();
        let next = WRITES.load(Ordering::Relaxed);
        for write in next..next + 3 {
            let name = format!(".settings.ron.kirie-{}-{write}", std::process::id());
            std::fs::write(folder.join(name), "crashed").unwrap();
        }
        write(&path, "saved").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 4);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn unreadable_files_are_kept_aside() {
        let folder = folder("aside");
        let path = folder.join("settings.ron");
        let backup = folder.join("settings.ron.bak");
        write(&path, "(folder: Some(\"/shots\"), oops").unwrap();
        assert_eq!(read::<Vec<u32>>(&path), None);
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "(folder: Some(\"/shots\"), oops"
        );
        save(&path, &vec![7u32]).unwrap();
        assert_eq!(read::<Vec<u32>>(&path), Some(vec![7]));
        assert!(backup.exists());

        assert_eq!(read::<String>(&path), None, "a value of another kind");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            std::fs::read_to_string(&path).unwrap()
        );
        write(&path, [0xff, 0xfe, b'(']).unwrap();
        assert_eq!(read::<Vec<u32>>(&path), None);
        assert_eq!(std::fs::read(&backup).unwrap(), [0xff, 0xfe, b'(']);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn writers_at_once_each_leave_a_whole_file() {
        let folder = folder("at-once");
        let path = folder.join("settings.ron");
        let texts: Vec<String> = (0..8)
            .map(|writer| writer.to_string().repeat(64 * 1024))
            .collect();
        std::thread::scope(|scope| {
            for text in &texts {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..20 {
                        write(path, text).unwrap();
                    }
                });
            }
        });
        let left = std::fs::read_to_string(&path).unwrap();
        assert!(texts.contains(&left));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn values_survive_a_round_trip_and_go_away() {
        let folder = folder("round-trip");
        let path = folder.join("value.ron");
        assert_eq!(read::<Vec<u32>>(&path), None);
        save(&path, &vec![1u32, 2, 3]).unwrap();
        assert_eq!(read::<Vec<u32>>(&path), Some(vec![1, 2, 3]));
        remove(&path).unwrap();
        remove(&path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
