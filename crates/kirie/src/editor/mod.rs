//! App mode: a window to mark up any image or screenshot, keep it editable, and export it.

mod about;
mod canvas;
mod closing;
mod demo;
mod drop_zone;
mod frame;
mod mark_list;
mod release_card;
mod settings;
mod view;
mod viewport;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ::image::RgbaImage;
use iced::widget::image;
use iced::{Event, Point, Size, Subscription, Task, Vector, event, keyboard, window};
use kirie_annotate::project::{self, Project};
use kirie_annotate::{
    Document, Draft, History, Kind, Op, Paint, Press, Rgba, Shape, ShapeId, StyleId, Tool,
    Typesetter, Weight,
};
use kirie_ui::{Palette, Scheme};

use self::closing::{Answer, Closing, Then};
use self::viewport::{Viewport, Zoom};
use crate::controls;
use crate::drawing::Drawing;
use crate::emoji_picker::EmojiPicker;
use crate::picker::{self, Picker, Target};
use crate::selection::{Release, Selection};
use crate::swatches::Swatches;
use crate::typing::{self, Typing};

/// How close to a mark a click picks it, in screen pixels.
const MARK_REACH: f64 = 6.0;
/// How close to a crop handle a press grabs it, in screen pixels.
const HANDLE_REACH: f32 = 8.0;

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

/// The image pixel `at` falls in.
#[allow(clippy::cast_possible_truncation)]
fn pixel(at: kurbo::Point) -> (i64, i64) {
    (at.x.floor() as i64, at.y.floor() as i64)
}

#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Pressed(kurbo::Point),
    Moved(kurbo::Point),
    Released(kurbo::Point),
}

#[derive(Debug, Clone)]
pub enum Message {
    /// A pointer event in image coordinates, with the current canvas pixels per image pixel.
    Pointer(Pointer, f32),
    Pan(Vector),
    Zoom {
        factor: f32,
        anchor: Point,
        canvas: Size,
    },
    ZoomFit,
    ZoomActual,
    /// Zooms to the next level in, or out, around the middle of the canvas.
    ZoomIn,
    ZoomOut,
    ZoomTo(Zoom),
    /// Starts cropping, or keeps the crop when already cropping.
    Crop,
    CropDone,
    /// Keeps the whole image.
    CropReset,
    CropCancel,
    /// The image point under the pointer, if it is over the canvas.
    Hover(Option<kurbo::Point>),
    /// The canvas has a new size.
    Canvas(Size),
    Modifiers(keyboard::Modifiers),
    PickTool(Tool),
    PickStyle(StyleId),
    ToggleShadow,
    PickColor(Rgba),
    /// Opens the color picker on a swatch, or to add one.
    EditColor(Target),
    Picker(picker::Event),
    Weight(crate::controls::WeightEvent),
    ToggleFill,
    Brightness(controls::BrightnessEvent),
    Spot(kirie_annotate::SpotShape),
    Look(controls::TextEvent),
    Emoji(crate::emoji_picker::Event),
    /// Moves through or picks the emoji offered while typing.
    Choose(crate::typing::Choice),
    Frame(frame::Event),
    MarkList(mark_list::Event),
    ToggleScheme,
    TextEdit(iced::widget::text_editor::Action),
    TextDone,
    /// Pastes the clipboard's text into the text being typed.
    PasteText,
    /// The window lost the keyboard.
    Unfocused,
    Undo,
    Redo,
    Delete,
    /// Picks every mark with the select tool.
    SelectAll,
    /// Ctrl+C: copies the picked marks, else the picture.
    CopyKey,
    /// Copies the picked marks and deletes them.
    Cut,
    /// Copies the picked marks a step aside.
    Duplicate,
    /// Moves the picked marks up or down among the others.
    Restack(crate::marks::Restack),
    /// Copied marks reached the clipboard, or did not; with what to tell.
    Offered(Result<String, String>),
    /// What was on the clipboard when Ctrl+V was pressed.
    Pasted(Result<Pasted, String>),
    Escape,
    Copy,
    Open,
    Paste,
    Save,
    SaveProject,
    /// Asks where to save, and as what.
    SaveAs,
    SaveTo(PathBuf, crate::save::Format),
    Done(Result<String, String>),
    /// The image left the editor, saved (to the file) or copied, as this document had it.
    Exported(String, Option<PathBuf>, Box<Document>),
    /// Shows a saved file in the file manager.
    ShowInFiles(PathBuf),
    CloseRequested,
    Close(Answer),
    /// Puts the image away, asking first about unsaved changes.
    Discard,
    /// Opens or closes the about card.
    About(bool),
    Settings(settings::Event),
    /// Looking for a newer release, and installing it.
    Release(release_card::Event),
    OpenLink(&'static str),
    Opened(Result<Loaded, String>),
    Dropped(crate::drops::Dropped),
    ToastExpired(u64),
    Ipc(crate::ipc::Request),
    /// The desktop switched between light and dark.
    SystemScheme(Scheme),
    /// Hides the window and opens the overlay; the capture comes back over D-Bus.
    Capture,
    /// The window's size and state, taken just before it closes for a capture.
    Hide(Placement),
    /// Window `id`, placed so, is to be opened anew to come to the front.
    Remap(window::Id, Placement),
    CaptureEnded(Result<(), String>),
}

/// How far copied marks got to the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offer {
    Pending,
    Failed,
}

/// What Ctrl+V found on the clipboard.
#[derive(Debug, Clone)]
pub enum Pasted {
    Marks(Vec<Shape>),
    Image(RgbaImage),
}

/// How big the window is, to open it again the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    size: Size,
    maximized: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            size: Size::new(1280.0, 840.0),
            maximized: false,
        }
    }
}

/// The crop being drawn, in image coordinates.
#[derive(Debug, Clone, Copy)]
struct Cropping {
    selection: Selection,
    /// The crop before the last press, kept when that press turns out to be a click.
    before: Selection,
}

/// The window's data device, for drops and the clipboard.
#[derive(Debug, Default)]
struct Drops {
    /// The device, once it is ready.
    device: Option<crate::drops::Device>,
    /// The window the device was last made anew for.
    renewed_for: Option<window::Id>,
    /// Something that can be opened is dragged over the window.
    hovered: bool,
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
    /// The file just saved, to show in the file manager.
    file: Option<PathBuf>,
}

