//! The on-screen mode: every screen frozen on a surface of its own, where the user picks a
//! region, marks it up, and copies or saves it.

mod canvas;
mod selection;
mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::widget::image;
use iced::{Event, Point, Rectangle, Size, Subscription, Task, Vector, event, keyboard, window};
use iced_layershell::actions::{LayerShellCustomAction, LayerShellCustomActionWithId};
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use kirie_annotate::{
    Document, Draft, History, Kind, Op, Paint, Press, StyleId, Tool, Typesetter, Weight,
};
use kirie_capture::{Frame, geometry};
use kirie_ui::Palette;

use self::selection::{Release, Selection};

pub const NAMESPACE: &str = "kirie-overlay";
/// How close to a handle a press grabs it, in logical pixels.
const HANDLE_REACH: f32 = 8.0;
/// How close to a mark the select tool picks it, in logical pixels.
const MARK_REACH: f64 = 6.0;

/// One frozen screen.
#[derive(Debug)]
struct Screen {
    handle: image::Handle,
    area: Rectangle,
    scale: f64,
}

#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Pressed(Point),
    Moved(Point),
    Released(Point),
}

/// What to do with the finished picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Copy,
    Save,
    /// Hands the selection and its marks to the editor.
    Edit,
}

#[derive(Debug, Clone)]
pub enum Message {
    Pointer(Pointer),
    Modifiers(keyboard::Modifiers),
    PickTool(Option<Tool>),
    PickStyle(StyleId),
    PickColor(kirie_annotate::Rgba),
    PickWeight(Weight),
    ToggleFill,
    TextChanged(String),
    TextDone,
    Undo,
    Redo,
    Escape,
    Finish(Action),
    /// Enter: the action the overlay was opened for.
    Accept,
    SelectAll,
    Nudge(Vector),
    Finished(Result<Outcome, String>),
    Resized(window::Id, Size),
    Layer(LayerShellCustomActionWithId),
}

/// How the overlay's surfaces are made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// A layer-shell surface on every screen, above everything.
    Layer,
    /// One fullscreen window, for desktops without layer-shell (GNOME without Kirie's
    /// extension).
    Window,
    /// A window per screen, titled so Kirie's GNOME Shell extension puts each on its screen
    /// and above everything.
    Shell,
}

impl TryInto<LayerShellCustomActionWithId> for Message {
    type Error = Self;

    fn try_into(self) -> Result<LayerShellCustomActionWithId, Self> {
        match self {
            Self::Layer(action) => Ok(action),
            message => Err(message),
        }
    }
}

/// How the overlay ended.
#[derive(Debug, Clone, Default)]
pub enum Outcome {
    #[default]
    Cancelled,
    Saved(PathBuf),
    Copied,
    /// A project for the editor to open.
    Edit(PathBuf),
    Failed(String),
}

/// Text being typed for a text mark.
#[derive(Debug, Clone)]
struct Typing {
    at: kurbo::Point,
    text: String,
}

pub struct Overlay {
    captures: Vec<Frame>,
    screens: Vec<Screen>,
    surfaces: HashMap<window::Id, usize>,
    desktop: Rectangle,
    selection: Selection,
    pointer: Option<Point>,
    modifiers: keyboard::Modifiers,
    palette: Palette,
    tool: Option<Tool>,
    paint: Paint,
    history: History,
    draft: Option<Draft>,
    /// The tool the current pointer gesture draws with, if it draws at all.
    drawing: Option<Tool>,
    typing: Option<Typing>,
    busy: bool,
    /// What Enter does.
    primary: Action,
    typesetter: RefCell<Typesetter>,
    patches: RefCell<HashMap<crate::paint::PatchKey, image::Handle>>,
    outcome: Arc<Mutex<Outcome>>,
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("screens", &self.screens.len())
            .field("tool", &self.tool)
            .finish_non_exhaustive()
    }
}

fn area(rect: geometry::Rect) -> Rectangle {
    #[allow(clippy::cast_precision_loss)]
    Rectangle::new(
        Point::new(rect.x as f32, rect.y as f32),
        Size::new(rect.width as f32, rect.height as f32),
    )
}

