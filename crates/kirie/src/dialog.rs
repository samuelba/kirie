//! The desktop's own file dialog, through the `FileChooser` portal.

use std::collections::HashMap;
use std::path::PathBuf;

use kirie_capture::CaptureError;
use kirie_capture::portal;
use zbus::Variant;

/// Asks for an image or a `.kirie` project; `None` when the dialog was cancelled.
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
        Err(CaptureError::Cancelled) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let uris: Vec<String> = results
        .get("uris")
        .and_then(|uris| Vec::<String>::try_from(uris.clone()).ok())
        .unwrap_or_default();
    Ok(uris.first().and_then(|uri| portal::file_path(uri)))
}