pub struct Editor {
    /// The window, unless it is closed while a screenshot is taken.
    window: Option<window::Id>,
    placement: Placement,
    palette: Palette,
    image: Option<(RgbaImage, image::Handle)>,
    path: Option<PathBuf>,
    history: History,
    /// The document as last opened, saved or copied, to tell unsaved changes.
    kept: Document,
    closing: Closing,
    /// The about card is open.
    about: bool,
    /// The settings card, while it is open.
    settings: Option<settings::Form>,
    /// Looking for newer releases, and what was found.
    updates: release_card::Updates,
    /// What to do once the question about unsaved changes is answered.
    then: Then,
    viewport: Viewport,
    cropping: Option<Cropping>,
    /// The canvas's size, for the zoom level when the image is fitted.
    canvas: Size,
    /// The image point under the pointer.
    hover: Option<kurbo::Point>,
    tool: Tool,
    paint: Paint,
    swatches: Swatches,
    picker: Option<Picker>,
    emojis: EmojiPicker,
    draft: Option<Draft>,
    /// The marks the select tool picked; a lone one shows its grips.
    picked: Vec<ShapeId>,
    mark_list: mark_list::Shown,
    /// The marks copied last, while the clipboard is not known to hold them: Ctrl+V pastes them
    /// while they are on their way there, or, should they never get there, when it holds
    /// neither marks nor an image.
    clip: Option<(Vec<Shape>, Offer)>,
    /// A slider is being dragged; its moves undo as one step.
    sliding: bool,
    typing: Option<Typing>,
    /// The last press, to tell a double click.
    last_click: Option<iced::advanced::mouse::Click>,
    modifiers: keyboard::Modifiers,
    toast: Option<Toast>,
    toasts: u64,
    /// Set once the user picks light or dark; until then the editor follows the desktop.
    scheme_chosen: bool,
    typesetter: RefCell<Typesetter>,
    patches: RefCell<HashMap<crate::paint::PatchKey, image::Handle>>,
    pictures: RefCell<HashMap<u64, image::Handle>>,
    drops: Drops,
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("path", &self.path)
            .field("tool", &self.tool)
            .finish_non_exhaustive()
    }
}

impl Editor {
    #[cfg(test)]
    pub fn boot(loaded: Option<Loaded>) -> (Self, Task<Message>) {
        Self::boot_with(loaded, None)
    }

    /// Like [`Self::boot`], bringing the window to the front with an activation `token`.
    fn boot_with(loaded: Option<Loaded>, token: Option<String>) -> (Self, Task<Message>) {
        let swatches = Swatches::load();
        let drawing = Drawing::load();
        let blank = drawing.styled(Document::new(1, 1, 1.0));
        let placement = Placement::default();
        let (id, opened) = window::open(settings(placement));
        let mut editor = Self {
            window: Some(id),
            placement,
            palette: Palette::DARK,
            image: None,
            path: None,
            history: History::new(blank.clone()),
            kept: blank,
            closing: Closing::No,
            about: false,
            settings: None,
            updates: release_card::Updates::new(),
            then: Then::Exit,
            viewport: Viewport::fitted(),
            cropping: None,
            canvas: Size::ZERO,
            hover: None,
            tool: Tool::Arrow,
            paint: drawing.paint_for(Some(Tool::Arrow), &swatches),
            swatches,
            picker: None,
            emojis: EmojiPicker::load(),
            draft: None,
            picked: Vec::new(),
            mark_list: mark_list::Shown::Closed,
            clip: None,
            sliding: false,
            typing: None,
            last_click: None,
            modifiers: keyboard::Modifiers::default(),
            toast: None,
            toasts: 0,
            scheme_chosen: false,
            typesetter: RefCell::new(Typesetter::new()),
            patches: RefCell::new(HashMap::new()),
            pictures: RefCell::new(HashMap::new()),
            drops: Drops::default(),
        };
        if let Some(loaded) = loaded {
            editor.open(loaded);
        }
        let opened = opened.then(move |id| {
            let raised = token
                .clone()
                .map_or_else(Task::none, |token| activate(id, token).discard());
            Task::batch([raised, take_drops(id)])
        });
        (editor, opened)
    }

    fn open(&mut self, mut loaded: Loaded) {
        let handoff = loaded
            .path
            .take_if(|path| crate::save::is_handoff(path))
            .map(std::fs::remove_file)
            .is_some();
        let handle = image::Handle::from_rgba(
            loaded.image.width(),
            loaded.image.height(),
            loaded.image.as_raw().clone(),
        );
        // The overlay may have changed the options since, in a process of its own.
        let drawing = Drawing::load();
        let demo = (loaded.fresh || handoff) && demo::wanted();
        let document = match (loaded.fresh, handoff) {
            (true, _) => drawing.framed(drawing.styled(loaded.document)),
            (false, true) => drawing.framed(loaded.document),
            (false, false) => loaded.document,
        };
        let document = if demo {
            demo::marked(document)
        } else {
            document
        };
        self.paint = drawing.paint_for(Some(self.tool), &self.swatches);
        self.kept = document.clone();
        self.history = History::new(document);
        self.follow_style(drawing.style);
        self.image = Some((loaded.image, handle));
        self.path = loaded.path;
        self.viewport = Viewport::fitted();
        self.cropping = None;
        self.draft = None;
        self.picked.clear();
        if demo {
            self.tool = Tool::Select;
            self.picked = demo::picked(self.document()).into_iter().collect();
        }
        self.typing = None;
        self.sliding = false;
        self.closing = Closing::No;
        self.then = Then::Exit;
        self.patches.borrow_mut().clear();
    }

    /// Puts the image away: the editor shows its empty page again.
    fn clear(&mut self) {
        let blank = Drawing::load().styled(Document::new(1, 1, 1.0));
        self.kept = blank.clone();
        self.history = History::new(blank);
        self.image = None;
        self.path = None;
        self.viewport = Viewport::fitted();
        self.cropping = None;
        self.hover = None;
        self.draft = None;
        self.picked.clear();
        self.typing = None;
        self.sliding = false;
        self.closing = Closing::No;
        self.then = Then::Exit;
        self.patches.borrow_mut().clear();
    }

    fn document(&self) -> &Document {
        self.history.current()
    }

    fn image_size(&self) -> Size {
        self.image.as_ref().map_or(Size::ZERO, |(image, _)| {
            #[allow(clippy::cast_precision_loss)]
            Size::new(image.width() as f32, image.height() as f32)
        })
    }

