//! The desktop's own file dialog, through the `FileChooser` portal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kirie_capture::CaptureError;
use kirie_capture::portal;
use zbus::Variant;

use crate::save::Format;

/// Asks for an image or a `.kirie` project; `None` when the dialog was canceled.
pub async fn open_image() -> Result<Option<PathBuf>, String> {
    let token = portal::token();
    let filters: Vec<(&str, Vec<(u32, &str)>)> = vec![
        (
            "Images and Kirie projects",
            vec![
                (1, "image/png"),
                (1, "image/jpeg"),
                (1, "image/webp"),
                (0, "*.kirie"),
            ],
        ),
        ("Kirie projects", vec![(0, "*.kirie")]),
    ];
    let options: HashMap<&str, Variant<'_>> = HashMap::from([
        ("handle_token", Variant::from(token.as_str())),
        ("modal", Variant::from(true)),
        ("filters", Variant::from(filters)),
    ]);
    let results = match portal::request(
        "org.freedesktop.portal.FileChooser",
        "OpenFile",
        &token,
        &("", "Open an image", options),
    )
    .await
    {
        Ok(results) => results,
        Err(CaptureError::Canceled) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let uris: Vec<String> = results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.clone()).ok())
        .unwrap_or_default();
    Ok(uris.first().and_then(|uri| portal::file_path(uri)))
}

/// A filter of the file dialog: its name, and patterns (0) or mime types (1).
type Filter = (&'static str, Vec<(u32, &'static str)>);

fn filter(format: Format) -> Filter {
    let globs = format.globs().iter().map(|glob| (0, *glob)).collect();
    (format.label(), globs)
}

/// Asks where to save, in `folder` under `name`, as `formats` with the first picked; `None` when
/// the dialog was canceled.
pub async fn save_as(
    name: &str,
    folder: &Path,
    formats: &[Format],
) -> Result<Option<(PathBuf, Format)>, String> {
    use std::os::unix::ffi::OsStrExt as _;

    let token = portal::token();
    let filters: Vec<Filter> = formats.iter().copied().map(filter).collect();
    let mut folder = folder.as_os_str().as_bytes().to_vec();
    folder.push(0);
    let mut options: HashMap<&str, Variant<'_>> = HashMap::from([
        ("handle_token", Variant::from(token.as_str())),
        ("modal", Variant::from(true)),
        ("current_name", Variant::from(name)),
        ("current_folder", Variant::from(folder)),
        ("filters", Variant::from(filters)),
    ]);
    if let Some(first) = formats.first() {
        options.insert("current_filter", Variant::from(filter(*first)));
    }
    let results = match portal::request(
        "org.freedesktop.portal.FileChooser",
        "SaveFile",
        &token,
        &("", "Save as", options),
    )
    .await
    {
        Ok(results) => results,
        Err(CaptureError::Canceled) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let picked = results
        .get("current_filter")
        .and_then(|value| <(String, Vec<(u32, String)>)>::try_from(value.clone()).ok())
        .and_then(|(label, _)| {
            formats
                .iter()
                .copied()
                .find(|format| format.label() == label)
        });
    let path = results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.clone()).ok())
        .unwrap_or_default()
        .first()
        .and_then(|uri| portal::file_path(uri));
    Ok(path.map(|path| Format::of(&path, picked)))
}
