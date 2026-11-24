//! Where screenshots go and how they are named.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use kirie_annotate::project::Project;

/// What a file chosen in the save dialog is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
    /// A `.kirie` project, whose marks stay editable.
    Project,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Png, Format::Jpeg, Format::Webp, Format::Project];

    /// The name of the save dialog's filter for this format.
    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG image",
            Format::Jpeg => "JPEG image",
            Format::Webp => "WebP image",
            Format::Project => "Kirie project",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Webp => "webp",
            Format::Project => kirie_annotate::project::EXTENSION,
        }
    }

    /// The file name patterns of the save dialog's filter.
    pub fn globs(self) -> &'static [&'static str] {
        match self {
            Format::Png => &["*.png"],
            Format::Jpeg => &["*.jpg", "*.jpeg"],
            Format::Webp => &["*.webp"],
            Format::Project => &["*.kirie"],
        }
    }

    /// The format a file of this name is written in: its extension's, else the one picked in
    /// the dialog, whose extension the name then gets.
    pub fn of(path: &Path, picked: Option<Format>) -> (PathBuf, Format) {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        let known = Format::ALL.into_iter().find(|format| {
            extension.as_deref().is_some_and(|extension| {
                format
                    .globs()
                    .iter()
                    .any(|glob| glob.strip_prefix("*.") == Some(extension))
            })
        });
        if let Some(format) = known {
            return (path.to_path_buf(), format);
        }
        let format = picked.unwrap_or(Format::Png);
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(format.extension());
        (PathBuf::from(name), format)
    }
}

/// The folder screenshots are saved to, as set in the settings.
pub fn screenshots_dir() -> PathBuf {
    crate::storage::settings::Settings::load().folder()
}