#[allow(clippy::cast_possible_truncation)]
fn whole(area: Rectangle) -> geometry::Rect {
    let (x, y) = (area.x.round() as i32, area.y.round() as i32);
    geometry::Rect::new(
        x,
        y,
        (area.x + area.width).round() as i32 - x,
        (area.y + area.height).round() as i32 - y,
    )
}

fn to_kurbo(point: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(point.x), f64::from(point.y))
}

impl Overlay {
    pub fn boot(
        frames: Vec<Frame>,
        primary: Action,
        host: Host,
        outcome: Arc<Mutex<Outcome>>,
    ) -> (Self, Task<Message>) {
        let screens: Vec<Screen> = frames
            .iter()
            .map(|frame| Screen {
                handle: image::Handle::from_rgba(
                    frame.image.width(),
                    frame.image.height(),
                    frame.image.as_raw().clone(),
                ),
                area: area(frame.output.logical),
                scale: frame.scale(),
            })
            .collect();
        let desktop = screens
            .iter()
            .map(|screen| screen.area)
            .reduce(|all, area| all.union(&area))
            .unwrap_or_default();
        let mut surfaces = HashMap::new();
        let open: Vec<Task<Message>> = match host {
            Host::Layer => frames
                .iter()
                .enumerate()
                .map(|(index, frame)| {
                    let id = window::Id::unique();
                    surfaces.insert(id, index);
                    Task::done(Message::Layer(LayerShellCustomActionWithId::new(
                        None,
                        LayerShellCustomAction::NewLayerShell {
                            settings: surface_settings(&frame.output.name),
                            id,
                        },
                    )))
                })
                .collect(),
            // Without layer-shell the compositor puts the window on a screen of its choosing;
            // which one it is shows once the window has its size.
            Host::Window => {
                let (id, open) = window::open(window_settings());
                surfaces.insert(id, 0);
                vec![open.discard()]
            }
            Host::Shell => (0..frames.len())
                .map(|index| {
                    let (id, open) = window::open(window_settings());
                    surfaces.insert(id, index);
                    open.discard()
                })
                .collect(),
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let document = Document::new(
            (desktop.x + desktop.width).max(1.0) as u32,
            (desktop.y + desktop.height).max(1.0) as u32,
            1.0,
        );
        let overlay = Self {
            captures: frames,
            screens,
            surfaces,
            desktop,
            selection: Selection::default(),
            pointer: None,
            modifiers: keyboard::Modifiers::default(),
            palette: Palette::DARK,
            tool: None,
            paint: Paint::default(),
            history: History::new(document),
            draft: None,
            drawing: None,
            typing: None,
            busy: false,
            primary,
            typesetter: RefCell::new(Typesetter::new()),
            patches: RefCell::new(HashMap::new()),
            outcome,
        };
        let overlay = if std::env::var_os("KIRIE_OVERLAY_DEMO").is_some() {
            overlay.with_demo()
        } else {
            overlay
        };
        (overlay, Task::batch(open))
    }

    /// A selection with a few marks, for screenshots of the overlay and the smoke test.
    fn with_demo(mut self) -> Self {
        let Some(screen) = self.screens.first() else {
            return self;
        };
        let area = screen.area;
        let region = Rectangle::new(
            Point::new(area.x + area.width * 0.2, area.y + area.height * 0.15),
            Size::new(area.width * 0.6, area.height * 0.5),
        );
        self.selection = Selection::selected(region);
        self.tool = Some(Tool::Arrow);
        let point = |x: f32, y: f32| {
            kurbo::Point::new(
                f64::from(region.x + region.width * x),
                f64::from(region.y + region.height * y),
            )
        };
        for kind in [
            Kind::Rectangle {
                rect: kurbo::Rect::from_points(point(0.08, 0.15), point(0.45, 0.55)),
            },
            Kind::Arrow {
                from: point(0.8, 0.85),
                to: point(0.5, 0.5),
            },
            Kind::Counter {
                at: point(0.1, 0.15),
                number: 1,
            },
            Kind::Text {
                at: point(0.55, 0.15),
                text: "Kirie".into(),
            },
        ] {
            self.history.apply(Op::Add {
                kind,
                paint: self.paint,
            });
        }
        self
    }

    fn document(&self) -> &Document {
        self.history.current()
    }

    fn style(&self) -> StyleId {
        self.document().style
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(pointer) => self.pointer(pointer),
            Message::Modifiers(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::PickTool(tool) => {
                self.commit_text();
                self.tool = tool;
                self.paint.color =
                    crate::controls::tool_color(tool, self.paint.color, self.style());
                self.draft = None;
                Task::none()
            }
            Message::PickStyle(style) => {
                self.history.apply(Op::SetStyle(style));
                self.paint.color = style.palette()[0];
                Task::none()
            }
            Message::PickColor(color) => {
                self.paint.color = color;
                Task::none()
            }
            Message::PickWeight(weight) => {
                self.paint.weight = weight;
                Task::none()
            }
            Message::ToggleFill => {
                self.paint.filled = !self.paint.filled;
                Task::none()
            }
            Message::TextChanged(text) => {
                if let Some(typing) = &mut self.typing {
                    typing.text = text;
                }
                Task::none()
            }
            Message::TextDone => {
                self.commit_text();
                Task::none()
            }
            Message::Undo => {
                self.history.undo();
                Task::none()
            }
            Message::Redo => {
                self.history.redo();
                Task::none()
            }
            Message::Escape => self.escape(),
            Message::Nudge(by) => {
                self.selection = self.selection.nudge(by, self.desktop);
                Task::none()
            }
            Message::SelectAll => {
                self.selection = Selection::selected(self.desktop);
                Task::none()
            }
            Message::Finish(action) => self.finish(action),
            Message::Accept => self.finish(self.primary),
            Message::Finished(Ok(outcome)) => self.close(outcome),
            Message::Finished(Err(error)) => self.close(Outcome::Failed(error)),
            Message::Resized(id, size) => {
                self.place(id, size);
                Task::none()
            }
            Message::Layer(_) => Task::none(),
        }
    }

    /// Shows the screen a fullscreen window landed on: the one with the window's size.
    fn place(&mut self, id: window::Id, size: Size) {
        let Some(index) = self.screens.iter().position(|screen| {
            (screen.area.width - size.width).abs() < 1.0
                && (screen.area.height - size.height).abs() < 1.0
        }) else {
            return;
        };
        if self.surfaces.len() > 1 {
            return;
        }
        if let Some(shown) = self.surfaces.get_mut(&id)
            && *shown != index
        {
            *shown = index;
            if self.surfaces.len() == 1 {
                self.desktop = self.screens[index].area;
                self.selection = Selection::default();
            }
        }
    }

    fn escape(&mut self) -> Task<Message> {
        // A save or copy on its way finishes first; leaving now would drop it half done.
        if self.busy {
            return Task::none();
        }
        if self.typing.take().is_some() || self.draft.take().is_some() {
            return Task::none();
        }
        if self.tool.take().is_some() {
            return Task::none();
        }
        self.close(Outcome::Cancelled)
    }

    fn pointer(&mut self, pointer: Pointer) -> Task<Message> {
        let at = match pointer {
            Pointer::Pressed(at) | Pointer::Moved(at) | Pointer::Released(at) => at,
        };
        self.pointer = Some(at);
        // A gesture stays with what its press started, whatever happens on the way.
        if let Pointer::Pressed(at) = pointer {
            let inside = self.selection.hit(at, HANDLE_REACH) == Some(selection::Hit::Inside);
            self.drawing = match self.tool {
                Some(tool) if inside => Some(tool),
                _ => None,
            };
        }
        let task = if let Some(tool) = self.drawing {
            self.annotate(tool, pointer)
        } else {
            self.select(pointer);
            Task::none()
        };
        if let Pointer::Released(_) = pointer {
            self.drawing = None;
        }
        task
    }

    fn select(&mut self, pointer: Pointer) {
        match pointer {
            Pointer::Pressed(at) => self.selection = self.selection.press(at, HANDLE_REACH),
            Pointer::Moved(at) => self.selection = self.selection.drag_to(at, self.desktop),
            Pointer::Released(at) => {
                let (selection, release) = self.selection.release(at);
                self.selection = selection;
                if let Release::Click(at) = release
                    && let Some(screen) = self.screens.iter().find(|s| s.area.contains(at))
                {
                    self.selection = Selection::selected(screen.area);
                }
            }
        }
    }

    fn annotate(&mut self, tool: Tool, pointer: Pointer) -> Task<Message> {
        match pointer {
            Pointer::Pressed(at) => {
                self.commit_text();
                let press = kirie_annotate::tools::press(
                    tool,
                    to_kurbo(at),
                    self.document(),
                    self.paint,
                    MARK_REACH,
                );
                match press {
                    Press::Draft(draft) => self.draft = Some(draft),
                    Press::Done(op) => self.history.apply(op),
                    Press::TypeAt(at) => {
                        self.typing = Some(Typing {
                            at,
                            text: String::new(),
                        });
                        return iced::widget::operation::focus(view::TEXT_INPUT);
                    }
                    Press::Nothing => {}
                }
            }
            Pointer::Moved(at) => {
                if let Some(draft) = self.draft.take() {
                    self.draft = Some(draft.drag(to_kurbo(at), self.modifiers.shift()));
                }
            }
            Pointer::Released(at) => {
                if let Some(draft) = self.draft.take() {
                    let draft = draft.drag(to_kurbo(at), self.modifiers.shift());
                    if let Some(op) = draft.finish(self.paint) {
                        self.history.apply(op);
                    }
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

    /// The selection's pixels at the highest scale of the screens it covers, and the marks
    /// moved onto them.
    fn project(&self) -> Result<kirie_annotate::project::Project, String> {
        let region = self.selection.rect().ok_or("nothing is selected")?;
        let region = whole(region);
        let image = geometry::crop(&self.captures, region).ok_or("the selection is empty")?;
        let scale = f64::from(image.width()) / f64::from(region.width.max(1));
        let transform = kurbo::Affine::translate((-f64::from(region.x), -f64::from(region.y)))
            .then_scale(scale);
        let document = self
            .document()
            .transformed(transform, image.width(), image.height(), scale);
        Ok(kirie_annotate::project::Project { image, document })
    }

    /// The selection with every mark drawn on it.
    fn picture(&self) -> Result<::image::RgbaImage, String> {
        let project = self.project()?;
        kirie_annotate::render::render(
            &project.document,
            &project.image,
            &mut self.typesetter.borrow_mut(),
        )
        .map_err(|error| error.to_string())
    }

    fn finish(&mut self, action: Action) -> Task<Message> {
        self.commit_text();
        if self.busy || self.selection.rect().is_none() {
            return Task::none();
        }
        self.busy = true;
        if action == Action::Edit {
            let handed = self
                .project()
                .and_then(|project| crate::save::save_handoff(&project));
            return match handed {
                Ok(path) => self.close(Outcome::Edit(path)),
                Err(error) => self.close(Outcome::Failed(error)),
            };
        }
        let picture = match self.picture() {
            Ok(picture) => picture,
            Err(error) => return self.close(Outcome::Failed(error)),
        };
        match action {
            Action::Save => Task::perform(crate::save::save(picture), |saved| {
                Message::Finished(saved.map(Outcome::Saved))
            }),
            Action::Copy => Task::perform(crate::clipboard::copy(picture), |copied| {
                Message::Finished(copied.map(|()| Outcome::Copied))
            }),
            Action::Edit => Task::none(),
        }
    }

    fn close(&mut self, outcome: Outcome) -> Task<Message> {
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = outcome;
        }
        iced::exit()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let keys = if self.typing.is_some() {
            event::listen_with(typing_key)
        } else {
            event::listen_with(key)
        };
        Subscription::batch([
            keys,
            window::resize_events().map(|(id, size)| Message::Resized(id, size)),
        ])
    }

    /// What Kirie's GNOME Shell extension reads to put a window on its screen.
    pub fn title(&self, id: window::Id) -> String {
        format!(
            "kirie-overlay-{}",
            self.surfaces.get(&id).copied().unwrap_or(0)
        )
    }

    #[expect(
        clippy::unused_self,
        reason = "the overlay is always dark over a dimmed screen"
    )]
    pub fn theme(&self, _: window::Id) -> iced::Theme {
        iced::Theme::Dark
    }
}

fn window_settings() -> window::Settings {
    window::Settings {
        decorations: false,
        fullscreen: true,
        resizable: false,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: true,
        platform_specific: window::settings::PlatformSpecific {
            application_id: crate::APP_ID.to_owned(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    }
}

fn surface_settings(output: &str) -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some((0, 0)),
        layer: Layer::Overlay,
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        exclusive_zone: Some(-1),
        margin: Some((0, 0, 0, 0)),
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        output_option: OutputOption::OutputName(output.to_owned()),
        events_transparent: false,
        namespace: Some(NAMESPACE.to_owned()),
    }
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
    let step = if modifiers.shift() { 10.0 } else { 1.0 };
    let command = modifiers.command();
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Enter) => Some(Message::Accept),
        keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Nudge(Vector::new(-step, 0.0))),
        keyboard::Key::Named(Named::ArrowRight) => Some(Message::Nudge(Vector::new(step, 0.0))),
        keyboard::Key::Named(Named::ArrowUp) => Some(Message::Nudge(Vector::new(0.0, -step))),
        keyboard::Key::Named(Named::ArrowDown) => Some(Message::Nudge(Vector::new(0.0, step))),
        keyboard::Key::Character("z") if command && modifiers.shift() => Some(Message::Redo),
        keyboard::Key::Character("z") if command => Some(Message::Undo),
        keyboard::Key::Character("y") if command => Some(Message::Redo),
        keyboard::Key::Character("a") if command => Some(Message::SelectAll),
        keyboard::Key::Character("s") if command => Some(Message::Finish(Action::Save)),
        keyboard::Key::Character("c") if command => Some(Message::Finish(Action::Copy)),
        keyboard::Key::Character("e") if command => Some(Message::Finish(Action::Edit)),
        keyboard::Key::Character(c) if !command => c
            .chars()
            .next()
            .and_then(Tool::from_key)
            .map(|tool| Message::PickTool(Some(tool))),
        _ => None,
    }
}

