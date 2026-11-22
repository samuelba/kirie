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
    named(dir, now, "png", taken)
}

fn named(dir: &Path, now: &jiff::Zoned, extension: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let stem = format!("Kirie_{}", now.strftime("%Y-%m-%d_%H-%M-%S"));
    let first = dir.join(format!("{stem}.{extension}"));
    if !taken(&first) {
        return first;
    }
    (2..u32::MAX)
        .map(|n| dir.join(format!("{stem}_{n}.{extension}")))
        .find(|path| !taken(path))
        .unwrap_or(first)
}

/// A path for messages, with the home folder shown as `~`.
pub fn pretty(path: &Path) -> String {
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Writes an editable `.kirie` project into the screenshots folder.
pub async fn save_project(project: kirie_annotate::project::Project) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let dir = screenshots_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        let path = named(
            &dir,
            &jiff::Zoned::now(),
            kirie_annotate::project::EXTENSION,
            Path::exists,
        );
        let file = std::fs::File::create(&path)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        kirie_annotate::project::save(&project, std::io::BufWriter::new(file))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(path)
    })
    .await
    .map_err(|error| error.to_string())?
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
    fn paths_under_home_are_shortened() {
        if let Some(dirs) = directories::BaseDirs::new() {
            let path = dirs.home_dir().join("Pictures/a.png");
            assert_eq!(pretty(&path), "~/Pictures/a.png");
        }
        assert_eq!(pretty(Path::new("/tmp/a.png")), "/tmp/a.png");
    }

    #[test]
    fn the_folder_is_called_screenshots() {
        assert!(screenshots_dir().ends_with("Screenshots"));
    }
}
