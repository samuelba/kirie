//! App mode: a window to mark up any image or screenshot, keep it editable, and export it.

mod canvas;
mod closing;
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
    Document, Draft, History, Kind, Op, Paint, Press, Rgba, ShapeId, StyleId, Tool, Typesetter,
    Weight,
};
use kirie_ui::{Palette, Scheme};

use self::closing::{Answer, Closing};
use self::viewport::Viewport;
use crate::controls;
use crate::picker::{self, Picker, Target};
use crate::swatches::Swatches;
use crate::typing::{self, Typing};

/// How close to a mark a click picks it, in screen pixels.
const MARK_REACH: f64 = 6.0;

/// An image open for editing.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub image: RgbaImage,
    pub document: Document,
    pub path: Option<PathBuf>,
}

impl Loaded {
    pub fn image(image: RgbaImage, path: Option<PathBuf>) -> Self {
        let document = Document::new(image.width(), image.height(), 1.0);
        Self {
            image,
            document,
            path,
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
        });
    }
    let image = ::image::open(path)
        .map_err(|error| fail(&error))?
        .to_rgba8();
    Ok(Loaded::image(image, Some(path.to_path_buf())))
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
    Look(controls::TextEvent),
    ToggleScheme,
    TextEdit(iced::widget::text_editor::Action),
    TextDone,
    Undo,
    Redo,
    Delete,
    Escape,
    Copy,
    Open,
    Paste,
    Save,
    SaveProject,
    Done(Result<String, String>),
    /// The image left the editor, saved or copied, as this document had it.
    Exported(String, Box<Document>),
    CloseRequested,
    Close(Answer),
    Opened(Result<Loaded, String>),
    ToastExpired(u64),
    Ipc(crate::ipc::Request),
    /// The desktop switched between light and dark.
    SystemScheme(Scheme),
    /// Hides the window and opens the overlay; the capture comes back over D-Bus.
    Capture,
    CaptureEnded(Result<(), String>),
}

#[derive(Debug, Clone)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
}

pub struct Editor {
    palette: Palette,
    image: Option<(RgbaImage, image::Handle)>,
    path: Option<PathBuf>,
    history: History,
    /// The document as last opened, saved or copied, to tell unsaved changes.
    kept: Document,
    closing: Closing,
    /// An image to open once the question about unsaved changes is answered.
    then_open: Option<Box<Loaded>>,
    viewport: Viewport,
    tool: Tool,
    paint: Paint,
    swatches: Swatches,
    picker: Option<Picker>,
    draft: Option<Draft>,
    picked: Option<ShapeId>,
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
    pub fn boot(loaded: Option<Loaded>) -> (Self, Task<Message>) {
        let swatches = Swatches::load();
        let mut editor = Self {
            palette: Palette::DARK,
            image: None,
            path: None,
            history: History::new(Document::new(1, 1, 1.0)),
            kept: Document::new(1, 1, 1.0),
            closing: Closing::No,
            then_open: None,
            viewport: Viewport::fitted(),
            tool: Tool::Arrow,
            paint: Paint {
                color: swatches.first(),
                ..Paint::default()
            },
            swatches,
            picker: None,
            draft: None,
            picked: None,
            sliding: false,
            typing: None,
            last_click: None,
            modifiers: keyboard::Modifiers::default(),
            toast: None,
            toasts: 0,
            scheme_chosen: false,
            typesetter: RefCell::new(Typesetter::new()),
            patches: RefCell::new(HashMap::new()),
        };
        if let Some(loaded) = loaded {
            editor.open(loaded);
        }
        (editor, Task::none())
    }

