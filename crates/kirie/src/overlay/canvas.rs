//! What one screen's surface draws over its frozen image, and how it reports the pointer in
//! desktop coordinates.

use iced::mouse::{self, Interaction};
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke, Text, fill};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector, alignment};
use kirie_ui::{radius, space};

use kirie_annotate::Tool;

use super::{HANDLE_REACH, MARK_REACH, Message, Overlay, Pointer, to_kurbo};
use crate::annotate::paint::{Captures, Marks, Painter, Part, draw_marquee, draw_picked};
use crate::annotate::selection::Handle;
use crate::canvas::loupe::{self, Lens};

const HANDLE_RADIUS: f32 = 4.5;
const LABEL_SIZE: f32 = 13.0;

pub(super) struct Surface<'a> {
    pub overlay: &'a Overlay,
    /// This screen in desktop coordinates.
    pub area: Rectangle,
    /// Physical pixels per logical pixel on this screen.
    pub scale: f64,
    pub part: Part,
}

impl Surface<'_> {
    fn to_desktop(&self, local: Point) -> Point {
        local + Vector::new(self.area.x, self.area.y)
    }

    fn to_local(&self, rect: Rectangle) -> Rectangle {
        Rectangle {
            x: rect.x - self.area.x,
            y: rect.y - self.area.y,
            ..rect
        }
    }
}

impl canvas::Program<Message> for Surface<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        _bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        if self.part != Part::Input {
            return None;
        }
        // While a button is held the compositor keeps sending the pointer here, even beyond the
        // screen's edge, so a drag can cross onto the next screen.
        let at = self.to_desktop(cursor.position()?);
        let pointer = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Pointer::Pressed(at),
            Event::Mouse(mouse::Event::CursorMoved { .. }) => Pointer::Moved(at),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Pointer::Released(at)
            }
            _ => return None,
        };
        Some(Action::publish(Message::Pointer(pointer)).and_capture())
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let palette = self.overlay.palette;
        let mut frame = Frame::new(renderer, bounds.size());
        match self.part {
            Part::Input => return vec![frame.into_geometry()],
            Part::Marks(layer) => {
                self.draw_marks(&mut frame, Some(layer));
                return vec![frame.into_geometry()];
            }
            Part::Chrome => self.draw_marks(&mut frame, None),
        }
        let selection = self
            .overlay
            .selection
            .rect()
            .map(|rect| self.to_local(rect));

        let scrim = Path::new(|path| {
            path.rectangle(Point::ORIGIN, bounds.size());
            if let Some(rect) = selection {
                path.rectangle(rect.position(), rect.size());
            }
        });
        frame.fill(
            &scrim,
            canvas::Fill {
                style: canvas::Style::Solid(palette.scrim),
                rule: fill::Rule::EvenOdd,
            },
        );

        if let Some(rect) = selection {
            self.draw_selection(&mut frame, rect);
        } else if let Some(at) = cursor.position_in(bounds) {
            draw_crosshair(&mut frame, at, bounds.size());
            self.draw_window_under(&mut frame, self.to_desktop(at));
        }
        if let Some(at) = cursor.position_in(bounds)
            && self.magnifying()
        {
            self.draw_loupe(&mut frame, at, bounds.size());
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Interaction {
        if self.part != Part::Input {
            return Interaction::default();
        }
        let Some(at) = cursor.position().map(|at| self.to_desktop(at)) else {
            return Interaction::default();
        };
        if self.overlay.selection.dragging() || self.overlay.annotator.sampling() {
            return Interaction::Crosshair;
        }
        match self.overlay.tool {
            Some(Tool::Select) => {
                return crate::canvas::cursor::select(
                    self.overlay.document(),
                    &self.overlay.annotator.picked,
                    self.overlay.annotator.draft.as_ref(),
                    to_kurbo(at),
                    MARK_REACH,
                );
            }
            Some(Tool::Text) => return Interaction::Text,
            Some(_) => return Interaction::Crosshair,
            None => {}
        }
        crate::annotate::selection::interaction(self.overlay.selection.hit(at, HANDLE_REACH))
    }
}

