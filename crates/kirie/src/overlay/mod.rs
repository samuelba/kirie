//! The on-screen mode: every screen frozen on a surface of its own, where the user picks a
//! region, marks it up, and copies or saves it.

mod canvas;
pub mod region;
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
use kirie_annotate::{Document, Kind, Op, StyleId, Tool, Typesetter};
use kirie_capture::{Frame, geometry};
use kirie_ui::Palette;

use crate::annotate::annotator::{Annotator, Change, Effect};
use crate::annotate::drawing::Drawing;
use crate::annotate::picker;
use crate::annotate::selection::{self, Release, Selection};
use crate::annotate::swatches::Swatches;

pub const NAMESPACE: &str = "kirie-overlay";
/// How close to a handle a press grabs it, in logical pixels.
const HANDLE_REACH: f32 = 8.0;
/// How close to a mark the select tool picks it, in logical pixels.
const MARK_REACH: f64 = 6.0;
/// How long the flash that tells the screen was taken takes to fade.
const FLASH: std::time::Duration = std::time::Duration::from_millis(200);
/// How bright the flash starts, as the opacity of white over the screens.
const FLASH_START: f32 = 0.35;

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
    /// Closes the overlay and asks where to save, and as what.
    SaveAs,
    /// Hands the selection and its marks to the editor.
    Edit,
}

#[derive(Debug, Clone)]
pub enum Message {
    Pointer(Pointer),
    Modifiers(keyboard::Modifiers),
    PickTool(Option<Tool>),
    /// An edit the editor offers too.
    Annotate(Change),
    Escape,
    Finish(Action),
    /// Enter: the action the overlay was opened for.
    Accept,
    SelectAll,
    /// Selects the screen with this index.
    SelectScreen(usize),
    Nudge(Vector),
    /// Keeps the selection for the next screenshot, or forgets it.
    ToggleRemember,
    /// Shows or hides the magnifier while a region is picked, for this and later screenshots.
    ToggleMagnifier,
    Finished(Result<Outcome, String>),
    Resized(window::Id, Size),
    /// The pointer over the toolbar's grip, in the grip's coordinates.
    GripHover(Point),
    ToolbarGrab,
    /// The pointer while the toolbar is dragged, in the coordinates of a surface.
    ToolbarMoved(window::Id, Point),
    ToolbarDropped,
    Layer(LayerShellCustomActionWithId),
    /// Time passes while the flash fades.
    Flash(std::time::Instant),
}

/// The flash over the screens as the overlay opens, like a camera's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Flash {
    /// When it started fading: at the first tick after the surfaces showed.
    since: Option<std::time::Instant>,
    /// The opacity of the white over the screens.
    level: f32,
}

impl Flash {
    fn new() -> Self {
        Self {
            since: None,
            level: FLASH_START,
        }
    }

    /// The flash at `now`, or `None` once it has faded.
    fn at(self, now: std::time::Instant) -> Option<Self> {
        let since = self.since.unwrap_or(now);
        let done = now.saturating_duration_since(since).as_secs_f32() / FLASH.as_secs_f32();
        (done < 1.0).then(|| Self {
            since: Some(since),
            level: FLASH_START * (1.0 - done).powi(2),
        })
    }
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
    Canceled,
    /// The picture was saved here, and put on the clipboard too if `copied`.
    Saved {
        path: PathBuf,
        copied: bool,
    },
    /// The picture and its marks, to save where the user picks once the overlay is gone.
    SaveAs(Box<kirie_annotate::project::Project>),
    Copied,
    /// A project for the editor to open.
    Edit(PathBuf),
    Failed(String),
}

