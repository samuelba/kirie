//! Copying, saving, opening and pasting pictures, and what is dropped on the window.

use std::path::{Path, PathBuf};

use ::image::RgbaImage;
use iced::{Task, window};
use kirie_annotate::project::{self, Project};
use kirie_annotate::{Document, Shape};

use super::closing::{Closing, Then};
use super::{Editor, MarksEvent, Message, Offer, Pasted};

/// Copying, saving, opening and pasting pictures, and what is dropped on the window.
#[derive(Debug, Clone)]
pub enum Event {
    Copy,
    Open,
    Paste,
    Save,
    SaveProject,
    /// Asks where to save, and as what.
    SaveAs,
    SaveTo(PathBuf, crate::storage::save::Format),
    Dropped(crate::desktop::drops::Dropped),
    /// What became of a save, a copy or a file dialog; empty text tells nothing.
    Done(Result<String, String>),
    /// The image left the editor, saved (to the file) or copied, as this document had it.
    Exported(String, Option<PathBuf>, Box<Document>),
}

/// Where the picture goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Export {
    Copy,
    Save,
    Project,
    /// To a file the user chose.
    To(PathBuf, crate::storage::save::Format),
}

/// The window's data device, for drops and the clipboard.
#[derive(Debug, Default)]
pub(super) struct Drops {
    /// The device, once it is ready.
    pub(super) device: Option<crate::desktop::drops::Device>,
    /// The window the device was last made anew for.
    pub(super) renewed_for: Option<window::Id>,
    /// Something that can be opened is dragged over the window.
    pub(super) hovered: bool,
}

/// An image open for editing.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub image: RgbaImage,
    pub document: Document,
    pub path: Option<PathBuf>,
    /// A new document, not a project: it takes the drawing options last used.
    pub fresh: bool,
}

impl Loaded {
    pub fn image(image: RgbaImage, path: Option<PathBuf>) -> Self {
        let document = Document::new(image.width(), image.height(), 1.0);
        Self {
            image,
            document,
            path,
            fresh: true,
        }
    }
}

/// Files larger than this are not opened.
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// Reads an image or a `.kirie` project.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let fail = |error: &dyn std::fmt::Display| format!("cannot open {}: {error}", path.display());
    let meta = std::fs::metadata(path).map_err(|error| fail(&error))?;
    if !meta.is_file() {
        return Err(fail(&"it is not a file"));
    }
    if meta.len() > MAX_FILE {
        return Err(fail(&"it is too large"));
    }
    let is_project = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(project::EXTENSION));
    if is_project {
        let file = std::fs::File::open(path).map_err(|error| fail(&error))?;
        let project = project::load(std::io::BufReader::new(file)).map_err(|error| fail(&error))?;
        return Ok(Loaded {
            image: project.image,
            document: project.document,
            path: Some(path.to_path_buf()),
            fresh: false,
        });
    }
    let image = ::image::open(path)
        .map_err(|error| fail(&error))?
        .to_rgba8();
    Ok(Loaded::image(image, Some(path.to_path_buf())))
}