    /// What the canvas shows, in image coordinates: all of the image while cropping, else
    /// what gets saved, the kept part in its frame.
    fn content(&self) -> kurbo::Rect {
        if self.cropping.is_some() {
            let size = self.image_size();
            kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height))
        } else {
            self.document().canvas()
        }
    }

    /// What the canvas draws on: like [`Self::content`], but grown as marks being drawn reach
    /// beyond it, before they are done and the view settles on them.
    fn shown_canvas(&self) -> kurbo::Rect {
        if self.cropping.is_some() {
            self.content()
        } else {
            self.document().canvas_with(&self.drafts())
        }
    }

    /// The marks being drawn, moved, reshaped or typed, as they will be once done.
    fn drafts(&self) -> Vec<Shape> {
        let document = self.document();
        let mut drafts = Vec::new();
        match &self.draft {
            Some(Draft::Moving { ids, by, .. }) => {
                drafts.extend(
                    ids.iter()
                        .filter_map(|id| document.shape(*id))
                        .map(|shape| Shape {
                            kind: shape.kind.translated(*by),
                            ..shape.clone()
                        }),
                );
            }
            Some(draft) => {
                drafts.extend(draft.edited().cloned());
                drafts.extend(draft.preview().map(|kind| Shape {
                    id: document.next_id(),
                    kind,
                    paint: self.paint,
                    style: None,
                    rotation: 0.0,
                }));
            }
            None => {}
        }
        drafts.extend(
            self.typing
                .as_ref()
                .map(|typing| typing.preview(document, self.paint)),
        );
        drafts
    }

    /// The crop being drawn, while cropping.
    fn crop(&self) -> Option<iced::Rectangle> {
        self.cropping.and_then(|cropping| cropping.selection.rect())
    }

    pub fn title(&self, _: window::Id) -> String {
        match self.path.as_deref().and_then(Path::file_name) {
            Some(name) => format!("{} — Kirie", name.to_string_lossy()),
            None => "Kirie".to_owned(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(pointer, scale) => return self.pointer(pointer, scale),
            Message::Pan(by) => self.viewport = self.viewport.panned(by),
            Message::Zoom { .. }
            | Message::ZoomFit
            | Message::ZoomActual
            | Message::ZoomIn
            | Message::ZoomOut
            | Message::ZoomTo(_) => self.zoom(&message),
            Message::Hover(at) => self.hover = at,
            Message::Canvas(size) => self.canvas = size,
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::PickTool(tool) => self.pick_tool(tool),
            Message::PickStyle(_)
            | Message::ToggleShadow
            | Message::PickColor(_)
            | Message::EditColor(_)
            | Message::Picker(_)
            | Message::Weight(_)
            | Message::ToggleFill
            | Message::Brightness(_)
            | Message::Spot(_)
            | Message::Look(_)
            | Message::Frame(_) => return self.restyle(message),
            Message::SystemScheme(scheme) => self.follow(Some(scheme)),
            Message::ToggleScheme => self.follow(None),
            Message::TextEdit(_)
            | Message::TextDone
            | Message::PasteText
            | Message::Choose(_)
            | Message::Emoji(_) => {
                return self.typed(message);
            }
            Message::Undo => self.travel(true),
            Message::Redo => self.travel(false),
            Message::Delete
            | Message::SelectAll
            | Message::CopyKey
            | Message::Cut
            | Message::Duplicate
            | Message::Restack(_)
            | Message::Offered(_)
            | Message::Pasted(_)
            | Message::MarkList(_) => return self.edit_marks(message),
            Message::Escape if self.closing == Closing::Asking => self.closing = Closing::No,
            Message::Escape | Message::About(false) if self.about => self.about = false,
            Message::Settings(event) => return self.edit_settings(event),
            Message::Release(event) => return self.release_event(event),
            Message::About(_) | Message::OpenLink(_) | Message::ShowInFiles(_) => {
                return self.outside(message);
            }
            Message::CropCancel if self.cropping.is_some() => {
                self.cropping = None;
                self.viewport = Viewport::fitted();
            }
            Message::Crop | Message::CropDone | Message::CropReset | Message::CropCancel => {
                self.crop_message(&message);
            }
            Message::Escape => return self.escape(),
            Message::Copy
            | Message::Open
            | Message::Paste
            | Message::Save
            | Message::SaveProject
            | Message::SaveAs
            | Message::SaveTo(..)
            | Message::Unfocused
            | Message::Dropped(_) => return self.files(message),
            Message::Done(Ok(text)) if text.is_empty() => {}
            Message::Done(_)
            | Message::Exported(..)
            | Message::CloseRequested
            | Message::Discard
            | Message::Close(_) => return self.finish(message),
            Message::Opened(Ok(loaded)) => return self.replace(loaded),
            Message::Ipc(_)
            | Message::Capture
            | Message::Remap(..)
            | Message::Hide(_)
            | Message::CaptureEnded(_) => return self.windowing(message),
            Message::Opened(Err(error)) => return self.toast(Err(error)),
            Message::ToastExpired(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
        }
        Task::none()
    }

    fn pick_tool(&mut self, tool: Tool) {
        self.keep_crop();
        self.commit_text();
        self.tool = tool;
        self.paint.color = controls::tool_color(
            Some(tool),
            self.paint.color,
            self.swatches.first(self.document().style),
        );
        self.draft = None;
        if tool != Tool::Select {
            self.picked.clear();
        }
    }

    /// Esc puts down what is open, one thing at a time: the eyedropper, the settings, the crop,
    /// the color picker, the text being typed, the mark being drawn, then the picked marks.
    fn escape(&mut self) -> Task<Message> {
        if self.sampling() {
            return self.edit_color(picker::Event::Eyedropper);
        }
        if self.settings.take().is_some() {
            return Task::none();
        }
        if self.cropping.take().is_some() {
            self.viewport = Viewport::fitted();
            return Task::none();
        }
        if let Some(typing) = &mut self.typing
            && typing.dismiss()
        {
            return iced::widget::operation::focus(view::TEXT_INPUT);
        }
        if self.picker.take().is_none()
            && self.typing.take().is_none()
            && self.draft.take().is_none()
        {
            self.picked.clear();
        }
        Task::none()
    }

    /// Deleting, picking, copying and pasting marks, and the list of them.
    fn edit_marks(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::MarkList(event) => return self.mark_list_event(event),
            Message::CopyKey if self.picked.is_empty() => return self.export(Export::Copy),
            Message::CopyKey => return self.copy_marks(),
            Message::Cut if !self.picked.is_empty() => {
                let copied = self.copy_marks();
                let ids = std::mem::take(&mut self.picked);
                self.history.apply(Op::Remove { ids });
                return copied;
            }
            Message::Duplicate => {
                if let Some((op, ids)) = crate::marks::duplicate(self.document(), &self.picked) {
                    self.history.apply(op);
                    self.picked = ids;
                }
            }
            Message::Restack(to) => {
                if let Some(op) = crate::marks::restack(self.document(), &self.picked, to) {
                    self.history.apply(op);
                }
            }
            Message::Pasted(Ok(Pasted::Marks(_))) if self.image.is_none() => {
                return self.toast(Err("Open an image to paste the marks on".to_owned()));
            }
            Message::Pasted(Ok(Pasted::Marks(clip))) => {
                self.keep_crop();
                let _ = self.update(Message::PickTool(Tool::Select));
                if let Some((op, ids)) = crate::marks::paste(self.document(), &clip) {
                    self.history.apply(op);
                    self.picked = ids;
                }
            }
            Message::Pasted(Ok(Pasted::Image(image))) if self.image.is_none() => {
                return self.update(Message::Opened(Ok(Loaded::image(image, None))));
            }
            Message::Pasted(Ok(Pasted::Image(image))) => {
                self.keep_crop();
                match crate::marks::place_image(self.document(), image) {
                    Ok((op, id)) => {
                        let _ = self.update(Message::PickTool(Tool::Select));
                        self.history.apply(op);
                        self.picked = vec![id];
                    }
                    Err(error) => return self.toast(Err(error)),
                }
            }
            Message::Pasted(Err(error)) => return self.toast(Err(error)),
            Message::Offered(Ok(copied)) => {
                self.clip = None;
                return self.toast(Ok(copied));
            }
            Message::Offered(Err(copied)) => {
                if let Some((_, offer)) = &mut self.clip {
                    *offer = Offer::Failed;
                }
                return self.toast(Ok(copied));
            }
            Message::Delete if !self.picked.is_empty() => {
                let ids = std::mem::take(&mut self.picked);
                self.history.apply(Op::Remove { ids });
            }
            Message::SelectAll if self.image.is_some() && self.cropping.is_none() => {
                let _ = self.update(Message::PickTool(Tool::Select));
                let document = self.document();
                let canvas = document.canvas();
                self.picked = document
                    .shapes()
                    .filter(|shape| {
                        let area = shape.turn().transform_rect_bbox(document.bounds(shape));
                        area.x0 <= canvas.x1
                            && canvas.x0 <= area.x1
                            && area.y0 <= canvas.y1
                            && canvas.y0 <= area.y1
                    })
                    .map(|shape| shape.id)
                    .collect();
            }
            _ => {}
        }
        Task::none()
    }

    /// Puts the picked marks on the clipboard, for Ctrl+V here or in another Kirie.
    fn copy_marks(&mut self) -> Task<Message> {
        let marks = crate::marks::of(self.document(), &self.picked);
        if marks.is_empty() {
            return Task::none();
        }
        self.clip = Some((marks.clone(), Offer::Pending));
        let copied = if marks.len() == 1 {
            "Copied the mark; Ctrl+V pastes it".to_owned()
        } else {
            format!("Copied {} marks; Ctrl+V pastes them", marks.len())
        };
        let offered = async move {
            let bytes = tokio::task::spawn_blocking(move || crate::marks::encode(marks))
                .await
                .map_err(|error| error.to_string())??;
            crate::clipboard::offer(bytes, crate::marks::MIME).await
        };
        Task::perform(offered, move |offered| {
            Message::Offered(match offered {
                Ok(()) => Ok(copied),
                Err(error) => Err(format!("{copied} here; other apps cannot ({error})")),
            })
        })
    }

    fn zoom(&mut self, message: &Message) {
        let content = self.content();
        let middle = Point::new(self.canvas.width / 2.0, self.canvas.height / 2.0);
        let scale = self.viewport.scale(content, self.canvas);
        self.viewport = match *message {
            Message::Zoom {
                factor,
                anchor,
                canvas,
            } => self.viewport.zoomed(factor, anchor, content, canvas),
            Message::ZoomFit | Message::ZoomTo(Zoom::Fit) => Viewport::fitted(),
            Message::ZoomActual => Viewport::actual_size(),
            Message::ZoomIn | Message::ZoomOut => {
                let level = viewport::step(scale, matches!(message, Message::ZoomIn));
                self.viewport.zoomed_to(level, middle, content, self.canvas)
            }
            #[allow(clippy::cast_precision_loss)]
            Message::ZoomTo(Zoom::Percent(percent)) => {
                self.viewport
                    .zoomed_to(percent as f32 / 100.0, middle, content, self.canvas)
            }
            _ => self.viewport,
        };
    }

    /// Taking a screenshot from the editor, and bringing its window to the front.
    fn windowing(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Ipc(crate::ipc::Request::Open(path, token)) => self
                .raise(token)
                .chain(Task::perform(load_async(path), Message::Opened)),
            Message::Ipc(crate::ipc::Request::Present(token)) => self.raise(token),
            Message::Ipc(crate::ipc::Request::About(token)) => {
                self.about = true;
                Task::batch([self.raise(token), self.check_on_about()])
            }
            Message::Ipc(crate::ipc::Request::Settings(token)) => {
                let _ = self.edit_settings(settings::Event::Open);
                self.raise(token)
            }
            Message::Ipc(crate::ipc::Request::Resume) if self.window.is_none() => self.raise(None),
            Message::Capture => self
                .window
                .map_or_else(Task::none, |id| placement(id).map(Message::Hide)),
            Message::Remap(id, placement) if self.window == Some(id) => {
                self.placement = placement;
                self.reopen(Some(id), None)
            }
            Message::Hide(placement) => {
                let Some(id) = self.window.take() else {
                    return Task::none();
                };
                self.placement = placement;
                window::close(id).chain(Task::perform(capture(), Message::CaptureEnded))
            }
            Message::CaptureEnded(Err(error)) => self.raise(None).chain(self.toast(Err(error))),
            // The overlay asks for the window back when it is done, through `Resume` or `Open`.
            _ => Task::none(),
        }
    }

    /// What became of an export, and closing the window.
    fn finish(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Done(result) => {
                if result.is_err() {
                    self.closing = Closing::No;
                    self.then = Then::Exit;
                }
                self.toast(result)
            }
            Message::Exported(text, file, kept) => {
                self.kept = *kept;
                if self.closing == Closing::Saving {
                    let then = self.carry_on();
                    return then.chain(self.toast_file(text, file));
                }
                self.toast_file(text, file)
            }
            Message::CloseRequested => self.close_requested(),
            Message::Discard => self.discard(),
            Message::Close(answer) => self.answer(answer),
            _ => Task::none(),
        }
    }

    /// Changes to the style, colors, weight, fill, text look and frame, kept for the next time.
    fn restyle(&mut self, message: Message) -> Task<Message> {
        let settled = settled(&message);
        let task = self.change_style(message);
        if settled {
            self.remember_drawing();
        }
        task
    }

    fn change_style(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PickStyle(style) => {
                let before = self.document().style;
                let recolor = self.swatches.recolor(before, style);
                self.history.apply(Op::SetStyle { style, recolor });
                self.follow_style(before);
            }
            Message::ToggleShadow => {
                let shadow = !self.document().shadow;
                self.history.apply(Op::SetShadow(shadow));
            }
            Message::PickColor(color) => self.repaint(|paint| paint.color = color),
            Message::EditColor(target) => {
                self.picker = Some(Picker::new(
                    target,
                    self.document().style,
                    &self.swatches,
                    self.paint.color,
                ));
            }
            Message::Picker(event) => return self.edit_color(event),
            Message::Weight(controls::WeightEvent::Done)
            | Message::Brightness(controls::BrightnessEvent::Done)
            | Message::Look(controls::TextEvent::SizeDone)
            | Message::Frame(frame::Event::MarginDone) => self.sliding = false,
            Message::Frame(event) => {
                let document = self.document();
                let op = Op::SetFrame(event.apply(document.frame, document.unit));
                if let frame::Event::Margin(_) = event {
                    self.history.apply_gradually(op, self.sliding);
                    self.sliding = true;
                } else {
                    self.history.apply(op);
                }
            }
            Message::Brightness(controls::BrightnessEvent::Slide(brightness)) => {
                self.history
                    .apply_gradually(Op::SetSpotlightBrightness(brightness), self.sliding);
                self.sliding = true;
            }
            Message::Weight(controls::WeightEvent::Pick(weight)) => {
                self.repaint(|paint| paint.weight = weight);
            }
            Message::Weight(controls::WeightEvent::Slide(width)) => {
                self.repaint_gradually(|paint| paint.weight = Weight::new(width), true);
            }
            Message::ToggleFill => self.repaint(|paint| paint.filled = !paint.filled),
            Message::Spot(spot) => self.repaint(|paint| paint.spot = spot),
            Message::Look(event) => {
                self.repaint_gradually(
                    |paint| paint.text = event.apply(paint.text),
                    event.gradual(),
                );
            }
            _ => {}
        }
        Task::none()
    }

    /// Keeps the drawing options for the next image, and the next session.
    fn remember_drawing(&self) {
        let drawing = Drawing::load().with_marks(self.paint, self.document());
        if self.image.is_some() {
            drawing.with_frame(self.document()).save();
        } else {
            drawing.save();
        }
    }

    fn edit_color(&mut self, event: picker::Event) -> Task<Message> {
        let picker::Outcome::Changed { pick } =
            picker::update(&mut self.picker, &mut self.swatches, event)
        else {
            return Task::none();
        };
        if let Some(color) = pick {
            self.repaint(|paint| paint.color = color);
        }
        match self.swatches.save() {
            Ok(()) => Task::none(),
            Err(error) => self.toast(Err(error)),
        }
    }

    /// Copying, saving, opening and pasting.
    fn files(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Copy => self.export(Export::Copy),
            // The window is in front already, so the image opens in it as it is.
            Message::Open => {
                Task::perform(crate::dialog::open_image(), |picked| picked).then(|picked| {
                    match picked {
                        Ok(Some(path)) => Task::perform(load_async(path), Message::Opened),
                        Ok(None) => Task::none(),
                        Err(error) => Task::done(Message::Done(Err(error))),
                    }
                })
            }
            Message::Paste => Task::perform(
                paste(self.drops.device.clone(), self.clip.clone()),
                Message::Pasted,
            ),
            Message::Save => self.export(Export::Save),
            Message::SaveProject => self.export(Export::Project),
            Message::SaveAs if self.image.is_some() => {
                let name = self.suggested_name();
                Task::perform(
                    async move {
                        let folder = crate::save::screenshots_dir();
                        let _ = std::fs::create_dir_all(&folder);
                        crate::dialog::save_as(&name, &folder, &crate::save::Format::ALL).await
                    },
                    |picked| match picked {
                        Ok(Some((path, format))) => Message::SaveTo(path, format),
                        Ok(None) => Message::Done(Ok(String::new())),
                        Err(error) => Message::Done(Err(error)),
                    },
                )
            }
            Message::SaveTo(path, format) => self.export(Export::To(path, format)),
            // Made anew before anything can be dragged from another window onto this one.
            // Once per window: iced's clipboard makes a device of its own when a window opens.
            Message::Unfocused => {
                if self.drops.renewed_for != self.window
                    && let Some(device) = &self.drops.device
                {
                    device.renew();
                    self.drops.renewed_for = self.window;
                }
                Task::none()
            }
            Message::Dropped(crate::drops::Dropped::Ready(device)) => {
                self.drops.device = Some(device);
                Task::none()
            }
            Message::Dropped(crate::drops::Dropped::Hovering(hovering)) => {
                self.drops.hovered = hovering;
                Task::none()
            }
            Message::Dropped(crate::drops::Dropped::Files(paths)) => {
                paths.into_iter().next().map_or_else(Task::none, |path| {
                    Task::perform(load_async(path), Message::Opened)
                })
            }
            Message::Dropped(crate::drops::Dropped::Png(bytes)) => {
                let decoded = ::image::load_from_memory(&bytes)
                    .map(|image| Loaded::image(image.to_rgba8(), None))
                    .map_err(|error| format!("cannot open the dropped image: {error}"));
                self.update(Message::Opened(decoded))
            }
            _ => Task::none(),
        }
    }

    /// Follows the desktop's `scheme` until the user switches it themselves (`None`).
    fn follow(&mut self, scheme: Option<Scheme>) {
        match scheme {
            Some(scheme) if !self.scheme_chosen => self.palette = Palette::of(scheme),
            Some(_) => {}
            None => {
                self.scheme_chosen = true;
                self.palette = match self.palette.scheme {
                    Scheme::Dark => Palette::LIGHT,
                    Scheme::Light => Palette::DARK,
                };
            }
        }
    }

    fn crop_message(&mut self, message: &Message) {
        match message {
            Message::Crop if self.cropping.is_some() => self.keep_crop(),
            Message::Crop if self.image.is_some() => {
                self.commit_text();
                self.draft = None;
                self.picked.clear();
                let selection = Selection::selected(to_rectangle(self.document().visible()));
                self.cropping = Some(Cropping {
                    selection,
                    before: selection,
                });
                self.viewport = Viewport::fitted();
            }
            Message::CropDone => self.keep_crop(),
            Message::CropReset => {
                let whole = iced::Rectangle::new(Point::ORIGIN, self.image_size());
                if let Some(cropping) = &mut self.cropping {
                    cropping.selection = Selection::selected(whole);
                }
            }
            _ => {}
        }
    }

    /// Ends cropping, keeping the crop drawn, in whole pixels; all of the image is no crop.
    fn keep_crop(&mut self) {
        let Some(cropping) = self.cropping.take() else {
            return;
        };
        self.viewport = Viewport::fitted();
        let Some(rect) = cropping.selection.rect() else {
            return;
        };
        let crop = kept(rect, self.image_size());
        if crop != self.document().crop {
            self.history.apply(Op::Crop(crop));
        }
    }

    /// Follows the pointer while cropping.
    fn crop_pointer(&mut self, pointer: Pointer, scale: f32) {
        let bounds = iced::Rectangle::new(Point::ORIGIN, self.image_size());
        let Some(cropping) = &mut self.cropping else {
            return;
        };
        let reach = HANDLE_REACH / scale.max(0.01);
        match pointer {
            Pointer::Pressed(at) => {
                cropping.before = cropping.selection;
                cropping.selection = cropping.selection.press(to_point(at), reach);
            }
            Pointer::Moved(at) => {
                cropping.selection = cropping.selection.drag_to(to_point(at), bounds);
            }
            Pointer::Released(at) => {
                let (selection, release) = cropping.selection.release(to_point(at));
                cropping.selection = match release {
                    Release::Click(_) => cropping.before,
                    _ if selection.rect().is_none() => cropping.before,
                    _ => selection,
                };
            }
        }
    }

    /// Undoes or redoes a step; the drawing color follows the style there.
    fn travel(&mut self, undo: bool) {
        self.cropping = None;
        let before = self.document().style;
        if undo {
            self.history.undo();
        } else {
            self.history.redo();
        }
        crate::marks::prune(self.history.current(), &mut self.picked);
        self.follow_style(before);
    }

    /// Keeps drawing in the standard color drawn with before the style changed from `before`,
    /// in the new style's shade of it.
    fn follow_style(&mut self, before: StyleId) {
        let now = self.document().style;
        if now != before {
            let recolor = self.swatches.recolor(before, now);
            self.paint.color = crate::controls::restyled(self.paint.color, &recolor);
        }
    }

    /// Changes the paint for new marks, and the picked mark's paint too.
    fn repaint(&mut self, change: impl Fn(&mut Paint)) {
        self.repaint_gradually(change, false);
    }

    /// Like [`Self::repaint`]; a `gradual` change joins the step the slider drag began.
    fn repaint_gradually(&mut self, change: impl Fn(&mut Paint), gradual: bool) {
        change(&mut self.paint);
        let Some(op) = crate::marks::repaint(self.document(), &self.picked, change) else {
            return;
        };
        if gradual {
            self.history.apply_gradually(op, self.sliding);
        } else {
            self.history.apply(op);
        }
        self.sliding = gradual;
    }

    /// The paint the controls show: the first picked mark's, else the one new marks get.
    fn shown_paint(&self) -> Paint {
        crate::marks::shown_paint(self.document(), &self.picked, self.paint)
    }

    /// Whether the text controls apply: to words being typed, picked text marks and bubbles, or
    /// the next.
    fn texting(&self) -> bool {
        matches!(self.tool, Tool::Text | Tool::Bubble)
            || self.typing.is_some()
            || crate::marks::all(self.document(), &self.picked, |kind| {
                matches!(kind, Kind::Text { .. } | Kind::Bubble { .. })
            })
    }

    /// Whether the emoji picker shows: for the sticker tool or picked stickers.
    fn stickering(&self) -> bool {
        crate::marks::stickering(Some(self.tool), self.document(), &self.picked)
    }

    /// Whether the spotlight controls apply: to the spotlight tool or picked spotlights.
    fn spotlighting(&self) -> bool {
        self.tool == Tool::Spotlight
            || crate::marks::all(self.document(), &self.picked, |kind| {
                matches!(kind, Kind::Spotlight { .. })
            })
    }

    /// Whether a press adds to the picked marks instead of picking anew.
    fn extending(&self) -> bool {
        self.modifiers.shift() || self.modifiers.command()
    }

    fn pointer(&mut self, pointer: Pointer, scale: f32) -> Task<Message> {
        if self.image.is_none() {
            return Task::none();
        }
        if let Pointer::Moved(at) = pointer {
            self.hover = Some(at);
        }
        if self.sampling() {
            return match pointer {
                Pointer::Pressed(at) => self.sample(at),
                _ => Task::none(),
            };
        }
        if self.cropping.is_some() {
            self.crop_pointer(pointer, scale);
            return Task::none();
        }
        match pointer {
            Pointer::Pressed(at) => {
                self.commit_text();
                let reach = MARK_REACH / f64::from(scale.max(0.01));
                #[allow(clippy::cast_possible_truncation)]
                let double = typing::double_click(
                    &mut self.last_click,
                    Point::new(at.x as f32, at.y as f32),
                );
                let words = match self.tool {
                    Tool::Text | Tool::Bubble => typing::words_at(self.document(), at, reach),
                    Tool::Select if double => typing::words_at(self.document(), at, reach),
                    _ => None,
                };
                if let Some(id) = words {
                    return self.edit_text(id);
                }
                let extend = self.extending();
                match kirie_annotate::tools::press(
                    self.tool,
                    at,
                    self.document(),
                    self.paint,
                    reach,
                    &self.picked,
                    extend,
                ) {
                    Press::Draft(draft) => {
                        crate::marks::pressed(&draft, &mut self.picked);
                        self.draft = Some(draft);
                    }
                    Press::Done(op) => {
                        self.history.apply(op);
                    }
                    Press::TypeAt(at) => {
                        self.typing = Some(Typing::new(at));
                        return iced::widget::operation::focus(view::TEXT_INPUT);
                    }
                    Press::PlaceSticker(at) => {
                        let size = kirie_annotate::tools::STICKER_SIZE * self.document().unit;
                        let draft = Draft::sticker(self.emojis.chosen(), at, size);
                        crate::marks::pressed(&draft, &mut self.picked);
                        self.draft = Some(draft);
                    }
                    Press::Toggle(id) => crate::marks::toggle(&mut self.picked, id),
                    Press::Nothing => self.picked.clear(),
                }
            }
            Pointer::Moved(at) => {
                if let Some(draft) = self.draft.take() {
                    self.draft = Some(draft.drag(at, self.modifiers.shift()));
                }
            }
            Pointer::Released(at) => {
                if let Some(draft) = self.draft.take() {
                    let draft = draft.drag(at, self.modifiers.shift());
                    if let Some((center, tail)) = draft.bubble() {
                        self.typing = Some(Typing::bubble(center, tail));
                        return iced::widget::operation::focus(view::TEXT_INPUT);
                    }
                    let document = self.history.current();
                    if let Some(op) =
                        crate::marks::release(draft, document, &mut self.picked, self.paint)
                    {
                        self.history.apply(op);
                    }
                }
            }
        }
        Task::none()
    }

    /// Whether the eyedropper waits for a click on the image.
    fn sampling(&self) -> bool {
        self.picker.as_ref().is_some_and(Picker::sampling)
    }

    /// Hands the picker the image's color at `at`, if `at` is on the image.
    fn sample(&mut self, at: kurbo::Point) -> Task<Message> {
        let (column, row) = pixel(at);
        let color = self.image.as_ref().and_then(|(image, _)| {
            let found =
                image.get_pixel_checked(u32::try_from(column).ok()?, u32::try_from(row).ok()?)?;
            let [red, green, blue, _] = found.0;
            Some(Rgba::rgb(red, green, blue))
        });
        match color {
            Some(color) => self.edit_color(picker::Event::Sampled(color)),
            None => Task::none(),
        }
    }

    /// The about card, and links and files handed to the desktop.
    fn outside(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::About(open) => {
                self.about = open;
                if open {
                    return self.check_on_about();
                }
            }
            Message::OpenLink(link) => crate::launch::open(link),
            Message::ShowInFiles(path) => {
                return Task::future(async move { crate::launch::show_in_files(&path).await })
                    .discard();
            }
            _ => {}
        }
        Task::none()
    }

    /// Typing in the text box.
    fn typed(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TextEdit(action) => {
                if let Some(typing) = &mut self.typing {
                    typing.act(action, self.emojis.tone());
                }
            }
            Message::Choose(choice) => {
                let refocus = self
                    .typing
                    .as_mut()
                    .is_some_and(|typing| typing.choose(choice, &mut self.emojis));
                if refocus {
                    return iced::widget::operation::focus(view::TEXT_INPUT);
                }
            }
            Message::TextDone => self.commit_text(),
            Message::PasteText => return self.paste_text(),
            Message::Emoji(event) => {
                if let Some(emoji) = self.emojis.update(event)
                    && let Some(op) = crate::marks::restick(self.document(), &self.picked, &emoji)
                {
                    self.history.apply(op);
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Pastes text from the clipboard, read through the window's data device, else through
    /// iced's clipboard.
    fn paste_text(&self) -> Task<Message> {
        let paste = |text: String| {
            Message::TextEdit(iced::widget::text_editor::Action::Edit(
                iced::widget::text_editor::Edit::Paste(std::sync::Arc::new(text)),
            ))
        };
        let nothing = || Message::Done(Ok(String::new()));
        Task::perform(
            crate::drops::clipboard(self.drops.device.clone(), &crate::drops::TEXT),
            |bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        )
        .then(move |text| match text {
            Some(text) => Task::done(paste(text)),
            None => iced::clipboard::read().map(move |text| text.map_or_else(nothing, paste)),
        })
    }

    fn commit_text(&mut self) {
        if let Some(op) = self
            .typing
            .take()
            .and_then(|typing| typing.finish(self.paint))
        {
            self.history.apply(op);
        }
    }

    /// Opens the words of a placed text mark for typing.
    fn edit_text(&mut self, id: ShapeId) -> Task<Message> {
        let Some(typing) = self.document().shape(id).and_then(Typing::edit) else {
            return Task::none();
        };
        self.typing = Some(typing);
        self.picked = vec![id];
        self.draft = None;
        iced::widget::operation::focus(view::TEXT_INPUT)
    }

    /// The name the save dialog suggests: the open file's, else one with the time.
    fn suggested_name(&self) -> String {
        let stem = self
            .path
            .as_deref()
            .filter(|path| !crate::save::is_handoff(path))
            .and_then(Path::file_stem)
            .map_or_else(
                || crate::save::stem(&jiff::Zoned::now()),
                |stem| stem.to_string_lossy().into_owned(),
            );
        format!("{stem}.png")
    }

    fn export(&mut self, export: Export) -> Task<Message> {
        self.keep_crop();
        self.commit_text();
        let Some((image, _)) = &self.image else {
            return Task::none();
        };
        let kept = Box::new(self.document().clone());
        let exported = move |done: Result<(String, Option<PathBuf>), String>| match done {
            Ok((text, file)) => Message::Exported(text, file, kept),
            Err(error) => Message::Done(Err(error)),
        };
        let saved = |path: PathBuf| (format!("Saved {}", crate::save::pretty(&path)), Some(path));
        let project = || Project {
            image: image.clone(),
            document: self.document().clone(),
        };
        match export {
            Export::Project => {
                return Task::perform(crate::save::save_project(project()), move |done| {
                    exported(done.map(saved))
                });
            }
            Export::To(path, format) => {
                return Task::perform(crate::save::save_as(project(), path, format), move |done| {
                    exported(done.map(saved))
                });
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
            Export::Copy => Task::perform(crate::clipboard::copy(picture), move |copied| {
                exported(copied.map(|()| ("Copied to the clipboard".to_owned(), None)))
            }),
            Export::Save | Export::Project | Export::To(..) => {
                Task::perform(crate::save::save_and_copy(picture), move |done| {
                    exported(done.map(|(path, copied)| {
                        let text = if copied {
                            format!("Saved {} and copied", crate::save::pretty(&path))
                        } else {
                            format!("Saved {}", crate::save::pretty(&path))
                        };
                        (text, Some(path))
                    }))
                })
            }
        }
    }

    fn toast(&mut self, result: Result<String, String>) -> Task<Message> {
        match result {
            Ok(text) => self.show_toast(text, false, None),
            Err(text) => self.show_toast(text, true, None),
        }
    }

    /// Tells that the image was saved to `file`, or copied without one.
    fn toast_file(&mut self, text: String, file: Option<PathBuf>) -> Task<Message> {
        self.show_toast(text, false, file)
    }

    fn show_toast(&mut self, text: String, error: bool, file: Option<PathBuf>) -> Task<Message> {
        self.toasts += 1;
        let id = self.toasts;
        // A toast with a button stays a little longer, to reach the button.
        let shown = Duration::from_secs(if file.is_some() { 8 } else { 4 });
        self.toast = Some(Toast {
            id,
            text,
            error,
            file,
        });
        Task::perform(tokio::time::sleep(shown), move |()| {
            Message::ToastExpired(id)
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let quiet = self.typing.is_some()
            || self.picker.is_some()
            || self.closing != Closing::No
            || self.about
            || self.settings.is_some();
        let keys = if quiet {
            event::listen_with(typing_key)
        } else if self.cropping.is_some() {
            event::listen_with(crop_key)
        } else {
            event::listen_with(key)
        };
        let releases = if self.updates.check() {
            iced::time::every(release_card::TICK)
                .map(|_| Message::Release(release_card::Event::Tick))
        } else {
            Subscription::none()
        };
        Subscription::batch([
            keys,
            releases,
            event::listen_with(focus),
            Subscription::run(crate::ipc::requests).map(Message::Ipc),
            Subscription::run(crate::appearance::changes).map(Message::SystemScheme),
            window::close_requests().map(|_| Message::CloseRequested),
        ])
    }

    pub fn theme(&self, _: window::Id) -> iced::Theme {
        match self.palette.scheme {
            Scheme::Dark => iced::Theme::Dark,
            Scheme::Light => iced::Theme::Light,
        }
    }
}

/// What Ctrl+V pastes: `clip`, marks still on their way to the clipboard; else marks or an
/// image on the clipboard, read through the window's data device, else through the clipboard
/// protocol of wlroots compositors; else `clip`, marks that never got there.
async fn paste(
    device: Option<crate::drops::Device>,
    clip: Option<(Vec<Shape>, Offer)>,
) -> Result<Pasted, String> {
    const KINDS: &[&str] = &[crate::marks::MIME, crate::drops::PNG];
    let unsent = match clip {
        Some((marks, Offer::Pending)) => return Ok(Pasted::Marks(marks)),
        Some((marks, Offer::Failed)) => Some(marks),
        None => None,
    };
    let found = match crate::drops::clipboard_kind(device, KINDS).await {
        Some(found) => Ok(found),
        None => crate::clipboard::paste(KINDS).await,
    };
    let (kind, bytes) = match found {
        Ok(found) => found,
        Err(error) => return unsent.map(Pasted::Marks).ok_or(error),
    };
    tokio::task::spawn_blocking(move || {
        if kind == crate::marks::MIME {
            return crate::marks::decode(&bytes).map(Pasted::Marks);
        }
        ::image::load_from_memory_with_format(&bytes, ::image::ImageFormat::Png)
            .map(|image| Pasted::Image(image.to_rgba8()))
            .map_err(|error| format!("the clipboard image cannot be read: {error}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn load_async(path: PathBuf) -> Result<Loaded, String> {
    tokio::task::spawn_blocking(move || load(&path))
        .await
        .map_err(|error| error.to_string())?
}

/// Opens what is dropped on the editor's windows, from when window `id` is open.
fn take_drops(id: window::Id) -> Task<Message> {
    window::run(id, crate::drops::display).then(|display| {
        display.map_or_else(Task::none, |display| {
            Task::run(crate::drops::drops(display), Message::Dropped)
        })
    })
}

impl Editor {
    /// Brings the window to the front. On Wayland a window cannot raise itself: it takes an
    /// activation `token` from whatever the user clicked. Without one, the window is opened
    /// anew, which most compositors put in front.
    fn raise(&mut self, token: Option<String>) -> Task<Message> {
        let remap = |id| placement(id).map(move |placement| Message::Remap(id, placement));
        match (self.window, token) {
            (Some(id), Some(token)) => {
                activate(id, token).then(move |done| if done { Task::none() } else { remap(id) })
            }
            (Some(id), None) => remap(id),
            (None, token) => self.reopen(None, token),
        }
    }

    /// Opens the window as it was placed, closing `old` first, and brings it to the front with
    /// `token` if given.
    fn reopen(&mut self, old: Option<window::Id>, token: Option<String>) -> Task<Message> {
        let closed = old.map_or_else(Task::none, window::close);
        let (id, opened) = window::open(settings(self.placement));
        self.window = Some(id);
        let opened = match token {
            Some(token) => opened.then(move |id| activate(id, token.clone())).discard(),
            None => opened.discard(),
        };
        closed.chain(opened)
    }
}

/// Brings window `id` to the front with an activation `token`; true when the compositor was
/// asked.
fn activate(id: window::Id, token: String) -> Task<bool> {
    window::run(id, move |window| {
        crate::activation::activate(window, &token)
    })
}

/// The size and state of window `id`.
fn placement(id: window::Id) -> Task<Placement> {
    window::is_maximized(id)
        .then(move |maximized| window::size(id).map(move |size| Placement { size, maximized }))
}

fn settings(placement: Placement) -> window::Settings {
    window::Settings {
        size: placement.size,
        maximized: placement.maximized,
        min_size: Some(Size::new(760.0, 520.0)),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: crate::APP_ID.to_owned(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    }
}

/// Runs the overlay in its own process, after the window has had time to hide.
async fn capture() -> Result<(), String> {
    tokio::time::sleep(Duration::from_millis(350)).await;
    let exe = crate::launch::exe().map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .args(["capture", "--edit"])
            .status()
            .map_err(|error| format!("cannot take a screenshot: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err("the screenshot failed".to_owned())
                }
            })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Whether a change is not one step of a slider drag, whose last step follows.
fn settled(message: &Message) -> bool {
    !matches!(
        message,
        Message::Weight(controls::WeightEvent::Slide(_))
            | Message::Brightness(controls::BrightnessEvent::Slide(_))
            | Message::Look(controls::TextEvent::Size(_))
            | Message::Frame(frame::Event::Margin(_))
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Export {
    Copy,
    Save,
    Project,
    /// To a file the user chose.
    To(PathBuf, crate::save::Format),
}

#[allow(clippy::needless_pass_by_value)]
fn typing_key(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::Modifiers(modifiers))
        }
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) => Some(Message::Escape),
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            quit_key(&key, modifiers)
        }
        _ => None,
    }
}

/// Ctrl+Q closes the editor as its window's close button does, asking about unsaved changes.
fn quit_key(key: &keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    (modifiers.command() && key.as_ref() == keyboard::Key::Character("q"))
        .then_some(Message::CloseRequested)
}

/// The window losing the keyboard, when the data device is made anew to take drops.
#[allow(clippy::needless_pass_by_value)]
fn focus(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    matches!(event, Event::Window(window::Event::Unfocused)).then_some(Message::Unfocused)
}

/// The crop that keeps `rect` of an image of `size`, in whole pixels: none for all of it.
fn kept(rect: iced::Rectangle, size: Size) -> Option<kurbo::Rect> {
    let rect = kurbo::Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x + rect.width),
        f64::from(rect.y + rect.height),
    )
    .round();
    let whole = kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
    (rect != whole && rect.area() >= 1.0).then_some(rect)
}

/// Enter keeps the crop; every other key works as usual.
fn crop_key(event: Event, status: event::Status, id: window::Id) -> Option<Message> {
    if let Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Enter),
        ..
    }) = event
    {
        return Some(Message::CropDone);
    }
    key(event, status, id)
}

#[allow(clippy::cast_possible_truncation)]
fn to_point(at: kurbo::Point) -> Point {
    Point::new(at.x as f32, at.y as f32)
}

#[allow(clippy::cast_possible_truncation)]
fn to_rectangle(rect: kurbo::Rect) -> iced::Rectangle {
    iced::Rectangle::new(
        Point::new(rect.x0 as f32, rect.y0 as f32),
        Size::new(rect.width() as f32, rect.height() as f32),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn key(event: Event, status: event::Status, _: window::Id) -> Option<Message> {
    use keyboard::key::Named;
    if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
        return Some(Message::Modifiers(modifiers));
    }
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    if let Some(quit) = quit_key(&key, modifiers) {
        return Some(quit);
    }
    if status == event::Status::Captured {
        return None;
    }
    let command = modifiers.command();
    if command
        && let keyboard::Key::Character(written) = key.as_ref()
        && let Some(to) = crate::marks::restack_key(written)
    {
        return Some(Message::Restack(to));
    }
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Enter) if modifiers.shift() || command => Some(Message::Save),
        keyboard::Key::Named(Named::Delete | Named::Backspace) => Some(Message::Delete),
        keyboard::Key::Character("z") if command && modifiers.shift() => Some(Message::Redo),
        keyboard::Key::Character("z") if command => Some(Message::Undo),
        keyboard::Key::Character("y") if command => Some(Message::Redo),
        keyboard::Key::Character("c") if command => Some(Message::CopyKey),
        keyboard::Key::Character("x") if command => Some(Message::Cut),
        keyboard::Key::Character("d") if command => Some(Message::Duplicate),
        keyboard::Key::Character("v") if command => Some(Message::Paste),
        keyboard::Key::Character("w") if command => Some(Message::Discard),
        keyboard::Key::Character("a") if command => Some(Message::SelectAll),
        keyboard::Key::Character(",") if command => Some(Message::Settings(settings::Event::Open)),
        keyboard::Key::Character("o") if command => Some(Message::Open),
        keyboard::Key::Character("s") if command && modifiers.shift() => Some(Message::SaveAs),
        keyboard::Key::Character("s") if command && modifiers.alt() => Some(Message::SaveProject),
        keyboard::Key::Character("s") if command => Some(Message::Save),
        keyboard::Key::Character("0") if command => Some(Message::ZoomFit),
        keyboard::Key::Character("1") if command => Some(Message::ZoomActual),
        keyboard::Key::Character("=" | "+") if command => Some(Message::ZoomIn),
        keyboard::Key::Character("-") if command => Some(Message::ZoomOut),
        keyboard::Key::Character("c") if !command => Some(Message::Crop),
        keyboard::Key::Character("m") if !command => {
            Some(Message::MarkList(mark_list::Event::Toggle))
        }
        keyboard::Key::Character(c) if !command => c
            .chars()
            .next()
            .and_then(Tool::from_key)
            .map(Message::PickTool),
        _ => None,
    }
}

/// A card the editor opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    About,
    Settings,
}