impl Surface<'_> {
    /// The marks of `layer`, and the one being drawn, under the scrim so it dims those
    /// outside; with no layer, the picked marks' outlines and a marquee instead.
    fn draw_marks(&self, frame: &mut Frame, layer: Option<usize>) {
        let overlay = self.overlay;
        let document = overlay.document();
        let visible = overlay.selection.rect().map_or(kurbo::Rect::ZERO, |rect| {
            kurbo::Rect::new(
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.x + rect.width),
                f64::from(rect.y + rect.height),
            )
        });
        let source = Captures(&overlay.captures);
        let painter = Painter {
            source: &source,
            typesetter: &overlay.typesetter,
            patches: &overlay.patches,
            pictures: &overlay.pictures,
        };
        let transform =
            kurbo::Affine::translate((-f64::from(self.area.x), -f64::from(self.area.y)));
        let preview = overlay.annotator.preview();
        let picked = painter.draw_marks(
            frame,
            Marks {
                document,
                draft: overlay.annotator.draft.as_ref(),
                preview,
                picked: &overlay.annotator.picked,
                visible,
                layer: Some(layer.unwrap_or(usize::MAX)),
            },
            transform,
        );
        if layer.is_some() {
            return;
        }
        let accent = overlay.palette.accent;
        draw_picked(
            frame,
            &picked,
            &overlay.annotator.picked,
            document,
            transform,
            1.0,
            accent,
        );
        if let Some(rect) = overlay
            .annotator
            .draft
            .as_ref()
            .and_then(kirie_annotate::Draft::marquee)
        {
            draw_marquee(frame, rect, transform, accent);
        }
    }

    /// Whether the loupe shows: while a region is picked unless it was turned off, not while
    /// marks are drawn, and while the eyedropper waits for a click.
    fn magnifying(&self) -> bool {
        let overlay = self.overlay;
        overlay.annotator.sampling()
            || overlay.magnifier
                && overlay.tool.is_none()
                && overlay.toolbar_drag.is_none()
                && (overlay.selection.rect().is_none() || overlay.selection.dragging())
    }

    /// The frozen pixels around the pointer at `at`, magnified, with its place and color.
    fn draw_loupe(&self, frame: &mut Frame, at: Point, size: Size) {
        let desktop = self.to_desktop(at);
        let Some(capture) = self
            .overlay
            .captures
            .iter()
            .find(|capture| super::area(capture.output.logical).contains(desktop))
        else {
            return;
        };
        let logical = capture.output.logical;
        let scale = capture.scale();
        #[allow(clippy::cast_possible_truncation)]
        let pixel = |at: f32, from: i32| ((f64::from(at) - f64::from(from)) * scale).floor() as i64;
        let lens = Lens::sample(
            &capture.image,
            pixel(desktop.x, logical.x),
            pixel(desktop.y, logical.y),
        );
        #[allow(clippy::cast_possible_truncation)]
        let mut label = format!("{}, {}", desktop.x.floor() as i64, desktop.y.floor() as i64);
        if let Some(color) = lens.center() {
            label = format!("{label}   {}", loupe::hex(color));
        }
        loupe::draw(frame, &lens, &label, at, size, self.overlay.palette);
    }

    fn draw_selection(&self, frame: &mut Frame, rect: Rectangle) {
        let palette = self.overlay.palette;
        frame.stroke(
            &Path::rectangle(rect.position(), rect.size()),
            Stroke::default().with_color(palette.accent).with_width(1.5),
        );
        if !self.overlay.selection.dragging() {
            for handle in Handle::ALL {
                let dot = Path::circle(handle.position(rect), HANDLE_RADIUS);
                frame.fill(&dot, Color::WHITE);
                frame.stroke(
                    &dot,
                    Stroke::default().with_color(palette.accent).with_width(1.5),
                );
            }
        }
        #[allow(clippy::cast_possible_truncation)]
        let (width, height) = (
            (f64::from(rect.width) * self.scale).round() as i64,
            (f64::from(rect.height) * self.scale).round() as i64,
        );
        let label = format!("{width} × {height}");
        let size = Size::new(label_width(&label), LABEL_SIZE + space::S * 2.0);
        let above = rect.y - size.height - space::S;
        let at = Point::new(
            rect.x,
            if above >= 0.0 {
                above
            } else {
                rect.y + space::S
            },
        );
        pill(frame, at, size, palette.surface[1].scale_alpha(0.92));
        frame.fill_text(Text {
            content: label,
            position: Point::new(at.x + size.width / 2.0, at.y + size.height / 2.0),
            color: palette.text,
            size: Pixels(LABEL_SIZE),
            align_x: alignment::Horizontal::Center.into(),
            align_y: alignment::Vertical::Center,
            ..Text::default()
        });
    }

    /// Outlines the window under the pointer, which a click takes.
    fn draw_window_under(&self, frame: &mut Frame, at: Point) {
        let Some((area, title)) = self
            .overlay
            .windows
            .iter()
            .find(|(area, _)| area.contains(at))
        else {
            return;
        };
        let palette = self.overlay.palette;
        let local = self.to_local(*area);
        frame.fill_rectangle(
            local.position(),
            local.size(),
            palette.accent.scale_alpha(0.12),
        );
        frame.stroke(
            &Path::rectangle(local.position(), local.size()),
            Stroke::default().with_color(palette.accent).with_width(2.0),
        );
        if title.is_empty() {
            return;
        }
        let label: String = title.chars().take(60).collect();
        let size = Size::new(label_width(&label), LABEL_SIZE + space::S * 2.0);
        let at = Point::new(local.x + space::S, local.y + space::S);
        pill(frame, at, size, palette.surface[1].scale_alpha(0.92));
        frame.fill_text(Text {
            content: label,
            position: Point::new(at.x + size.width / 2.0, at.y + size.height / 2.0),
            color: palette.text,
            size: Pixels(LABEL_SIZE),
            align_x: alignment::Horizontal::Center.into(),
            align_y: alignment::Vertical::Center,
            ..Text::default()
        });
    }
}

fn draw_crosshair(frame: &mut Frame, at: Point, size: Size) {
    let line = Stroke::default()
        .with_color(Color::WHITE.scale_alpha(0.35))
        .with_width(1.0);
    frame.stroke(
        &Path::line(Point::new(0.0, at.y), Point::new(size.width, at.y)),
        line,
    );
    frame.stroke(
        &Path::line(Point::new(at.x, 0.0), Point::new(at.x, size.height)),
        line,
    );
}

fn pill(frame: &mut Frame, at: Point, size: Size, color: Color) {
    frame.fill(
        &Path::rounded_rectangle(at, size, radius::PILL.min(size.height / 2.0).into()),
        color,
    );
}

/// A generous width for a one-line label, with room on both sides.
#[allow(clippy::cast_precision_loss)]
fn label_width(text: &str) -> f32 {
    text.chars().count() as f32 * LABEL_SIZE * 0.56 + space::L * 2.0
}