    fn open(&mut self, mut loaded: Loaded) {
        if let Some(path) = loaded.path.take_if(|path| crate::save::is_handoff(path)) {
            let _ = std::fs::remove_file(path);
        }
        let handle = image::Handle::from_rgba(
            loaded.image.width(),
            loaded.image.height(),
            loaded.image.as_raw().clone(),
        );
        self.kept = loaded.document.clone();
        self.history = History::new(loaded.document);
        self.image = Some((loaded.image, handle));
        self.path = loaded.path;
        self.viewport = Viewport::fitted();
        self.draft = None;
        self.picked = None;
        self.typing = None;
        self.sliding = false;
        self.closing = Closing::No;
        self.then_open = None;
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

    pub fn title(&self) -> String {
        match self.path.as_deref().and_then(Path::file_name) {
            Some(name) => format!("{} — Kirie", name.to_string_lossy()),
            None => "Kirie".to_owned(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(pointer, scale) => return self.pointer(pointer, scale),
            Message::Pan(by) => self.viewport = self.viewport.panned(by),
            Message::Zoom {
                factor,
                anchor,
                canvas,
            } => {
                self.viewport = self
                    .viewport
                    .zoomed(factor, anchor, self.image_size(), canvas);
            }
            Message::ZoomFit => self.viewport = Viewport::fitted(),
            Message::ZoomActual => self.viewport = Viewport::actual_size(),
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::PickTool(tool) => {
                self.commit_text();
                self.tool = tool;
                self.paint.color =
                    controls::tool_color(Some(tool), self.paint.color, self.swatches.first());
                self.draft = None;
                if tool != Tool::Select {
                    self.picked = None;
                }
            }
            Message::PickStyle(_)
            | Message::ToggleShadow
            | Message::PickColor(_)
            | Message::EditColor(_)
            | Message::Picker(_)
            | Message::Weight(_)
            | Message::ToggleFill
            | Message::Look(_) => return self.restyle(message),
            Message::SystemScheme(scheme) => self.follow(Some(scheme)),
            Message::ToggleScheme => self.follow(None),
            Message::TextEdit(action) => {
                if let Some(typing) = &mut self.typing {
                    typing.content.perform(action);
                }
            }
            Message::TextDone => self.commit_text(),
            Message::Undo => {
                self.history.undo();
            }
            Message::Redo => {
                self.history.redo();
            }
            Message::Delete => {
                if let Some(id) = self.picked.take() {
                    self.history.apply(Op::Remove { ids: vec![id] });
                }
            }
            Message::Escape if self.closing == Closing::Asking => self.closing = Closing::No,
            Message::Escape => {
                if self.picker.take().is_none()
                    && self.typing.take().is_none()
                    && self.draft.take().is_none()
                {
                    self.picked = None;
                }
            }
            Message::Copy
            | Message::Open
            | Message::Paste
            | Message::Save
            | Message::SaveProject => return self.files(&message),
            Message::Done(Ok(text)) if text.is_empty() => {}
            Message::Done(_)
            | Message::Exported(..)
            | Message::CloseRequested
            | Message::Close(_) => return self.finish(message),
            Message::Opened(Ok(loaded)) => return self.replace(loaded),
            Message::Ipc(crate::ipc::Request::Open(path)) => {
                return Task::perform(load_async(path), Message::Opened);
            }
            Message::Ipc(crate::ipc::Request::Present) => return present(),
            Message::Capture => {
                return window::oldest()
                    .then(|id| id.map_or_else(Task::none, |id| window::minimize(id, true)))
                    .chain(Task::perform(capture(), Message::CaptureEnded));
            }
            Message::CaptureEnded(result) => {
                let shown = present();
                return match result {
                    Ok(()) => shown,
                    Err(error) => shown.chain(self.toast(Err(error))),
                };
            }
            Message::Opened(Err(error)) => return self.toast(Err(error)),
            Message::ToastExpired(id) => {
                if self.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    self.toast = None;
                }
            }
        }
        Task::none()
    }

    /// What became of an export, and closing the window.
    fn finish(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Done(result) => {
                if result.is_err() {
                    self.closing = Closing::No;
                    self.then_open = None;
                }
                self.toast(result)
            }
            Message::Exported(text, kept) => {
                self.kept = *kept;
                if self.closing == Closing::Saving {
                    self.closing = Closing::No;
                    let Some(next) = self.then_open.take() else {
                        return iced::exit();
                    };
                    self.open(*next);
                }
                self.toast(Ok(text))
            }
            Message::CloseRequested => self.close_requested(),
            Message::Close(answer) => self.answer(answer),
            _ => Task::none(),
        }
    }