/// Opens the editor window, with `loaded` in it if given, and `card` if given; the activation
/// `token` brings it to the front.
pub fn run(loaded: Option<Loaded>, card: Option<Card>, token: Option<String>) -> iced::Result {
    let loaded = std::sync::Mutex::new(loaded);
    iced::daemon(
        move || {
            let (mut editor, task) = Editor::boot_with(
                loaded.lock().ok().and_then(|mut loaded| loaded.take()),
                token.clone(),
            );
            match card {
                Some(Card::About) => editor.about = true,
                Some(Card::Settings) => {
                    let _ = editor.edit_settings(settings::Event::Open);
                }
                None => {}
            }
            let checked = if editor.updates.check() {
                editor.check_release(false)
            } else {
                Task::none()
            };
            (editor, Task::batch([task, checked]))
        },
        Editor::update,
        Editor::view,
    )
    .antialiasing(true)
    .font(kirie_annotate::text::EMOJI_FONT)
    .title(Editor::title)
    .theme(Editor::theme)
    .subscription(Editor::subscription)
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_q_closes_the_editor_even_while_typing() {
        let press = |key: &str, modifiers| {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(key.into()),
                modified_key: keyboard::Key::Character(key.into()),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            })
        };
        let id = window::Id::unique();
        let ctrl_q = || press("q", keyboard::Modifiers::CTRL);
        for listen in [key, typing_key, crop_key] {
            assert!(matches!(
                listen(ctrl_q(), event::Status::Ignored, id),
                Some(Message::CloseRequested)
            ));
        }
        assert!(matches!(
            key(ctrl_q(), event::Status::Captured, id),
            Some(Message::CloseRequested)
        ));
        assert!(
            typing_key(
                press("q", keyboard::Modifiers::empty()),
                event::Status::Ignored,
                id
            )
            .is_none()
        );
    }

    #[test]
    fn crops_keep_whole_pixels_and_all_of_the_image_is_no_crop() {
        let size = Size::new(200.0, 100.0);
        let part = iced::Rectangle::new(Point::new(10.4, 20.6), Size::new(50.2, 30.0));
        assert_eq!(
            kept(part, size),
            Some(kurbo::Rect::new(10.0, 21.0, 61.0, 51.0))
        );
        let all = iced::Rectangle::new(Point::new(0.2, -0.3), Size::new(199.9, 100.1));
        assert_eq!(kept(all, size), None);
        let sliver = iced::Rectangle::new(Point::new(5.0, 5.0), Size::new(0.3, 40.0));
        assert_eq!(kept(sliver, size), None);
    }
}
