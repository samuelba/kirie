//! `.kirie` projects: the untouched image and its marks, so annotations stay editable.
//!
//! A project is a zip with `manifest.ron`, `image.png` and `document.ron`.

use std::io::{Read, Seek, Write};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;

use crate::document::Document;

pub const EXTENSION: &str = "kirie";
const FORMAT: &str = "kirie";
/// 2: weights are widths, and text has a look of its own.
const VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub image: RgbaImage,
    pub document: Document,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("not a Kirie project: {0}")]
    NotAProject(String),
    #[error("this project needs a newer Kirie (format version {0})")]
    TooNew(u32),
    #[error("the project's image is {got:?}, its marks expect {expected:?}")]
    SizeMismatch {
        expected: (u32, u32),
        got: (u32, u32),
    },
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Image(#[from] image::ImageError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the marks cannot be read: {0}")]
    Decode(#[from] ron::error::SpannedError),
    #[error("the marks cannot be written: {0}")]
    Encode(#[from] ron::Error),
}

pub fn save(project: &Project, writer: impl Write + Seek) -> Result<(), ProjectError> {
    let mut zip = zip::ZipWriter::new(writer);
    let text = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let pretty = ron::ser::PrettyConfig::default();

    zip.start_file("manifest.ron", text)?;
    zip.write_all(
        ron::ser::to_string_pretty(
            &Manifest {
                format: FORMAT.into(),
                version: VERSION,
            },
            pretty.clone(),
        )?
        .as_bytes(),
    )?;
    zip.start_file("image.png", stored)?;
    let mut png = Vec::new();
    project
        .image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    zip.write_all(&png)?;
    zip.start_file("document.ron", text)?;
    zip.write_all(ron::ser::to_string_pretty(&project.document, pretty)?.as_bytes())?;
    zip.finish()?;
    Ok(())
}

pub fn load(reader: impl Read + Seek) -> Result<Project, ProjectError> {
    let mut zip = zip::ZipArchive::new(reader)
        .map_err(|error| ProjectError::NotAProject(error.to_string()))?;
    let manifest: Manifest = ron::from_str(&read_text(&mut zip, "manifest.ron")?)?;
    if manifest.format != FORMAT {
        return Err(ProjectError::NotAProject(format!(
            "its format is {:?}",
            manifest.format
        )));
    }
    if manifest.version > VERSION {
        return Err(ProjectError::TooNew(manifest.version));
    }
    let mut png = Vec::new();
    zip.by_name("image.png")?.read_to_end(&mut png)?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.to_rgba8();
    let document: Document = ron::from_str(&read_text(&mut zip, "document.ron")?)?;
    if image.dimensions() != (document.width, document.height) {
        return Err(ProjectError::SizeMismatch {
            expected: (document.width, document.height),
            got: image.dimensions(),
        });
    }
    Ok(Project { image, document })
}

fn read_text(
    zip: &mut zip::ZipArchive<impl Read + Seek>,
    name: &str,
) -> Result<String, ProjectError> {
    let mut text = String::new();
    zip.by_name(name)
        .map_err(|_| ProjectError::NotAProject(format!("it has no {name}")))?
        .read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use kurbo::{Point, Rect};

    use super::*;
    use crate::document::Op;
    use crate::shape::{Kind, Paint};
    use crate::style::StyleId;

    fn sample() -> Project {
        let document = Document::new(8, 4, 2.0)
            .apply(Op::SetStyle {
                style: StyleId::Ink,
                recolor: Vec::new(),
            })
            .apply(Op::Add {
                kind: Kind::Arrow {
                    from: Point::new(1.0, 1.0),
                    to: Point::new(6.0, 3.0),
                },
                paint: Paint::default(),
            })
            .apply(Op::Add {
                kind: Kind::Text {
                    at: Point::ORIGIN,
                    text: "héllo\n世界".into(),
                },
                paint: Paint::default(),
            })
            .apply(Op::Crop(Some(Rect::new(1.0, 1.0, 7.0, 3.0))));
        Project {
            image: RgbaImage::from_fn(8, 4, |x, y| {
                image::Rgba([
                    u8::try_from(x * 30).unwrap(),
                    u8::try_from(y * 60).unwrap(),
                    9,
                    255,
                ])
            }),
            document,
        }
    }

    fn bytes(project: &Project) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        save(project, &mut out).unwrap();
        out.into_inner()
    }

    #[test]
    fn projects_round_trip() {
        let project = sample();
        let loaded = load(Cursor::new(bytes(&project))).unwrap();
        assert_eq!(loaded, project);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(matches!(
            load(Cursor::new(b"not a zip".to_vec())),
            Err(ProjectError::NotAProject(_))
        ));
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file("manifest.ron", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"(format: \"other\", version: 1)").unwrap();
        zip.finish().unwrap();
        assert!(matches!(
            load(Cursor::new(out.into_inner())),
            Err(ProjectError::NotAProject(_))
        ));
    }

    #[test]
    fn newer_formats_are_refused() {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file("manifest.ron", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"(format: \"kirie\", version: 99)").unwrap();
        zip.finish().unwrap();
        assert!(matches!(
            load(Cursor::new(out.into_inner())),
            Err(ProjectError::TooNew(99))
        ));
    }

    #[test]
    fn a_missing_part_is_reported() {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file("image.png", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        let error = load(Cursor::new(out.into_inner())).unwrap_err();
        assert!(error.to_string().contains("manifest.ron"), "{error}");
    }

    #[test]
    fn the_image_must_fit_the_marks() {
        let mut project = sample();
        project.image = RgbaImage::new(3, 3);
        let error = load(Cursor::new(bytes(&project))).unwrap_err();
        assert!(matches!(error, ProjectError::SizeMismatch { .. }));
    }
}
