//! Small RON files Kirie keeps between runs, always written whole.

use std::path::{Path, PathBuf};

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
    let partial = path.with_extension("ron.partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
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
        assert!(!path.with_extension("ron.partial").exists());
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
