//! The on-screen mode: every screen frozen on a surface of its own, where the user picks a
//! region (or a whole screen) and keeps it.

mod canvas;
mod selection;

use std::collections::HashMap;
use std::path::PathBuf;

use iced::widget::{image, stack};
use iced::{Element, Length, Point, Rectangle, Size, Subscription, Task, Vector, event, keyboard};
use iced::{Event, window};
use iced_layershell::actions::{LayerShellCustomAction, LayerShellCustomActionWithId};
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use kirie_capture::{Frame, geometry};
use kirie_ui::Palette;

use self::selection::{Release, Selection};

pub const NAMESPACE: &str = "kirie-overlay";
/// How close to a handle a press grabs it, in logical pixels.
const HANDLE_REACH: f32 = 8.0;

/// One frozen screen.
#[derive(Debug)]
struct Screen {
    frame: Frame,
    handle: image::Handle,
    area: Rectangle,
}

#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Pressed(Point),
    Moved(Point),
    Released(Point),
}

#[derive(Debug, Clone)]
pub enum Message {
    Pointer(Pointer),
    Accept,
    Cancel,
    SelectAll,
    Nudge(Vector),
    Saved(Result<PathBuf, String>),
    Layer(LayerShellCustomActionWithId),
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
    Failed(String),
}

#[derive(Debug)]
pub struct Overlay {
    screens: Vec<Screen>,
    surfaces: HashMap<window::Id, usize>,
    desktop: Rectangle,
    selection: Selection,
    pointer: Option<Point>,
    palette: Palette,
    busy: bool,
    outcome: std::sync::Arc<std::sync::Mutex<Outcome>>,
}

fn area(rect: geometry::Rect) -> Rectangle {
    #[allow(clippy::cast_precision_loss)]
    Rectangle::new(
        Point::new(rect.x as f32, rect.y as f32),
        Size::new(rect.width as f32, rect.height as f32),
    )
}

#[allow(clippy::cast_possible_truncation)]
fn rect(area: Rectangle) -> geometry::Rect {
    let (x, y) = (area.x.round() as i32, area.y.round() as i32);
    geometry::Rect::new(
        x,
        y,
        (area.x + area.width).round() as i32 - x,
        (area.y + area.height).round() as i32 - y,
    )
}

