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

/// What the file at `path` holds, or `None` when there is none or it cannot be read.
pub fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cannot read");
            return None;
        }
    };
    ron::from_str(&text)
        .inspect_err(|error| tracing::warn!(path = %path.display(), %error, "cannot parse"))
        .ok()
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
pub fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let partial = write_beside(path, text)?;
    std::fs::rename(&partial, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })
}

/// Numbers the temporary files of this process.
static WRITES: AtomicU32 = AtomicU32::new(0);

/// Writes `text` to a new hidden file next to `path`, flushed to disk, and returns where. Each
/// write gets its own file, so the editor, the overlay and the tray never write into one
/// another's.
pub fn write_beside(path: &Path, text: &str) -> std::io::Result<PathBuf> {
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
    match file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
    {
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
        assert_eq!(read::<String>(&path), None);
        remove(&path).unwrap();
        remove(&path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
