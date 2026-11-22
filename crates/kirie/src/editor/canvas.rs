//! The editor's canvas: the image and its marks, zoomed and panned, with the pointer reported
//! in image coordinates.

use iced::mouse::{self, Interaction, ScrollDelta};
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};
use kirie_annotate::{Draft, Kind, Shape, Tool, style};

use super::{Editor, MARK_REACH, Message, Pointer};
use crate::paint::{ImageSource, Painter};

/// Mouse wheel steps per doubling of the zoom.
const WHEEL_STEPS: f32 = 6.0;
const LINE: f32 = 40.0;

pub(super) struct Board<'a> {
    pub editor: &'a Editor,
}

#[derive(Debug, Default)]
pub(super) struct State {
    panning: Option<Point>,
}

impl Board<'_> {
    fn image(&self) -> Size {
        self.editor.image_size()
    }
}

impl canvas::Program<Message> for Board<'_> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let local = cursor.position_in(bounds);
        let canvas = bounds.size();
        let to_image = |at: Point| self.editor.viewport.image_point(at, self.image(), canvas);
        let publish = |message| Some(Action::publish(message).and_capture());
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let at = local?;
                let (x, y) = match *delta {
                    ScrollDelta::Lines { x, y } => (x * LINE, y * LINE),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                if self.editor.modifiers.command() {
                    publish(Message::Zoom {
                        factor: 2f32.powf(y / LINE / WHEEL_STEPS),
                        anchor: at,
                        canvas,
                    })
                } else if self.editor.modifiers.shift() {
                    publish(Message::Pan(Vector::new(y, x)))
                } else {
                    publish(Message::Pan(Vector::new(x, y)))
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                state.panning = Some(local?);
                Some(Action::capture())
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                state.panning = None;
                Some(Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.panning.is_some() => {
                let at = cursor.position_from(bounds.position())?;
                let from = state.panning.replace(at)?;
                publish(Message::Pan(at - from))
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                publish(Message::Pointer(
                    Pointer::Pressed(to_image(local?)),
                    scale(self.editor, canvas),
                ))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if self.editor.draft.is_some() => {
                let at = cursor.position_from(bounds.position())?;
                publish(Message::Pointer(
                    Pointer::Moved(to_image(at)),
                    scale(self.editor, canvas),
                ))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if self.editor.draft.is_some() =>
            {
                let at = cursor.position_from(bounds.position())?;
                publish(Message::Pointer(
                    Pointer::Released(to_image(at)),
                    scale(self.editor, canvas),
                ))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let editor = self.editor;
        let mut frame = Frame::new(renderer, bounds.size());
        let Some((image, handle)) = &editor.image else {
            return vec![frame.into_geometry()];
        };
        let transform = editor.viewport.transform(self.image(), bounds.size());
        let document = editor.document();
        let area = transform.transform_rect_bbox(document.visible());
        #[allow(clippy::cast_possible_truncation)]
        let area = Rectangle::new(
            Point::new(area.x0 as f32, area.y0 as f32),
            Size::new(area.width() as f32, area.height() as f32),
        );
        frame.fill_rectangle(
            area.position() + Vector::new(0.0, 6.0),
            area.size(),
            Color::BLACK.scale_alpha(0.25),
        );
        frame.draw_image(
            Rectangle::new(
                transform_point(transform, kurbo::Point::ORIGIN),
                Size::new(
                    self.image().width * editor.viewport.scale(self.image(), bounds.size()),
                    self.image().height * editor.viewport.scale(self.image(), bounds.size()),
                ),
            ),
            canvas::Image::new(handle.clone()).filter_method(
                if editor.viewport.scale(self.image(), bounds.size()) >= 2.0 {
                    iced::widget::image::FilterMethod::Nearest
                } else {
                    iced::widget::image::FilterMethod::Linear
                },
            ),
        );

        let picked_bounds = self.draw_marks(&mut frame, image, transform);
        if let Some(picked) = picked_bounds {
            let outline = transform.transform_rect_bbox(picked.inflate(6.0, 6.0));
            #[allow(clippy::cast_possible_truncation)]
            frame.stroke(
                &Path::rectangle(
                    Point::new(outline.x0 as f32, outline.y0 as f32),
                    Size::new(outline.width() as f32, outline.height() as f32),
                ),
                Stroke {
                    line_dash: canvas::LineDash {
                        segments: &[6.0, 4.0],
                        offset: 0,
                    },
                    ..Stroke::default()
                        .with_color(editor.palette.accent)
                        .with_width(1.5)
                },
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Interaction {
        if state.panning.is_some() {
            return Interaction::Grabbing;
        }
        let Some(local) = cursor.position_in(bounds) else {
            return Interaction::default();
        };
        if self.editor.image.is_none() {
            return Interaction::default();
        }
        match self.editor.tool {
            Tool::Select => {
                let at = self
                    .editor
                    .viewport
                    .image_point(local, self.image(), bounds.size());
                let reach = MARK_REACH / f64::from(scale(self.editor, bounds.size()));
                if self.editor.document().hit(at, reach).is_some() {
                    Interaction::Grab
                } else {
                    Interaction::default()
                }
            }
            Tool::Text => Interaction::Text,
            _ => Interaction::Crosshair,
        }
    }
}

impl Board<'_> {
    /// Draws the marks and the one in progress; returns the picked mark's bounds.
    fn draw_marks(
        &self,
        frame: &mut Frame,
        image: &::image::RgbaImage,
        transform: kurbo::Affine,
    ) -> Option<kurbo::Rect> {
        let editor = self.editor;
        let document = editor.document();
        let source = ImageSource(image);
        let painter = Painter {
            source: &source,
            typesetter: &editor.typesetter,
            patches: &editor.patches,
        };
        let moving = match &editor.draft {
            Some(Draft::Moving { ids, by, .. }) => Some((ids.as_slice(), *by)),
            _ => None,
        };
        let preview = editor
            .draft
            .as_ref()
            .and_then(Draft::preview)
            .or_else(|| {
                editor.typing.as_ref().map(|typing| Kind::Text {
                    at: typing.at,
                    text: if typing.text.is_empty() {
                        "|".to_owned()
                    } else {
                        typing.text.clone()
                    },
                })
            })
            .map(|kind| Shape {
                id: document.next_id(),
                kind,
                paint: editor.paint,
                style: None,
            });
        let visible = document.visible();
        let mut picked_bounds = None;
        for shape in document.shapes().cloned().chain(preview) {
            let shape = match moving {
                Some((ids, by)) if ids.contains(&shape.id) => Shape {
                    kind: shape.kind.translated(by),
                    ..shape
                },
                _ => shape,
            };
            if editor.picked == Some(shape.id) {
                picked_bounds = Some(shape.kind.bounds());
            }
            for primitive in style::primitives(&shape, document.style, document.unit, visible) {
                painter.draw(frame, &primitive, transform);
            }
        }
        picked_bounds
    }
}

fn scale(editor: &Editor, canvas: Size) -> f32 {
    editor.viewport.scale(editor.image_size(), canvas)
}

#[allow(clippy::cast_possible_truncation)]
fn transform_point(transform: kurbo::Affine, point: kurbo::Point) -> Point {
    let point = transform * point;
    Point::new(point.x as f32, point.y as f32)
}