/// `~/Pictures/Screenshots`, or the home folder's `Screenshots` without a pictures folder.
pub fn default_dir() -> PathBuf {
    let dirs = directories::UserDirs::new();
    let pictures = dirs
        .as_ref()
        .and_then(directories::UserDirs::picture_dir)
        .map(Path::to_path_buf)
        .or_else(|| dirs.as_ref().map(|dirs| dirs.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    pictures.join("Screenshots")
}

/// The name of a screenshot taken `now`, without extension, as the settings name it.
pub fn stem(now: &jiff::Zoned) -> String {
    crate::storage::settings::Settings::load().stem(now)
}

/// `Kirie_2026-11-22_10-30-15`: the name a capture handed to the editor gets, whatever the
/// settings say, so that only such files are ever deleted.
fn handoff_stem(now: &jiff::Zoned) -> String {
    format!("Kirie_{}", now.strftime("%Y-%m-%d_%H-%M-%S"))
}

/// The most `<stem>_<n>` names tried before a save gives up.
const MOST_NAMES: u32 = 10_000;

/// `<stem>.<extension>` in `dir` for the first name, `<stem>_<n>.<extension>` after.
fn named(dir: &Path, stem: &str, extension: &str, n: u32) -> PathBuf {
    if n == 1 {
        dir.join(format!("{stem}.{extension}"))
    } else {
        dir.join(format!("{stem}_{n}.{extension}"))
    }
}

/// Writes a new file in `dir` under the first free name, `Kirie_2026-11-22_10-30-15.png` and then
/// `Kirie_2026-11-22_10-30-15_2.png` and on. Creating the file is what claims its name, so two
/// saves at once never write into the same one.
fn create_named(
    dir: &Path,
    stem: &str,
    extension: &str,
    mut options: std::fs::OpenOptions,
    fill: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    options.write(true).create_new(true);
    let (path, file) = (1..=MOST_NAMES)
        .map(|n| named(dir, stem, extension, n))
        .find_map(|path| match options.open(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
            opened => Some(opened.map(|file| (path, file))),
        })
        .unwrap_or_else(|| Err(std::io::Error::other("every name is taken")))
        .map_err(|error| format!("cannot write {}: {error}", dir.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    match fill(&mut writer).and_then(|()| writer.flush()) {
        Ok(()) => Ok(path),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(format!("cannot write {}: {error}", path.display()))
        }
    }
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
    let mut options = std::fs::OpenOptions::new();
    options.mode(0o600);
    create_named(
        &dir,
        &handoff_stem(&jiff::Zoned::now()),
        kirie_annotate::project::EXTENSION,
        options,
        |writer| kirie_annotate::project::save(project, writer).map_err(std::io::Error::other),
    )
}

/// Writes an editable `.kirie` project into the screenshots folder.
pub async fn save_project(project: kirie_annotate::project::Project) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let dir = screenshots_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        create_named(
            &dir,
            &stem(&jiff::Zoned::now()),
            kirie_annotate::project::EXTENSION,
            std::fs::OpenOptions::new(),
            |writer| kirie_annotate::project::save(&project, writer).map_err(std::io::Error::other),
        )
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
        create_named(
            &dir,
            &stem(&jiff::Zoned::now()),
            "png",
            std::fs::OpenOptions::new(),
            |writer| {
                image
                    .write_to(writer, image::ImageFormat::Png)
                    .map_err(std::io::Error::other)
            },
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Writes `image` into the screenshots folder and puts it on the clipboard too: the file, and
/// whether the copy worked. Only the save must work. Where no background process can hold the
/// clipboard, the window of `device` does.
pub async fn save_and_copy(
    image: RgbaImage,
    device: Option<crate::desktop::drops::Device>,
) -> Result<(PathBuf, bool), String> {
    let path = save(image.clone()).await?;
    if !crate::storage::settings::Settings::load().copy_on_save {
        return Ok((path, false));
    }
    let copied = crate::desktop::clipboard::copy_from(image, device)
        .await
        .inspect_err(|error| tracing::warn!(%error, "saved, but not copied"))
        .is_ok();
    Ok((path, copied))
}

/// Writes `project` to `path`, a file the user chose, as `format`: the finished picture, or the
/// project itself.
pub async fn save_as(project: Project, path: PathBuf, format: Format) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let picture = match format {
            Format::Project => None,
            _ => Some(
                kirie_annotate::render::render(
                    &project.document,
                    &project.image,
                    &mut kirie_annotate::Typesetter::new(),
                )
                .map_err(|error| error.to_string())?,
            ),
        };
        // Through a link, the file it points at is the one replaced, and it keeps its mode.
        let target = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let mode = std::fs::metadata(&target)
            .ok()
            .map(|meta| meta.permissions());
        crate::storage::store::replace_with(&target, |file| {
            if let Some(mode) = mode {
                file.set_permissions(mode)?;
            }
            let mut writer = std::io::BufWriter::new(file);
            match picture {
                None => kirie_annotate::project::save(&project, &mut writer)
                    .map_err(std::io::Error::other)?,
                Some(picture) => {
                    encode(picture, format, &mut writer).map_err(std::io::Error::other)?;
                }
            }
            writer.flush()
        })
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(path)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn encode(
    picture: RgbaImage,
    format: Format,
    writer: &mut (impl std::io::Write + std::io::Seek),
) -> image::ImageResult<()> {
    match format {
        Format::Jpeg => image::DynamicImage::ImageRgba8(picture)
            .to_rgb8()
            .write_to(writer, image::ImageFormat::Jpeg),
        Format::Webp => picture.write_to(writer, image::ImageFormat::WebP),
        Format::Png | Format::Project => picture.write_to(writer, image::ImageFormat::Png),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noon() -> jiff::Zoned {
        "2026-11-22T12:00:05[UTC]".parse().unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kirie-save-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create(dir: &Path, text: &'static str) -> Result<PathBuf, String> {
        create_named(
            dir,
            &stem(&noon()),
            "png",
            std::fs::OpenOptions::new(),
            |writer| writer.write_all(text.as_bytes()),
        )
    }

    #[test]
    fn names_carry_the_time() {
        let name = |n| named(Path::new("/shots"), &stem(&noon()), "png", n);
        assert_eq!(name(1), Path::new("/shots/Kirie_2026-11-22_12-00-05.png"));
        assert_eq!(name(3), Path::new("/shots/Kirie_2026-11-22_12-00-05_3.png"));
    }

    #[test]
    fn taken_names_get_a_counter_and_stay_as_they_were() {
        let dir = scratch("taken");
        let first = create(&dir, "first").unwrap();
        std::fs::write(dir.join("Kirie_2026-11-22_12-00-05_2.png"), "second").unwrap();
        let third = create(&dir, "third").unwrap();
        assert_eq!(third, dir.join("Kirie_2026-11-22_12-00-05_3.png"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(
            std::fs::read_to_string(dir.join("Kirie_2026-11-22_12-00-05_2.png")).unwrap(),
            "second"
        );
        assert_eq!(std::fs::read_to_string(&third).unwrap(), "third");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_at_once_each_get_a_name_of_their_own() {
        let dir = scratch("at-once");
        let start = std::sync::Barrier::new(16);
        let mut saved: Vec<PathBuf> = std::thread::scope(|scope| {
            let saving: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        create(&dir, "shot").unwrap()
                    })
                })
                .collect();
            saving
                .into_iter()
                .map(|save| save.join().unwrap())
                .collect()
        });
        saved.sort();
        saved.dedup();
        assert_eq!(saved.len(), 16);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_save_leaves_no_file() {
        let dir = scratch("failed");
        let failed = create_named(&dir, "shot", "png", std::fs::OpenOptions::new(), |_| {
            Err(std::io::Error::other("the encoder gave up"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
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
    fn chosen_names_keep_a_known_extension_or_get_the_picked_one() {
        let of = |name: &str, picked| Format::of(Path::new(name), picked);
        assert_eq!(
            of("/a/shot.PNG", None),
            (PathBuf::from("/a/shot.PNG"), Format::Png)
        );
        assert_eq!(of("/a/shot.jpeg", Some(Format::Project)).1, Format::Jpeg);
        assert_eq!(of("/a/shot.kirie", None).1, Format::Project);
        assert_eq!(
            of("/a/shot", Some(Format::Project)),
            (PathBuf::from("/a/shot.kirie"), Format::Project)
        );
        assert_eq!(
            of("/a/v1.2", None),
            (PathBuf::from("/a/v1.2.png"), Format::Png),
            "an unknown extension is part of the name"
        );
    }

    #[test]
    fn saving_as_writes_the_chosen_format() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("kirie-save-as-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = Project {
            image: RgbaImage::from_pixel(4, 3, image::Rgba([200, 10, 10, 255])),
            document: kirie_annotate::Document::new(4, 3, 1.0),
        };
        for format in Format::ALL {
            let path = dir.join(format!("shot.{}", format.extension()));
            let saved = runtime
                .block_on(save_as(project.clone(), path.clone(), format))
                .unwrap();
            assert_eq!(saved, path);
            if format == Format::Project {
                let file = std::fs::File::open(&path).unwrap();
                let loaded = kirie_annotate::project::load(std::io::BufReader::new(file)).unwrap();
                assert_eq!(loaded.image, project.image);
            } else {
                assert_eq!(image::open(&path).unwrap().width(), 4);
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_as_over_a_file_replaces_it_whole() {
        use std::os::unix::fs::PermissionsExt as _;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let dir = scratch("over");
        let project = Project {
            image: RgbaImage::from_pixel(4, 3, image::Rgba([200, 10, 10, 255])),
            document: kirie_annotate::Document::new(4, 3, 1.0),
        };
        let real = dir.join("real.png");
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.join("link.png");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let saved = runtime
            .block_on(save_as(project, link.clone(), Format::Png))
            .unwrap();
        assert_eq!(saved, link);
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(image::open(&real).unwrap().width(), 4);
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_folder_is_called_screenshots() {
        assert!(screenshots_dir().ends_with("Screenshots"));
        assert_eq!(
            screenshots_dir(),
            default_dir(),
            "unless the settings say otherwise"
        );
    }

    #[test]
    fn handoffs_keep_kiries_own_names() {
        assert_eq!(handoff_stem(&noon()), "Kirie_2026-11-22_12-00-05");
    }
}