impl Overlay {
    pub fn boot(
        frames: Vec<Frame>,
        outcome: std::sync::Arc<std::sync::Mutex<Outcome>>,
    ) -> (Self, Task<Message>) {
        let screens: Vec<Screen> = frames
            .into_iter()
            .map(|frame| {
                let handle = image::Handle::from_rgba(
                    frame.image.width(),
                    frame.image.height(),
                    frame.image.as_raw().clone(),
                );
                let area = area(frame.output.logical);
                Screen {
                    frame,
                    handle,
                    area,
                }
            })
            .collect();
        let desktop = screens
            .iter()
            .map(|screen| screen.area)
            .reduce(|all, area| all.union(&area))
            .unwrap_or_default();
        let mut surfaces = HashMap::new();
        let open = screens.iter().enumerate().map(|(index, screen)| {
            let id = window::Id::unique();
            surfaces.insert(id, index);
            Task::done(Message::Layer(LayerShellCustomActionWithId::new(
                None,
                LayerShellCustomAction::NewLayerShell {
                    settings: NewLayerShellSettings {
                        size: Some((0, 0)),
                        layer: Layer::Overlay,
                        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                        exclusive_zone: Some(-1),
                        margin: Some((0, 0, 0, 0)),
                        keyboard_interactivity: KeyboardInteractivity::Exclusive,
                        output_option: OutputOption::OutputName(screen.frame.output.name.clone()),
                        events_transparent: false,
                        namespace: Some(NAMESPACE.to_owned()),
                    },
                    id,
                },
            )))
        });
        let open = Task::batch(open.collect::<Vec<_>>());
        let overlay = Self {
            screens,
            surfaces,
            desktop,
            selection: Selection::default(),
            pointer: None,
            palette: Palette::DARK,
            busy: false,
            outcome,
        };
        (overlay, open)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pointer(pointer) => self.pointer(pointer),
            Message::Nudge(by) => {
                self.selection = self.selection.nudge(by, self.desktop);
                Task::none()
            }
            Message::SelectAll => {
                self.selection = Selection::selected(self.desktop);
                Task::none()
            }
            Message::Accept => self.accept(),
            Message::Cancel => self.finish(Outcome::Cancelled),
            Message::Saved(Ok(path)) => self.finish(Outcome::Saved(path)),
            Message::Saved(Err(error)) => self.finish(Outcome::Failed(error)),
            Message::Layer(_) => Task::none(),
        }
    }

    fn pointer(&mut self, pointer: Pointer) -> Task<Message> {
        match pointer {
            Pointer::Pressed(at) => {
                self.pointer = Some(at);
                self.selection = self.selection.press(at, HANDLE_REACH);
            }
            Pointer::Moved(at) => {
                self.pointer = Some(at);
                self.selection = self.selection.drag_to(at, self.desktop);
            }
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
        Task::none()
    }

    fn accept(&mut self) -> Task<Message> {
        let Some(region) = self.selection.rect() else {
            return Task::none();
        };
        if self.busy {
            return Task::none();
        }
        self.busy = true;
        let Some(image) = geometry::crop(
            &self
                .screens
                .iter()
                .map(|screen| screen.frame.clone())
                .collect::<Vec<_>>(),
            rect(region),
        ) else {
            return self.finish(Outcome::Failed("the region is empty".into()));
        };
        Task::perform(crate::save::save(image), Message::Saved)
    }

    fn finish(&mut self, outcome: Outcome) -> Task<Message> {
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = outcome;
        }
        iced::exit()
    }

    pub fn view(&self, id: window::Id) -> Element<'_, Message> {
        let Some(&index) = self.surfaces.get(&id) else {
            return iced::widget::space().into();
        };
        let screen = &self.screens[index];
        stack![
            image(screen.handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Fill)
                .filter_method(image::FilterMethod::Linear),
            iced::widget::canvas(canvas::Surface {
                overlay: self,
                area: screen.area,
                scale: screen.frame.scale(),
            })
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .into()
    }

    #[expect(
        clippy::unused_self,
        reason = "iced passes the state to every subscription"
    )]
    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(key)
    }

    #[expect(
        clippy::unused_self,
        reason = "the overlay is always dark over a dimmed screen"
    )]
    pub fn theme(&self, _: window::Id) -> iced::Theme {
        iced::Theme::Dark
    }
}

#[allow(clippy::needless_pass_by_value)]
fn key(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    let step = if modifiers.shift() { 10.0 } else { 1.0 };
    match key.as_ref() {
        keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::Cancel),
        keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Message::Accept),
        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
            Some(Message::Nudge(Vector::new(-step, 0.0)))
        }
        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
            Some(Message::Nudge(Vector::new(step, 0.0)))
        }
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
            Some(Message::Nudge(Vector::new(0.0, -step)))
        }
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
            Some(Message::Nudge(Vector::new(0.0, step)))
        }
        keyboard::Key::Character("a") if modifiers.command() => Some(Message::SelectAll),
        keyboard::Key::Character("s") if modifiers.command() => Some(Message::Accept),
        _ => None,
    }
}

/// Shows the overlay on every screen as layer-shell surfaces until the user is done.
pub fn run(frames: Vec<Frame>) -> Result<Outcome, String> {
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    let outcome = std::sync::Arc::new(std::sync::Mutex::new(Outcome::Cancelled));
    let shared = outcome.clone();
    let frames = std::sync::Mutex::new(Some(frames));
    let boot = move || {
        let frames = frames
            .lock()
            .ok()
            .and_then(|mut frames| frames.take())
            .unwrap_or_default();
        Overlay::boot(frames, shared.clone())
    };
    iced_layershell::daemon(boot, NAMESPACE, Overlay::update, Overlay::view)
        .theme(Overlay::theme)
        .subscription(Overlay::subscription)
        .style(|_, _| iced::theme::Style {
            background_color: iced::Color::TRANSPARENT,
            text_color: iced::Color::WHITE,
        })
        .settings(Settings {
            id: Some(crate::APP_ID.to_owned()),
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..LayerShellSettings::default()
            },
            ..Settings::default()
        })
        .run()
        .map_err(|error| error.to_string())?;
    Ok(outcome
        .lock()
        .map(|outcome| outcome.clone())
        .unwrap_or_default())
}