    /// Changes to the style, colors, weight, fill and text look.
    fn restyle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PickStyle(style) => {
                self.history.apply(Op::SetStyle(style));
            }
            Message::ToggleShadow => {
                let shadow = !self.document().shadow;
                self.history.apply(Op::SetShadow(shadow));
            }
            Message::PickColor(color) => self.repaint(|paint| paint.color = color),
            Message::EditColor(target) => {
                self.picker = Some(Picker::new(target, &self.swatches, self.paint.color));
            }
            Message::Picker(event) => return self.edit_color(event),
            Message::Weight(controls::WeightEvent::Done)
            | Message::Look(controls::TextEvent::SizeDone) => self.sliding = false,
            Message::Weight(controls::WeightEvent::Pick(weight)) => {
                self.repaint(|paint| paint.weight = weight);
            }
            Message::Weight(controls::WeightEvent::Slide(width)) => {
                self.repaint_gradually(|paint| paint.weight = Weight::new(width), true);
            }
            Message::ToggleFill => self.repaint(|paint| paint.filled = !paint.filled),
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
    fn files(&mut self, message: &Message) -> Task<Message> {
        match message {
            Message::Copy => self.export(Export::Copy),
            Message::Open => Task::perform(crate::dialog::open_image(), |picked| match picked {
                Ok(Some(path)) => Message::Ipc(crate::ipc::Request::Open(path)),
                Ok(None) => Message::Done(Ok(String::new())),
                Err(error) => Message::Done(Err(error)),
            }),
            Message::Paste => Task::perform(crate::clipboard::paste(), |pasted| {
                Message::Opened(pasted.map(|image| Loaded::image(image, None)))
            }),
            Message::Save => self.export(Export::Save),
            Message::SaveProject => self.export(Export::Project),
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

    /// Changes the paint for new marks, and the picked mark's paint too.
    fn repaint(&mut self, change: impl Fn(&mut Paint)) {
        self.repaint_gradually(change, false);
    }

    /// Like [`Self::repaint`]; a `gradual` change joins the step the slider drag began.
    fn repaint_gradually(&mut self, change: impl Fn(&mut Paint), gradual: bool) {
        change(&mut self.paint);
        let Some(shape) = self.picked.and_then(|id| self.document().shape(id)) else {
            return;
        };
        let mut paint = shape.paint;
        change(&mut paint);
        let op = Op::Repaint {
            ids: vec![shape.id],
            paint,
        };
        if gradual {
            self.history.apply_gradually(op, self.sliding);
        } else {
            self.history.apply(op);
        }
        self.sliding = gradual;
    }

    /// The paint the controls show: the picked mark's, else the one new marks get.
    fn shown_paint(&self) -> Paint {
        self.picked
            .and_then(|id| self.document().shape(id))
            .map_or(self.paint, |shape| shape.paint)
    }

    /// Whether the text controls apply: to text being typed, a picked text mark, or the next.
    fn texting(&self) -> bool {
        self.tool == Tool::Text
            || self.typing.is_some()
            || self
                .picked
                .and_then(|id| self.document().shape(id))
                .is_some_and(|shape| matches!(shape.kind, Kind::Text { .. }))
    }

    fn pointer(&mut self, pointer: Pointer, scale: f32) -> Task<Message> {
        if self.image.is_none() {
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
                    Tool::Text => typing::text_at(self.document(), at, reach),
                    Tool::Select if double => typing::text_at(self.document(), at, reach),
                    _ => None,
                };
                if let Some(id) = words {
                    return self.edit_text(id);
                }
                match kirie_annotate::tools::press(
                    self.tool,
                    at,
                    self.document(),
                    self.paint,
                    reach,
                    self.picked,
                ) {
                    Press::Draft(draft) => {
                        if let Draft::Moving { ids, .. } = &draft {
                            self.picked = ids.first().copied();
                        }
                        self.draft = Some(draft);
                    }
                    Press::Done(op) => {
                        self.history.apply(op);
                    }
                    Press::TypeAt(at) => {
                        self.typing = Some(Typing::new(at));
                        return iced::widget::operation::focus(view::TEXT_INPUT);
                    }
                    Press::Nothing => self.picked = None,
                }
            }
            Pointer::Moved(at) => {
                if let Some(draft) = self.draft.take() {
                    self.draft = Some(draft.drag(at, self.modifiers.shift()));
                }
            }
            Pointer::Released(at) => {
                if let Some(draft) = self.draft.take()
                    && let Some(op) = draft.drag(at, self.modifiers.shift()).finish(self.paint)
                {
                    self.history.apply(op);
                }
            }
        }
        Task::none()
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
        self.picked = Some(id);
        self.draft = None;
        iced::widget::operation::focus(view::TEXT_INPUT)
    }