/// Shows the overlay until the user is done: layer-shell surfaces on every screen where the
/// compositor has layer-shell, else one fullscreen window.
pub fn run(frames: Vec<Frame>, primary: Action, host: Host) -> Result<Outcome, String> {
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    let outcome = Arc::new(Mutex::new(Outcome::Cancelled));
    let shared = outcome.clone();
    let frames = Mutex::new(Some(frames));
    let boot = move || {
        let frames = frames
            .lock()
            .ok()
            .and_then(|mut frames| frames.take())
            .unwrap_or_default();
        Overlay::boot(frames, primary, host, shared.clone())
    };
    let style = |_: &Overlay, _: &iced::Theme| iced::theme::Style {
        background_color: iced::Color::TRANSPARENT,
        text_color: iced::Color::WHITE,
    };
    let ran = match host {
        Host::Layer => iced_layershell::daemon(boot, NAMESPACE, Overlay::update, Overlay::view)
            .theme(Overlay::theme)
            .subscription(Overlay::subscription)
            .style(style)
            .settings(Settings {
                id: Some(crate::APP_ID.to_owned()),
                layer_settings: LayerShellSettings {
                    start_mode: StartMode::Background,
                    ..LayerShellSettings::default()
                },
                antialiasing: true,
                ..Settings::default()
            })
            .run()
            .map_err(|error| error.to_string()),
        Host::Window | Host::Shell => iced::daemon(boot, Overlay::update, Overlay::view)
            .antialiasing(true)
            .title(Overlay::title)
            .theme(Overlay::theme)
            .subscription(Overlay::subscription)
            .style(style)
            .run()
            .map_err(|error| error.to_string()),
    };
    ran?;
    Ok(outcome
        .lock()
        .map(|outcome| outcome.clone())
        .unwrap_or_default())
}