pub struct Overlay {
    /// The windows on screen, topmost first, with their titles.
    windows: Vec<(Rectangle, String)>,
    captures: Vec<Frame>,
    screens: Vec<Screen>,
    surfaces: HashMap<window::Id, usize>,
    desktop: Rectangle,
    selection: Selection,
    /// The selection kept for the next screenshot, while it is unchanged.
    remembered: Option<Rectangle>,
    pointer: Option<Point>,
    palette: Palette,
    tool: Option<Tool>,
    annotator: Annotator,
    /// The tool the current pointer gesture draws with, if it draws at all.
    drawing: Option<Tool>,
    /// Where the toolbar was dragged to, in desktop coordinates.
    toolbar_at: Option<Point>,
    /// While the toolbar is dragged: where it is held, from its top left.
    toolbar_drag: Option<Vector>,
    /// The pointer over the grip, from the grip's top left.
    grip_at: Point,
    busy: bool,
    /// What Enter does.
    primary: Action,
    host: Host,
    /// The flash as the overlay opens, while it fades.
    flash: Option<Flash>,
    /// The magnifier shows while a region is picked.
    magnifier: bool,
    typesetter: RefCell<Typesetter>,
    patches: RefCell<HashMap<crate::annotate::paint::PatchKey, image::Handle>>,
    pictures: RefCell<HashMap<u64, image::Handle>>,
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

/// The frozen screens, ready to show.
fn screens(frames: &[Frame]) -> Vec<Screen> {
    frames
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
        .collect()
}

fn to_kurbo(point: Point) -> kurbo::Point {
    kurbo::Point::new(f64::from(point.x), f64::from(point.y))
}

impl Overlay {
    pub fn boot(
        frames: Vec<Frame>,
        windows: Vec<kirie_capture::windows::Window>,
        primary: Action,
        host: Host,
        outcome: Arc<Mutex<Outcome>>,
    ) -> (Self, Task<Message>) {
        let screens = screens(&frames);
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
        let drawing = Drawing::load();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let document = drawing.styled(Document::new(
            (desktop.x + desktop.width).max(1.0) as u32,
            (desktop.y + desktop.height).max(1.0) as u32,
            1.0,
        ));
        let windows = windows
            .into_iter()
            .map(|window| (area(window.rect), window.title))
            .collect();
        let swatches = Swatches::load();
        let paint = drawing.paint_for(None, &swatches);
        let remembered = crate::overlay::region::load()
            .and_then(|region| crate::overlay::region::within(region, desktop));
        let overlay = Self {
            windows,
            captures: frames,
            screens,
            surfaces,
            desktop,
            selection: remembered.map(Selection::selected).unwrap_or_default(),
            remembered,
            pointer: None,
            palette: Palette::DARK,
            tool: None,
            annotator: Annotator::new(document, paint, swatches),
            drawing: None,
            toolbar_at: None,
            toolbar_drag: None,
            grip_at: Point::ORIGIN,
            busy: false,
            primary,
            host,
            flash: Some(Flash::new()),
            magnifier: crate::storage::settings::Settings::load().magnifier,
            typesetter: RefCell::new(Typesetter::new()),
            patches: RefCell::new(HashMap::new()),
            pictures: RefCell::new(HashMap::new()),
            outcome,
        };
        let overlay = if std::env::var_os("KIRIE_OVERLAY_DEMO").is_some() {
            overlay.with_demo()
        } else {
            overlay
        };
        (overlay, Task::batch(open))
    }

    /// Whether the overlay is one window on one of several screens: without layer-shell a
    /// window cannot pick its screen, so the others cannot be shown.
    fn covers_one_of_many(&self) -> bool {
        self.host == Host::Window && self.screens.len() > 1
    }

    /// A selection with a few marks, for screenshots of the overlay and the smoke test.
    fn with_demo(mut self) -> Self {
        // A screenshot of the overlay shows it as it stays, not as it opens.
        self.flash = None;
        let Some(screen) = self.screens.first() else {
            return self;
        };
        let area = screen.area;
        let region = Rectangle::new(
            Point::new(area.x + area.width * 0.2, area.y + area.height * 0.15),
            Size::new(area.width * 0.6, area.height * 0.5),
        );
        self.selection = Selection::selected(region);
        self.remembered = None;
        self.tool = Some(Tool::Select);
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
            self.annotator.history.apply(Op::Add {
                kind,
                paint: self.annotator.paint,
            });
        }
        let first = self.document().shapes().next().cloned();
        if let Some(first) = first {
            self.annotator.history.apply(Op::Reshape {
                id: first.id,
                kind: first.kind,
                rotation: -0.12,
            });
            self.annotator.picked = vec![first.id];
        }
        self
    }