    fn export(&mut self, export: Export) -> Task<Message> {
        self.commit_text();
        let Some((image, _)) = &self.image else {
            return Task::none();
        };
        let kept = Box::new(self.document().clone());
        let exported = move |done: Result<String, String>| match done {
            Ok(text) => Message::Exported(text, kept),
            Err(error) => Message::Done(Err(error)),
        };
        let saved = |path: std::path::PathBuf| format!("Saved {}", crate::save::pretty(&path));
        if export == Export::Project {
            let project = Project {
                image: image.clone(),
                document: self.document().clone(),
            };
            return Task::perform(crate::save::save_project(project), move |done| {
                exported(done.map(saved))
            });
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
                exported(copied.map(|()| "Copied to the clipboard".to_owned()))
            }),
            Export::Save | Export::Project => {
                Task::perform(crate::save::save(picture), move |done| {
                    exported(done.map(saved))
                })
            }
        }
    }

    fn toast(&mut self, result: Result<String, String>) -> Task<Message> {
        self.toasts += 1;
        let id = self.toasts;
        let (text, error) = match result {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        };
        self.toast = Some(Toast { id, text, error });
        Task::perform(tokio::time::sleep(Duration::from_secs(4)), move |()| {
            Message::ToastExpired(id)
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let quiet = self.typing.is_some() || self.picker.is_some() || self.closing != Closing::No;
        let keys = if quiet {
            event::listen_with(typing_key)
        } else {
            event::listen_with(key)
        };
        Subscription::batch([
            keys,
            Subscription::run(crate::ipc::requests).map(Message::Ipc),
            Subscription::run(crate::appearance::changes).map(Message::SystemScheme),
            window::close_requests().map(|_| Message::CloseRequested),
        ])
    }

    pub fn theme(&self) -> iced::Theme {
        match self.palette.scheme {
            Scheme::Dark => iced::Theme::Dark,
            Scheme::Light => iced::Theme::Light,
        }
    }
}

async fn load_async(path: PathBuf) -> Result<Loaded, String> {
    tokio::task::spawn_blocking(move || load(&path))
        .await
        .map_err(|error| error.to_string())?
}

/// Raises and focuses the editor window.
fn present() -> Task<Message> {
    window::oldest().then(|id| {
        id.map_or_else(Task::none, |id| {
            Task::batch([window::minimize(id, false), window::gain_focus(id)])
        })
    })
}

/// Runs the overlay in its own process, after the window has had time to hide.
async fn capture() -> Result<(), String> {
    tokio::time::sleep(Duration::from_millis(350)).await;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Export {
    Copy,
    Save,
    Project,
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
        _ => None,
    }
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
    if status == event::Status::Captured {
        return None;
    }
    let command = modifiers.command();
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Delete | Named::Backspace) => Some(Message::Delete),
        keyboard::Key::Character("z") if command && modifiers.shift() => Some(Message::Redo),
        keyboard::Key::Character("z") if command => Some(Message::Undo),
        keyboard::Key::Character("y") if command => Some(Message::Redo),
        keyboard::Key::Character("c") if command => Some(Message::Copy),
        keyboard::Key::Character("v") if command => Some(Message::Paste),
        keyboard::Key::Character("o") if command => Some(Message::Open),
        keyboard::Key::Character("s") if command && modifiers.shift() => Some(Message::SaveProject),
        keyboard::Key::Character("s") if command => Some(Message::Save),
        keyboard::Key::Character("0") if command => Some(Message::ZoomFit),
        keyboard::Key::Character("1") if command => Some(Message::ZoomActual),
        keyboard::Key::Character(c) if !command => c
            .chars()
            .next()
            .and_then(Tool::from_key)
            .map(Message::PickTool),
        _ => None,
    }
}

/// Opens the editor window, with `loaded` in it if given.
pub fn run(loaded: Option<Loaded>) -> iced::Result {
    let loaded = std::sync::Mutex::new(loaded);
    iced::application(
        move || Editor::boot(loaded.lock().ok().and_then(|mut loaded| loaded.take())),
        Editor::update,
        Editor::view,
    )
    .antialiasing(true)
    .title(Editor::title)
    .theme(Editor::theme)
    .subscription(Editor::subscription)
    .window(window::Settings {
        size: Size::new(1280.0, 840.0),
        min_size: Some(Size::new(760.0, 520.0)),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: crate::APP_ID.to_owned(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    })
    .run()
}