impl Editor {
    /// Copying, saving, opening and pasting.
    pub(super) fn files(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Copy => self.export(Export::Copy),
            // The window is in front already, so the image opens in it as it is.
            Event::Open => Task::perform(crate::desktop::dialog::open_image(), |picked| picked)
                .then(|picked| match picked {
                    Ok(Some(path)) => Task::perform(load_async(path), Message::Opened),
                    Ok(None) => Task::none(),
                    Err(error) => Task::done(Message::File(Event::Done(Err(error)))),
                }),
            Event::Paste => Task::perform(
                paste(self.drops.device.clone(), self.clip.clone()),
                |value| Message::Marks(MarksEvent::Pasted(value)),
            ),
            Event::Save => self.export(Export::Save),
            Event::SaveProject => self.export(Export::Project),
            Event::SaveAs if self.image.is_some() => {
                let name = self.suggested_name();
                Task::perform(
                    async move {
                        let folder = crate::storage::save::screenshots_dir();
                        let _ = std::fs::create_dir_all(&folder);
                        crate::desktop::dialog::save_as(
                            &name,
                            &folder,
                            &crate::storage::save::Format::ALL,
                        )
                        .await
                    },
                    |picked| match picked {
                        Ok(Some((path, format))) => Message::File(Event::SaveTo(path, format)),
                        Ok(None) => Message::File(Event::Done(Ok(String::new()))),
                        Err(error) => Message::File(Event::Done(Err(error))),
                    },
                )
            }
            Event::SaveTo(path, format) => self.export(Export::To(path, format)),
            Event::Dropped(crate::desktop::drops::Dropped::Ready(device)) => {
                self.drops.device = Some(device);
                Task::none()
            }
            Event::Dropped(crate::desktop::drops::Dropped::Hovering(hovering)) => {
                self.drops.hovered = hovering;
                Task::none()
            }
            Event::Dropped(crate::desktop::drops::Dropped::Files(paths)) => {
                paths.into_iter().next().map_or_else(Task::none, |path| {
                    Task::perform(load_async(path), Message::Opened)
                })
            }
            Event::Dropped(crate::desktop::drops::Dropped::Png(bytes)) => {
                let decoded = ::image::load_from_memory(&bytes)
                    .map(|image| Loaded::image(image.to_rgba8(), None))
                    .map_err(|error| format!("cannot open the dropped image: {error}"));
                self.update(Message::Opened(decoded))
            }
            Event::SaveAs => Task::none(),
            Event::Done(Ok(text)) if text.is_empty() => Task::none(),
            Event::Done(result) => {
                if result.is_err() {
                    self.closing = Closing::No;
                    self.then = Then::Exit;
                }
                self.toast(result)
            }
            Event::Exported(text, file, kept) => {
                self.kept = *kept;
                if self.closing == Closing::Saving {
                    let then = self.carry_on();
                    return then.chain(self.toast_file(text, file));
                }
                self.toast_file(text, file)
            }
        }
    }

    /// The name the save dialog suggests: the open file's, else one with the time.
    fn suggested_name(&self) -> String {
        let stem = self
            .path
            .as_deref()
            .filter(|path| !crate::storage::save::is_handoff(path))
            .and_then(Path::file_stem)
            .map_or_else(
                || crate::storage::save::stem(&jiff::Zoned::now()),
                |stem| stem.to_string_lossy().into_owned(),
            );
        format!("{stem}.png")
    }

    pub(super) fn export(&mut self, export: Export) -> Task<Message> {
        self.keep_crop();
        self.annotator.commit_text();
        let Some((image, _)) = &self.image else {
            return Task::none();
        };
        let kept = Box::new(self.document().clone());
        let exported = move |done: Result<(String, Option<PathBuf>), String>| match done {
            Ok((text, file)) => Message::File(Event::Exported(text, file, kept)),
            Err(error) => Message::File(Event::Done(Err(error))),
        };
        let saved = |path: PathBuf| {
            (
                format!("Saved {}", crate::storage::save::pretty(&path)),
                Some(path),
            )
        };
        let project = || Project {
            image: image.clone(),
            document: self.document().clone(),
        };
        match export {
            Export::Project => {
                return Task::perform(crate::storage::save::save_project(project()), move |done| {
                    exported(done.map(saved))
                });
            }
            Export::To(path, format) => {
                return Task::perform(
                    crate::storage::save::save_as(project(), path, format),
                    move |done| exported(done.map(saved)),
                );
            }
            Export::Copy | Export::Save => {}
        }
        let rendered = kirie_annotate::render::render(
            self.document(),
            image,
            &mut self.typesetter.borrow_mut(),
        );
        let picture = match rendered {
            Ok(picture) => picture,
            Err(error) => {
                self.closing = Closing::No;
                return self.toast(Err(error.to_string()));
            }
        };
        match export {
            Export::Copy => {
                let copied =
                    crate::desktop::clipboard::copy_from(picture, self.drops.device.clone());
                Task::perform(copied, move |copied| {
                    exported(copied.map(|()| ("Copied to the clipboard".to_owned(), None)))
                })
            }
            Export::Save | Export::Project | Export::To(..) => {
                let saved = crate::storage::save::save_and_copy(picture, self.drops.device.clone());
                Task::perform(saved, move |done| {
                    exported(done.map(|(path, copied)| {
                        let text = if copied {
                            format!("Saved {} and copied", crate::storage::save::pretty(&path))
                        } else {
                            format!("Saved {}", crate::storage::save::pretty(&path))
                        };
                        (text, Some(path))
                    }))
                })
            }
        }
    }
}

/// What Ctrl+V pastes: `clip`, marks still on their way to the clipboard; else marks or an
/// image on the clipboard, read through the window's data device, else through the clipboard
/// protocol of wlroots compositors; else `clip`, marks that never got there.
async fn paste(
    device: Option<crate::desktop::drops::Device>,
    clip: Option<(Vec<Shape>, Offer)>,
) -> Result<Pasted, String> {
    const KINDS: &[&str] = &[kirie_annotate::marks::MIME, crate::desktop::drops::PNG];
    let unsent = match clip {
        Some((marks, Offer::Pending)) => return Ok(Pasted::Marks(marks)),
        Some((marks, Offer::Failed)) => Some(marks),
        None => None,
    };
    let found = match crate::desktop::drops::clipboard_kind(device, KINDS).await {
        Some(found) => Ok(found),
        None => crate::desktop::clipboard::paste(KINDS).await,
    };
    let (kind, bytes) = match found {
        Ok(found) => found,
        Err(error) => return unsent.map(Pasted::Marks).ok_or(error),
    };
    tokio::task::spawn_blocking(move || {
        if kind == kirie_annotate::marks::MIME {
            return kirie_annotate::marks::decode(&bytes).map(Pasted::Marks);
        }
        ::image::load_from_memory_with_format(&bytes, ::image::ImageFormat::Png)
            .map(|image| Pasted::Image(image.to_rgba8()))
            .map_err(|error| format!("the clipboard image cannot be read: {error}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(super) async fn load_async(path: PathBuf) -> Result<Loaded, String> {
    tokio::task::spawn_blocking(move || load(&path))
        .await
        .map_err(|error| error.to_string())?
}

/// Opens what is dropped on the editor's windows, from when window `id` is open.
pub(super) fn take_drops(id: window::Id) -> Task<Message> {
    window::run(id, crate::desktop::drops::display).then(|display| {
        display.map_or_else(Task::none, |display| {
            Task::run(crate::desktop::drops::drops(display), |value| {
                Message::File(Event::Dropped(value))
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_open_fresh_and_folders_do_not_open() {
        let dir = std::env::temp_dir().join(format!("kirie-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        RgbaImage::new(3, 2).save(&path).unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.fresh);
        assert_eq!((loaded.document.width, loaded.document.height), (3, 2));
        assert_eq!(loaded.path.as_deref(), Some(path.as_path()));
        let error = load(&dir).unwrap_err();
        assert!(error.ends_with("it is not a file"), "{error}");
        assert!(load(&dir.join("missing.png")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
