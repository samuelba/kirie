//! Where screenshots go and how they are named.

use std::path::{Path, PathBuf};

use image::RgbaImage;

/// `~/Pictures/Screenshots`, or the home folder's `Screenshots` without a pictures folder.
pub fn screenshots_dir() -> PathBuf {
    let dirs = directories::UserDirs::new();
    let pictures = dirs
        .as_ref()
        .and_then(directories::UserDirs::picture_dir)
        .map(Path::to_path_buf)
        .or_else(|| dirs.as_ref().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    pictures.join("Screenshots")
}

/// `Kirie_2026-11-22_10-30-15.png`, with a counter when that name is taken.
pub fn file_name(dir: &Path, now: &jiff::Zoned, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let stem = format!("Kirie_{}", now.strftime("%Y-%m-%d_%H-%M-%S"));
    let first = dir.join(format!("{stem}.png"));
    if !taken(&first) {
        return first;
    }
    (2..u32::MAX)
        .map(|n| dir.join(format!("{stem}_{n}.png")))
        .find(|path| !taken(path))
        .unwrap_or(first)
}

/// Writes `image` as a PNG into the screenshots folder.
pub async fn save(image: RgbaImage) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let dir = screenshots_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        let path = file_name(&dir, &jiff::Zoned::now(), Path::exists);
        image
            .save(&path)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noon() -> jiff::Zoned {
        "2026-11-22T12:00:05[UTC]".parse().unwrap()
    }

    #[test]
    fn names_carry_the_time() {
        let path = file_name(Path::new("/shots"), &noon(), |_| false);
        assert_eq!(path, Path::new("/shots/Kirie_2026-11-22_12-00-05.png"));
    }

    #[test]
    fn taken_names_get_a_counter() {
        let path = file_name(Path::new("/shots"), &noon(), |path| {
            !path.ends_with("Kirie_2026-11-22_12-00-05_3.png")
        });
        assert_eq!(path, Path::new("/shots/Kirie_2026-11-22_12-00-05_3.png"));
    }

    #[test]
    fn the_folder_is_called_screenshots() {
        assert!(screenshots_dir().ends_with("Screenshots"));
    }
}
