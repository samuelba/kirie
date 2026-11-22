//! App mode: a window to mark up any image or screenshot, keep it editable, and export it.

mod canvas;
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

use self::viewport::Viewport;

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
    PickColor(Rgba),
    PickWeight(Weight),
    ToggleFill,
    ToggleScheme,
    TextChanged(String),
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
struct Typing {
    at: kurbo::Point,
    text: String,
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
    viewport: Viewport,
    tool: Tool,
    paint: Paint,
    draft: Option<Draft>,
    picked: Option<ShapeId>,
    typing: Option<Typing>,
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
        let mut editor = Self {
            palette: Palette::DARK,
            image: None,
            path: None,
            history: History::new(Document::new(1, 1, 1.0)),
            viewport: Viewport::fitted(),
            tool: Tool::Arrow,
            paint: Paint::default(),
            draft: None,
            picked: None,
            typing: None,
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
        self.paint.color = loaded.document.style.palette()[0];
        self.history = History::new(loaded.document);
        self.image = Some((loaded.image, handle));
        self.path = loaded.path;
        self.viewport = Viewport::fitted();
        self.draft = None;
        self.picked = None;
        self.typing = None;
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
                self.paint.color = crate::controls::tool_color(
                    Some(tool),
                    self.paint.color,
                    self.document().style,
                );
                self.draft = None;
                if tool != Tool::Select {
                    self.picked = None;
                }
            }
            Message::PickStyle(style) => {
                self.history.apply(Op::SetStyle(style));
                self.paint.color = style.palette()[0];
            }
            Message::PickColor(color) => self.repaint(|paint| paint.color = color),
            Message::PickWeight(weight) => self.repaint(|paint| paint.weight = weight),
            Message::ToggleFill => self.repaint(|paint| paint.filled = !paint.filled),
            Message::SystemScheme(scheme) => self.follow(Some(scheme)),
            Message::ToggleScheme => self.follow(None),
            Message::TextChanged(text) => {
                if let Some(typing) = &mut self.typing {
                    typing.text = text;
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
            Message::Escape => {
                if self.typing.take().is_none() && self.draft.take().is_none() {
                    self.picked = None;
                }
            }
            Message::Copy
            | Message::Open
            | Message::Paste
            | Message::Save
            | Message::SaveProject => return self.files(&message),
            Message::Done(Ok(text)) if text.is_empty() => {}
            Message::Done(result) => return self.toast(result),
            Message::Opened(Ok(loaded)) => {
                self.open(loaded);
                return present();
            }
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
        change(&mut self.paint);
        if let Some(id) = self.picked
            && let Some(shape) = self.document().shape(id)
        {
            let mut paint = shape.paint;
            change(&mut paint);
            self.history.apply(Op::Repaint {
                ids: vec![id],
                paint,
            });
        }
    }

    fn pointer(&mut self, pointer: Pointer, scale: f32) -> Task<Message> {
        if self.image.is_none() {
            return Task::none();
        }
        match pointer {
            Pointer::Pressed(at) => {
                self.commit_text();
                let reach = MARK_REACH / f64::from(scale.max(0.01));
                match kirie_annotate::tools::press(
                    self.tool,
                    at,
                    self.document(),
                    self.paint,
                    reach,
                ) {
                    Press::Draft(draft) => {
                        if let Draft::Moving { ids, .. } = &draft {
                            self.picked = ids.first().copied();
                        }
                        self.draft = Some(draft);
                    }
                    Press::Done(op) => self.history.apply(op),
                    Press::TypeAt(at) => {
                        self.typing = Some(Typing {
                            at,
                            text: String::new(),
                        });
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
        let Some(typing) = self.typing.take() else {
            return;
        };
        if typing.text.trim().is_empty() {
            return;
        }
        self.history.apply(Op::Add {
            kind: Kind::Text {
                at: typing.at,
                text: typing.text,
            },
            paint: self.paint,
        });
    }

    fn export(&mut self, export: Export) -> Task<Message> {
        self.commit_text();
        let Some((image, _)) = &self.image else {
            return Task::none();
        };
        if export == Export::Project {
            let project = Project {
                image: image.clone(),
                document: self.document().clone(),
            };
            return Task::perform(crate::save::save_project(project), |saved| {
                Message::Done(saved.map(|path| format!("Saved {}", crate::save::pretty(&path))))
            });
        }
        let rendered = kirie_annotate::render::render(
            self.document(),
            image,
            &mut self.typesetter.borrow_mut(),
        );
        let picture = match rendered {
            Ok(picture) => picture,
            Err(error) => return self.toast(Err(error.to_string())),
        };
        match export {
            Export::Copy => Task::perform(crate::clipboard::copy(picture), |copied| {
                Message::Done(copied.map(|()| "Copied to the clipboard".to_owned()))
            }),
            Export::Save | Export::Project => Task::perform(crate::save::save(picture), |saved| {
                Message::Done(saved.map(|path| format!("Saved {}", crate::save::pretty(&path))))
            }),
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
        let keys = if self.typing.is_some() {
            event::listen_with(typing_key)
        } else {
            event::listen_with(key)
        };
        Subscription::batch([
            keys,
            Subscription::run(crate::ipc::requests).map(Message::Ipc),
            Subscription::run(crate::appearance::changes).map(Message::SystemScheme),
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
        platform_specific: window::settings::PlatformSpecific {
            application_id: crate::APP_ID.to_owned(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    })
    .run()
}