    fn document(&self) -> &Document {
        self.annotator.history.current()
    }

    fn style(&self) -> StyleId {
        self.document().style
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(pointer) if self.toolbar_drag.is_some() => {
                if let Pointer::Released(_) = pointer {
                    self.drag_toolbar(&Message::ToolbarDropped);
                }
                Task::none()
            }
            Message::Pointer(pointer) => self.pointer(pointer),
            Message::GripHover(_)
            | Message::ToolbarGrab
            | Message::ToolbarMoved(..)
            | Message::ToolbarDropped => {
                self.drag_toolbar(&message);
                Task::none()
            }
            Message::Modifiers(modifiers) => {
                self.annotator.modifiers = modifiers;
                Task::none()
            }
            Message::PickTool(tool) => {
                self.annotator.pick_tool(tool);
                self.tool = tool;
                Task::none()
            }
            Message::Annotate(change) => {
                let effect = self.annotator.update(change);
                self.annotated(effect)
            }
            Message::Escape => self.escape(),
            Message::Nudge(by) => {
                self.selection = self.selection.nudge(by, self.desktop);
                self.keep_remembered();
                Task::none()
            }
            Message::SelectAll => self.select_area(self.desktop),
            Message::SelectScreen(index) => match self.screens.get(index) {
                Some(screen) => self.select_area(screen.area),
                None => Task::none(),
            },
            Message::ToggleRemember => {
                self.toggle_remember();
                Task::none()
            }
            Message::ToggleMagnifier => self.toggle_magnifier(),
            Message::Finish(action) => self.finish(action),
            Message::Accept => self.finish(self.primary),
            Message::Finished(Ok(outcome)) => self.close(outcome),
            Message::Finished(Err(error)) => self.close(Outcome::Failed(error)),
            Message::Resized(id, size) => {
                self.place(id, size);
                Task::none()
            }
            Message::Layer(_) => Task::none(),
            Message::Flash(now) => {
                self.flash = self.flash.and_then(|flash| flash.at(now));
                Task::none()
            }
        }
    }

    /// Moving the toolbar by its grip.
    fn drag_toolbar(&mut self, message: &Message) {
        match *message {
            Message::GripHover(at) => self.grip_at = at,
            Message::ToolbarGrab => {
                if let Some((at, _)) = self.toolbar_place() {
                    self.toolbar_at = Some(at);
                    self.toolbar_drag = Some(Vector::new(
                        self.grip_at.x + kirie_ui::space::S,
                        self.grip_at.y + kirie_ui::space::S,
                    ));
                }
            }
            Message::ToolbarMoved(id, at) => {
                let surface = self
                    .surfaces
                    .get(&id)
                    .map(|&index| self.screens[index].area);
                if let (Some(held), Some(surface)) = (self.toolbar_drag, surface) {
                    let desktop = Point::new(at.x + surface.x, at.y + surface.y);
                    self.toolbar_at = Some(desktop - held);
                }
            }
            Message::ToolbarDropped => {
                self.toolbar_drag = None;
                self.toolbar_at = self.toolbar_place().map(|(at, _)| at);
            }
            _ => {}
        }
    }

    fn toggle_remember(&mut self) {
        if self.remembered.take().is_some() {
            crate::overlay::region::forget();
        } else if let Some(region) = self.selection.rect() {
            crate::overlay::region::remember(region);
            self.remembered = Some(region);
        }
    }

    /// Keeps the choice for later screenshots, on top of whatever else was set meanwhile.
    fn toggle_magnifier(&mut self) -> Task<Message> {
        self.magnifier = !self.magnifier;
        let settings = crate::storage::settings::Settings {
            magnifier: self.magnifier,
            ..crate::storage::settings::Settings::load()
        };
        if let Err(error) = settings.save() {
            tracing::warn!(%error, "magnifier choice not kept");
        }
        Task::none()
    }

    /// Forgets the remembered region once the selection is changed.
    fn select_area(&mut self, area: Rectangle) -> Task<Message> {
        self.selection = Selection::selected(area);
        self.keep_remembered();
        Task::none()
    }

    fn keep_remembered(&mut self) {
        if self
            .remembered
            .is_some_and(|region| self.selection.rect() != Some(region))
        {
            self.remembered = None;
            crate::overlay::region::forget();
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
                // A remembered region on another screen stays remembered for when it fits.
                self.remembered = crate::overlay::region::load()
                    .and_then(|region| crate::overlay::region::within(region, self.desktop));
                self.selection = self.remembered.map(Selection::selected).unwrap_or_default();
            }
        }
    }

    fn escape(&mut self) -> Task<Message> {
        if self.toolbar_drag.is_some() {
            self.drag_toolbar(&Message::ToolbarDropped);
            return Task::none();
        }
        // A save or copy on its way finishes first; leaving now would drop it half done.
        if self.busy {
            return Task::none();
        }
        if self.annotator.sampling() {
            let effect = self.annotator.edit_color(picker::Event::Eyedropper);
            return self.annotated(effect);
        }
        if let Some(effect) = self.annotator.put_down() {
            return self.annotated(effect);
        }
        if self.tool.take().is_some() {
            return Task::none();
        }
        self.close(Outcome::Canceled)
    }

    fn pointer(&mut self, pointer: Pointer) -> Task<Message> {
        let at = match pointer {
            Pointer::Pressed(at) | Pointer::Moved(at) | Pointer::Released(at) => at,
        };
        self.pointer = Some(at);
        if self.annotator.sampling() {
            if let Pointer::Pressed(at) = pointer {
                self.sample(at);
            }
            return Task::none();
        }
        // A gesture stays with what its press started, whatever happens on the way.
        if let Pointer::Pressed(at) = pointer {
            let inside = self.selection.hit(at, HANDLE_REACH) == Some(selection::Hit::Inside)
                || self.on_grip(to_kurbo(at));
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

    /// Hands the picker the frozen screens' color at `at`.
    fn sample(&mut self, at: Point) {
        let Some(capture) = self
            .captures
            .iter()
            .find(|capture| area(capture.output.logical).contains(at))
        else {
            return;
        };
        let logical = capture.output.logical;
        let scale = capture.scale();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pixel = |at: f32, from: i32| ((f64::from(at) - f64::from(from)) * scale).floor() as u32;
        let Some(&::image::Rgba([r, g, b, _])) = capture
            .image
            .get_pixel_checked(pixel(at.x, logical.x), pixel(at.y, logical.y))
        else {
            return;
        };
        let effect = self
            .annotator
            .edit_color(picker::Event::Sampled(kirie_annotate::Rgba::rgb(r, g, b)));
        let _ = self.annotated(effect);
    }

    /// The topmost window under `at`.
    fn window_at(&self, at: Point) -> Option<Rectangle> {
        self.windows
            .iter()
            .find(|(area, _)| area.contains(at))
            .map(|(area, _)| *area)
    }

    /// Whether `at` is on a grip of the lone picked mark.
    fn on_grip(&self, at: kurbo::Point) -> bool {
        let [id] = self.annotator.picked[..] else {
            return false;
        };
        self.tool == Some(Tool::Select)
            && self.document().shape(id).is_some_and(|shape| {
                kirie_annotate::edit::grip_at(shape, at, MARK_REACH, 1.0).is_some()
            })
    }

    fn select(&mut self, pointer: Pointer) {
        match pointer {
            Pointer::Pressed(at) => self.selection = self.selection.press(at, HANDLE_REACH),
            Pointer::Moved(at) => self.selection = self.selection.drag_to(at, self.desktop),
            Pointer::Released(at) => {
                let (selection, release) = self.selection.release(at);
                self.selection = selection;
                if let Release::Click(at) = release {
                    let picked = self
                        .window_at(at)
                        .and_then(|window| window.intersection(&self.desktop))
                        .or_else(|| {
                            self.screens
                                .iter()
                                .find(|screen| screen.area.contains(at))
                                .map(|screen| screen.area)
                        });
                    if let Some(area) = picked {
                        self.selection = Selection::selected(area);
                    }
                }
            }
        }
        self.keep_remembered();
    }

    fn annotate(&mut self, tool: Tool, pointer: Pointer) -> Task<Message> {
        let effect = match pointer {
            Pointer::Pressed(at) => self.annotator.press(tool, to_kurbo(at), at, MARK_REACH),
            Pointer::Moved(at) => {
                self.annotator.drag(to_kurbo(at));
                return Task::none();
            }
            Pointer::Released(at) => self.annotator.release(to_kurbo(at)),
        };
        self.annotated(effect)
    }

    /// Acts on what an edit did: the text box takes the keyboard, the drawing options are kept
    /// for the next time, and colors that could not be kept are logged.
    fn annotated(&mut self, effect: Effect) -> Task<Message> {
        if effect.settled {
            Drawing::load()
                .with_marks(self.annotator.paint, self.document())
                .save();
        }
        if let Some(error) = effect.error {
            tracing::warn!(%error, "colors not saved");
        }
        if effect.focus {
            iced::widget::operation::focus(view::TEXT_INPUT)
        } else {
            Task::none()
        }
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
        self.annotator.commit_text();
        if self.busy || self.selection.rect().is_none() {
            return Task::none();
        }
        self.busy = true;
        if action == Action::Edit {
            let handed = self
                .project()
                .and_then(|project| crate::storage::save::save_handoff(&project));
            return match handed {
                Ok(path) => self.close(Outcome::Edit(path)),
                Err(error) => self.close(Outcome::Failed(error)),
            };
        }
        // The file dialog would open under the overlay, so it waits for the overlay to close.
        if action == Action::SaveAs {
            return match self.project() {
                Ok(project) => self.close(Outcome::SaveAs(Box::new(project))),
                Err(error) => self.close(Outcome::Failed(error)),
            };
        }
        let picture = match self.picture() {
            Ok(picture) => picture,
            Err(error) => return self.close(Outcome::Failed(error)),
        };
        match action {
            Action::Save => Task::perform(
                crate::storage::save::save_and_copy(picture, None),
                |saved| {
                    Message::Finished(saved.map(|(path, copied)| Outcome::Saved { path, copied }))
                },
            ),
            Action::Copy => Task::perform(crate::desktop::clipboard::copy(picture), |copied| {
                Message::Finished(copied.map(|()| Outcome::Copied))
            }),
            Action::Edit | Action::SaveAs => Task::none(),
        }
    }

    fn close(&mut self, outcome: Outcome) -> Task<Message> {
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = outcome;
        }
        if self.host == Host::Layer {
            return iced::exit();
        }
        // GNOME 46 can leave a window on screen when its process ends before the compositor
        // has seen it closed.
        Task::batch(self.surfaces.keys().copied().map(window::close))
            .chain(Task::future(tokio::time::sleep(TEARDOWN)).discard())
            .chain(iced::exit())
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let keys = if self.annotator.typing.is_some() || self.annotator.picker.is_some() {
            event::listen_with(typing_key)
        } else {
            event::listen_with(key)
        };
        let toolbar = if self.toolbar_drag.is_some() {
            event::listen_with(dragging_toolbar)
        } else {
            Subscription::none()
        };
        let flash = if self.flash.is_some() {
            iced::time::every(std::time::Duration::from_millis(16)).map(Message::Flash)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            keys,
            toolbar,
            flash,
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

/// How long the closed overlay windows get to reach the compositor before the process ends.
const TEARDOWN: std::time::Duration = std::time::Duration::from_millis(100);

fn window_settings() -> window::Settings {
    window::Settings {
        decorations: false,
        fullscreen: true,
        // A window of fixed size stays that size when GNOME 46 makes it fullscreen.
        resizable: true,
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
fn dragging_toolbar(event: Event, _: event::Status, id: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::ToolbarMoved(id, position))
        }
        Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(Message::ToolbarDropped)
        }
        _ => None,
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
    if command
        && let keyboard::Key::Character(written) = key.as_ref()
        && let Some(to) = kirie_annotate::marks::restack_key(written)
    {
        return Some(Message::Annotate(Change::Restack(to)));
    }
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Enter) if modifiers.shift() || command => {
            Some(Message::Finish(Action::Save))
        }
        keyboard::Key::Named(Named::Enter) => Some(Message::Accept),
        keyboard::Key::Named(Named::Delete | Named::Backspace) => {
            Some(Message::Annotate(Change::Delete))
        }
        keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Nudge(Vector::new(-step, 0.0))),
        keyboard::Key::Named(Named::ArrowRight) => Some(Message::Nudge(Vector::new(step, 0.0))),
        keyboard::Key::Named(Named::ArrowUp) => Some(Message::Nudge(Vector::new(0.0, -step))),
        keyboard::Key::Named(Named::ArrowDown) => Some(Message::Nudge(Vector::new(0.0, step))),
        keyboard::Key::Character("z") if command && modifiers.shift() => {
            Some(Message::Annotate(Change::Redo))
        }
        keyboard::Key::Character("z") if command => Some(Message::Annotate(Change::Undo)),
        keyboard::Key::Character("y") if command => Some(Message::Annotate(Change::Redo)),
        keyboard::Key::Character("a") if command => Some(Message::SelectAll),
        keyboard::Key::Character("s") if command && modifiers.shift() => {
            Some(Message::Finish(Action::SaveAs))
        }
        keyboard::Key::Character("s") if command => Some(Message::Finish(Action::Save)),
        keyboard::Key::Character("c") if command => Some(Message::Finish(Action::Copy)),
        keyboard::Key::Character("e") if command => Some(Message::Finish(Action::Edit)),
        keyboard::Key::Character("d") if command => Some(Message::Annotate(Change::Duplicate)),
        keyboard::Key::Character("m" | "M") if !command => Some(Message::ToggleMagnifier),
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
pub fn run(
    frames: Vec<Frame>,
    windows: Vec<kirie_capture::windows::Window>,
    primary: Action,
    host: Host,
) -> Result<Outcome, String> {
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    let outcome = Arc::new(Mutex::new(Outcome::Canceled));
    let shared = outcome.clone();
    let start = Mutex::new(Some((frames, windows)));
    let boot = move || {
        let (frames, windows) = start
            .lock()
            .ok()
            .and_then(|mut start| start.take())
            .unwrap_or_default();
        Overlay::boot(frames, windows, primary, host, shared.clone())
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
                fonts: vec![kirie_annotate::text::EMOJI_FONT.into()],
                ..Settings::default()
            })
            .run()
            .map_err(|error| error.to_string()),
        Host::Window | Host::Shell => iced::daemon(boot, Overlay::update, Overlay::view)
            .antialiasing(true)
            .font(kirie_annotate::text::EMOJI_FONT)
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

/// `change` as the message the views send for it.
fn annotate<T>(change: impl Fn(T) -> Change + Clone) -> impl Fn(T) -> Message + Clone {
    move |value| Message::Annotate(change(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_key_picks_its_tool() {
        let id = window::Id::unique();
        for tool in Tool::ALL {
            let letter = tool.key().to_string();
            let pressed = Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(letter.as_str().into()),
                modified_key: keyboard::Key::Character(letter.as_str().into()),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: None,
                repeat: false,
            });
            assert!(
                matches!(
                    key(pressed, event::Status::Ignored, id),
                    Some(Message::PickTool(Some(picked))) if picked == tool
                ),
                "{letter} picks {tool:?}"
            );
        }
    }

    #[test]
    fn the_flash_fades_from_its_first_tick() {
        let start = std::time::Instant::now();
        let first = Flash::new().at(start + FLASH * 5).unwrap();
        assert!(
            (first.level - FLASH_START).abs() < 1e-6,
            "a late first tick starts the fade"
        );
        let half = first.at(start + FLASH * 5 + FLASH / 2).unwrap();
        assert!(half.level < first.level && half.level > 0.0);
        assert!(first.at(start + FLASH * 6).is_none());
    }
}
