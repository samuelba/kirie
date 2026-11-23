//! The on-screen mode: every screen frozen on a surface of its own, where the user picks a
//! region, marks it up, and copies or saves it.

mod canvas;
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

use crate::drawing::Drawing;
use crate::picker::{self, Picker, Target};
use crate::selection::{self, Release, Selection};
use crate::swatches::Swatches;
use crate::typing::{self, Typing};

pub const NAMESPACE: &str = "kirie-overlay";
/// How close to a handle a press grabs it, in logical pixels.
const HANDLE_REACH: f32 = 8.0;
/// How close to a mark the select tool picks it, in logical pixels.
const MARK_REACH: f64 = 6.0;
/// How long the flash that tells the screen was taken takes to fade.
const FLASH: std::time::Duration = std::time::Duration::from_millis(300);
/// How bright the flash starts, as the opacity of white over the screens.
const FLASH_START: f32 = 0.8;

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
    PickStyle(StyleId),
    ToggleShadow,
    PickColor(kirie_annotate::Rgba),
    /// Opens the color picker on a swatch, or to add one.
    EditColor(Target),
    Picker(picker::Event),
    Weight(crate::controls::WeightEvent),
    ToggleFill,
    Brightness(crate::controls::BrightnessEvent),
    Spot(kirie_annotate::SpotShape),
    Look(crate::controls::TextEvent),
    TextEdit(iced::widget::text_editor::Action),
    TextDone,
    Undo,
    Redo,
    Delete,
    /// Copies the picked marks a step aside.
    Duplicate,
    /// Moves the picked marks up or down among the others.
    Restack(crate::marks::Restack),
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
    modifiers: keyboard::Modifiers,
    palette: Palette,
    tool: Option<Tool>,
    paint: Paint,
    swatches: Swatches,
    picker: Option<Picker>,
    history: History,
    draft: Option<Draft>,
    /// The tool the current pointer gesture draws with, if it draws at all.
    drawing: Option<Tool>,
    /// The marks the select tool picked; a lone one shows its grips.
    picked: Vec<kirie_annotate::ShapeId>,
    /// A slider is being dragged; its moves undo as one step.
    sliding: bool,
    typing: Option<Typing>,
    /// Where the toolbar was dragged to, in desktop coordinates.
    toolbar_at: Option<Point>,
    /// While the toolbar is dragged: where it is held, from its top left.
    toolbar_drag: Option<Vector>,
    /// The pointer over the grip, from the grip's top left.
    grip_at: Point,
    /// The last press, to tell a double click.
    last_click: Option<iced::advanced::mouse::Click>,
    busy: bool,
    /// What Enter does.
    primary: Action,
    host: Host,
    /// The flash as the overlay opens, while it fades.
    flash: Option<Flash>,
    typesetter: RefCell<Typesetter>,
    patches: RefCell<HashMap<crate::paint::PatchKey, image::Handle>>,
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
        let remembered =
            crate::region::load().and_then(|region| crate::region::within(region, desktop));
        let overlay = Self {
            windows,
            captures: frames,
            screens,
            surfaces,
            desktop,
            selection: remembered.map(Selection::selected).unwrap_or_default(),
            remembered,
            pointer: None,
            modifiers: keyboard::Modifiers::default(),
            palette: Palette::DARK,
            tool: None,
            paint: drawing.paint_for(None, &swatches),
            swatches,
            picker: None,
            history: History::new(document),
            draft: None,
            drawing: None,
            picked: Vec::new(),
            sliding: false,
            typing: None,
            toolbar_at: None,
            toolbar_drag: None,
            grip_at: Point::ORIGIN,
            last_click: None,
            busy: false,
            primary,
            host,
            flash: Some(Flash::new()),
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
            self.history.apply(Op::Add {
                kind,
                paint: self.paint,
            });
        }
        let first = self.document().shapes().next().cloned();
        if let Some(first) = first {
            self.history.apply(Op::Reshape {
                id: first.id,
                kind: first.kind,
                rotation: -0.12,
            });
            self.picked = vec![first.id];
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
                self.modifiers = modifiers;
                Task::none()
            }
            Message::PickTool(tool) => {
                self.commit_text();
                if tool != Some(Tool::Select) {
                    self.picked.clear();
                }
                self.tool = tool;
                self.paint.color = crate::controls::tool_color(
                    tool,
                    self.paint.color,
                    self.swatches.first(self.style()),
                );
                self.draft = None;
                Task::none()
            }
            Message::PickStyle(_)
            | Message::ToggleShadow
            | Message::PickColor(_)
            | Message::EditColor(_)
            | Message::Picker(_)
            | Message::Weight(_)
            | Message::ToggleFill
            | Message::Brightness(_)
            | Message::Spot(_)
            | Message::Look(_) => {
                self.restyle(message);
                Task::none()
            }
            Message::TextEdit(action) => {
                if let Some(typing) = &mut self.typing {
                    typing.content.perform(action);
                }
                Task::none()
            }
            Message::TextDone => {
                self.commit_text();
                Task::none()
            }
            Message::Undo => {
                self.travel(true);
                Task::none()
            }
            Message::Redo => {
                self.travel(false);
                Task::none()
            }
            Message::Delete | Message::Duplicate | Message::Restack(_) => {
                self.edit_marks(&message);
                Task::none()
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

    /// Deleting, duplicating and restacking the picked marks.
    fn edit_marks(&mut self, message: &Message) {
        match message {
            Message::Delete if !self.picked.is_empty() => {
                let ids = std::mem::take(&mut self.picked);
                self.history.apply(Op::Remove { ids });
            }
            Message::Duplicate => {
                if let Some((op, ids)) = crate::marks::duplicate(self.document(), &self.picked) {
                    self.history.apply(op);
                    self.picked = ids;
                }
            }
            Message::Restack(to) => {
                if let Some(op) = crate::marks::restack(self.document(), &self.picked, *to) {
                    self.history.apply(op);
                }
            }
            _ => {}
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
            crate::region::forget();
        } else if let Some(region) = self.selection.rect() {
            crate::region::remember(region);
            self.remembered = Some(region);
        }
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
            crate::region::forget();
        }
    }

    /// Changes to the style, colors, weight, fill and text look, kept for the next time.
    fn restyle(&mut self, message: Message) {
        let settled = settled(&message);
        self.change_style(message);
        if settled {
            Drawing::load()
                .with_marks(self.paint, self.document())
                .save();
        }
    }

    fn change_style(&mut self, message: Message) {
        match message {
            Message::PickStyle(style) => {
                let before = self.style();
                let recolor = self.swatches.recolor(before, style);
                self.history.apply(Op::SetStyle { style, recolor });
                self.follow_style(before);
            }
            Message::ToggleShadow => {
                let shadow = !self.document().shadow;
                self.history.apply(Op::SetShadow(shadow));
            }
            Message::PickColor(color) => {
                self.repaint(|paint| paint.color = color);
            }
            Message::EditColor(target) => {
                self.picker = Some(Picker::new(
                    target,
                    self.style(),
                    &self.swatches,
                    self.paint.color,
                ));
            }
            Message::Picker(event) => {
                if let picker::Outcome::Changed { pick } =
                    picker::update(&mut self.picker, &mut self.swatches, event)
                {
                    if let Some(color) = pick {
                        self.repaint(|paint| paint.color = color);
                    }
                    if let Err(error) = self.swatches.save() {
                        tracing::warn!(%error, "colors not saved");
                    }
                }
            }
            Message::Weight(crate::controls::WeightEvent::Done)
            | Message::Brightness(crate::controls::BrightnessEvent::Done)
            | Message::Look(crate::controls::TextEvent::SizeDone) => self.sliding = false,
            Message::Brightness(crate::controls::BrightnessEvent::Slide(brightness)) => {
                self.history
                    .apply_gradually(Op::SetSpotlightBrightness(brightness), self.sliding);
                self.sliding = true;
            }
            Message::Weight(crate::controls::WeightEvent::Pick(weight)) => {
                self.repaint(|paint| paint.weight = weight);
            }
            Message::Weight(crate::controls::WeightEvent::Slide(width)) => {
                self.repaint_gradually(|paint| paint.weight = Weight::new(width), true);
            }
            Message::ToggleFill => {
                self.repaint(|paint| paint.filled = !paint.filled);
            }
            Message::Spot(spot) => self.repaint(|paint| paint.spot = spot),
            Message::Look(event) => {
                self.repaint_gradually(
                    |paint| paint.text = event.apply(paint.text),
                    event.gradual(),
                );
            }
            _ => {}
        }
    }

    /// Undoes or redoes a step; the drawing color follows the style there.
    fn travel(&mut self, undo: bool) {
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

    /// Whether the text controls apply: to text being typed, picked text marks, or the next.
    fn texting(&self) -> bool {
        self.tool == Some(Tool::Text)
            || self.typing.is_some()
            || crate::marks::all(self.document(), &self.picked, |kind| {
                matches!(kind, Kind::Text { .. })
            })
    }

    /// Whether the spotlight controls apply: to the spotlight tool or picked spotlights.
    fn spotlighting(&self) -> bool {
        self.tool == Some(Tool::Spotlight)
            || crate::marks::all(self.document(), &self.picked, |kind| {
                matches!(kind, Kind::Spotlight { .. })
            })
    }

    /// Whether a press adds to the picked marks instead of picking anew.
    fn extending(&self) -> bool {
        self.modifiers.shift() || self.modifiers.command()
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
                self.remembered = crate::region::load()
                    .and_then(|region| crate::region::within(region, self.desktop));
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
        if self.sampling() {
            self.change_style(Message::Picker(picker::Event::Eyedropper));
            return Task::none();
        }
        if self.picker.take().is_some()
            || self.typing.take().is_some()
            || self.draft.take().is_some()
            || !std::mem::take(&mut self.picked).is_empty()
        {
            return Task::none();
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
        if self.sampling() {
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

    /// Whether the eyedropper waits for a click on the screens.
    fn sampling(&self) -> bool {
        self.picker.as_ref().is_some_and(Picker::sampling)
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
        self.change_style(Message::Picker(picker::Event::Sampled(
            kirie_annotate::Rgba::rgb(r, g, b),
        )));
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
        let [id] = self.picked[..] else {
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
        match pointer {
            Pointer::Pressed(at) => {
                self.commit_text();
                let double = typing::double_click(&mut self.last_click, at);
                let words = match tool {
                    Tool::Text => typing::text_at(self.document(), to_kurbo(at), MARK_REACH),
                    Tool::Select if double => {
                        typing::text_at(self.document(), to_kurbo(at), MARK_REACH)
                    }
                    _ => None,
                };
                if let Some(id) = words {
                    return self.edit_text(id);
                }
                let extend = self.extending();
                let press = kirie_annotate::tools::press(
                    tool,
                    to_kurbo(at),
                    self.document(),
                    self.paint,
                    MARK_REACH,
                    &self.picked,
                    extend,
                );
                match press {
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
                    Press::Toggle(id) => crate::marks::toggle(&mut self.picked, id),
                    Press::Nothing => self.picked.clear(),
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
    fn edit_text(&mut self, id: kirie_annotate::ShapeId) -> Task<Message> {
        let Some(typing) = self.document().shape(id).and_then(Typing::edit) else {
            return Task::none();
        };
        self.typing = Some(typing);
        self.picked = vec![id];
        self.draft = None;
        iced::widget::operation::focus(view::TEXT_INPUT)
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
            Action::Save => Task::perform(crate::save::save_and_copy(picture), |saved| {
                Message::Finished(saved.map(|(path, copied)| Outcome::Saved { path, copied }))
            }),
            Action::Copy => Task::perform(crate::clipboard::copy(picture), |copied| {
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
        let keys = if self.typing.is_some() || self.picker.is_some() {
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

/// Whether a change is not one step of a slider drag, whose last step follows.
fn settled(message: &Message) -> bool {
    !matches!(
        message,
        Message::Weight(crate::controls::WeightEvent::Slide(_))
            | Message::Brightness(crate::controls::BrightnessEvent::Slide(_))
            | Message::Look(crate::controls::TextEvent::Size(_))
    )
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
        && let Some(to) = crate::marks::restack_key(written, modifiers.shift())
    {
        return Some(Message::Restack(to));
    }
    match key.as_ref() {
        keyboard::Key::Named(Named::Escape) => Some(Message::Escape),
        keyboard::Key::Named(Named::Enter) if modifiers.shift() || command => {
            Some(Message::Finish(Action::Save))
        }
        keyboard::Key::Named(Named::Enter) => Some(Message::Accept),
        keyboard::Key::Named(Named::Delete | Named::Backspace) => Some(Message::Delete),
        keyboard::Key::Named(Named::ArrowLeft) => Some(Message::Nudge(Vector::new(-step, 0.0))),
        keyboard::Key::Named(Named::ArrowRight) => Some(Message::Nudge(Vector::new(step, 0.0))),
        keyboard::Key::Named(Named::ArrowUp) => Some(Message::Nudge(Vector::new(0.0, -step))),
        keyboard::Key::Named(Named::ArrowDown) => Some(Message::Nudge(Vector::new(0.0, step))),
        keyboard::Key::Character("z") if command && modifiers.shift() => Some(Message::Redo),
        keyboard::Key::Character("z") if command => Some(Message::Undo),
        keyboard::Key::Character("y") if command => Some(Message::Redo),
        keyboard::Key::Character("a") if command => Some(Message::SelectAll),
        keyboard::Key::Character("s") if command && modifiers.shift() => {
            Some(Message::Finish(Action::SaveAs))
        }
        keyboard::Key::Character("s") if command => Some(Message::Finish(Action::Save)),
        keyboard::Key::Character("c") if command => Some(Message::Finish(Action::Copy)),
        keyboard::Key::Character("e") if command => Some(Message::Finish(Action::Edit)),
        keyboard::Key::Character("d") if command => Some(Message::Duplicate),
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

#[cfg(test)]
mod tests {
    use super::*;

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
