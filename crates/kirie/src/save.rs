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

/// Where the overlay leaves a capture for the editor: the user's private runtime folder, else
/// the user's cache folder; never a shared temporary folder.
pub fn handoff_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.cache_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kirie")
}

/// Whether `path` is a capture the overlay handed over: a `Kirie_*.kirie` regular file directly
/// inside the hand-off folder, after resolving `..` and links. Only those are deleted once read.
pub fn is_handoff(path: &Path) -> bool {
    let (Ok(path), Ok(dir)) = (path.canonicalize(), handoff_dir().canonicalize()) else {
        return false;
    };
    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "only names exactly as Kirie writes them are ever deleted"
    )]
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("Kirie_") && name.ends_with(".kirie"));
    named
        && path.parent() == Some(dir.as_path())
        && std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file())
}

/// Writes a project for the editor to pick up, readable by the user alone.
pub fn save_handoff(project: &kirie_annotate::project::Project) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};

    let dir = handoff_dir();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("cannot protect {}: {error}", dir.display()))?;
    let path = named(
        &dir,
        &jiff::Zoned::now(),
        kirie_annotate::project::EXTENSION,
        Path::exists,
    );
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    kirie_annotate::project::save(project, std::io::BufWriter::new(file))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
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
    fn handoffs_are_only_kirie_files_in_the_handoff_folder() {
        use std::os::unix::fs::PermissionsExt as _;

        let project = kirie_annotate::project::Project {
            image: RgbaImage::new(2, 2),
            document: kirie_annotate::Document::new(2, 2, 1.0),
        };
        let path = save_handoff(&project).unwrap();
        assert!(is_handoff(&path));
        let sneaky = handoff_dir()
            .join("..")
            .join("kirie")
            .join(path.file_name().unwrap());
        assert!(
            is_handoff(&sneaky),
            "the same file through .. is still the hand-off"
        );
        assert!(!is_handoff(&handoff_dir().join("../../etc/passwd")));
        assert!(!is_handoff(Path::new("/elsewhere/Kirie_x.kirie")));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "only the user may read it");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_folder_is_called_screenshots() {
        assert!(screenshots_dir().ends_with("Screenshots"));
    }
}
