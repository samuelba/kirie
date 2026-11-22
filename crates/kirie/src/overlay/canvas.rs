//! What one screen's surface draws over its frozen image, and how it reports the pointer in
//! desktop coordinates.

use iced::mouse::{self, Interaction};
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke, Text, fill};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector, alignment};
use kirie_ui::{radius, space};

use kirie_annotate::{Shape, Tool};

use super::selection::{Handle, Hit};
use super::{HANDLE_REACH, MARK_REACH, Message, Overlay, Pointer, to_kurbo};
use crate::paint::{Captures, Marks, Painter, draw_grips};

const HANDLE_RADIUS: f32 = 4.5;
const LABEL_SIZE: f32 = 13.0;

pub(super) struct Surface<'a> {
    pub overlay: &'a Overlay,
    /// This screen in desktop coordinates.
    pub area: Rectangle,
    /// Physical pixels per logical pixel on this screen.
    pub scale: f64,
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
        self.draw_marks(&mut frame);
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
        } else {
            if let Some(at) = cursor.position_in(bounds) {
                draw_crosshair(&mut frame, at, bounds.size());
                self.draw_window_under(&mut frame, self.to_desktop(at));
            }
            self.draw_hint(&mut frame, bounds.size());
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Interaction {
        let Some(at) = cursor.position().map(|at| self.to_desktop(at)) else {
            return Interaction::default();
        };
        if self.overlay.selection.dragging() {
            return Interaction::Crosshair;
        }
        match self.overlay.tool {
            Some(Tool::Select) => {
                return crate::cursor::select(
                    self.overlay.document(),
                    self.overlay.picked,
                    self.overlay.draft.as_ref(),
                    to_kurbo(at),
                    MARK_REACH,
                );
            }
            Some(Tool::Text) => return Interaction::Text,
            Some(_) => return Interaction::Crosshair,
            None => {}
        }
        match self.overlay.selection.hit(at, HANDLE_REACH) {
            Some(Hit::Inside) => Interaction::Grab,
            Some(Hit::Handle(Handle::North | Handle::South)) => Interaction::ResizingVertically,
            Some(Hit::Handle(Handle::East | Handle::West)) => Interaction::ResizingHorizontally,
            Some(Hit::Handle(Handle::NorthEast | Handle::SouthWest)) => {
                Interaction::ResizingDiagonallyUp
            }
            Some(Hit::Handle(Handle::NorthWest | Handle::SouthEast)) => {
                Interaction::ResizingDiagonallyDown
            }
            None => Interaction::Crosshair,
        }
    }
}

impl Surface<'_> {
    /// The marks, and the one being drawn, under the scrim so it dims those outside.
    fn draw_marks(&self, frame: &mut Frame) {
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
        };
        let transform =
            kurbo::Affine::translate((-f64::from(self.area.x), -f64::from(self.area.y)));
        let preview = overlay
            .draft
            .as_ref()
            .and_then(kirie_annotate::Draft::preview)
            .map(|kind| Shape {
                id: document.next_id(),
                kind,
                paint: overlay.paint,
                style: None,
                rotation: 0.0,
            })
            .or_else(|| {
                overlay
                    .typing
                    .as_ref()
                    .map(|typing| typing.preview(document, overlay.paint))
            });
        let picked = painter.draw_marks(
            frame,
            Marks {
                document,
                draft: overlay.draft.as_ref(),
                preview,
                picked: overlay.picked,
                visible,
            },
            transform,
        );
        if let Some(picked) = picked {
            let bounds = document.bounds(&picked);
            draw_grips(
                frame,
                &picked,
                bounds,
                transform,
                1.0,
                overlay.palette.accent,
            );
        }
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

    fn draw_hint(&self, frame: &mut Frame, size: Size) {
        let palette = self.overlay.palette;
        let hint = if self.overlay.windows.is_empty() {
            "Drag to cut a region  ·  Click to take the screen  ·  Esc to cancel"
        } else {
            "Drag to cut a region  ·  Click to take a window or the screen  ·  Esc to cancel"
        };
        let pill_size = Size::new(label_width(hint), LABEL_SIZE + space::M * 2.0);
        let at = Point::new(
            (size.width - pill_size.width) / 2.0,
            size.height - pill_size.height - space::XL * 2.0,
        );
        pill(frame, at, pill_size, palette.surface[1].scale_alpha(0.92));
        frame.fill_text(Text {
            content: hint.to_owned(),
            position: Point::new(size.width / 2.0, at.y + pill_size.height / 2.0),
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
