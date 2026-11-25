//! The editor's canvases: the image and, above it, its marks, zoomed and panned, with the pointer reported
//! in image coordinates.

use iced::mouse::{self, Interaction, ScrollDelta};
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector};
use kirie_annotate::{Draft, Shape, Tool};

use super::{Editor, MARK_REACH, Message, Pointer, pixel, viewport};
use crate::annotate::paint::{
    ImageSource, Marks, Painter, Part, color_of, draw_listed, draw_marquee, draw_picked,
};
use crate::annotate::selection::Handle;
use crate::canvas::loupe::{self, Lens};

/// The radius of a crop handle, in screen pixels.
const CROP_HANDLE: f32 = 5.0;
/// Mouse wheel steps per doubling of the zoom.
const WHEEL_STEPS: f32 = 6.0;
const LINE: f32 = 40.0;

/// The marks over the image, on canvases stacked one above the other, each doing its [`Part`].
pub(super) struct Board<'a> {
    pub editor: &'a Editor,
    pub part: Part,
}

#[derive(Debug, Default)]
pub(super) struct State {
    panning: Option<Point>,
    /// Where the pointer was last seen, for a release that comes after it left the canvas.
    last: Option<Point>,
}

impl Board<'_> {
    fn content(&self) -> kurbo::Rect {
        self.editor.content()
    }

    /// Whether a press started a gesture whose moves and release matter.
    fn gesture(&self) -> bool {
        self.editor.annotator.draft.is_some()
            || self
                .editor
                .cropping
                .is_some_and(|cropping| cropping.dragging())
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
        if self.part != Part::Input {
            return None;
        }
        let local = cursor.position_in(bounds);
        if let Some(at) = cursor.position_from(bounds.position()) {
            state.last = Some(at);
        }
        let canvas = bounds.size();
        let to_image = |at: Point| self.editor.viewport.image_point(at, self.content(), canvas);
        let publish = |message| Some(Action::publish(message).and_capture());
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let at = local?;
                let (x, y) = match *delta {
                    ScrollDelta::Lines { x, y } => (x * LINE, y * LINE),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                if self.editor.annotator.modifiers.command() {
                    publish(Message::Viewport(viewport::Event::ZoomBy {
                        factor: 2f32.powf(y / LINE / WHEEL_STEPS),
                        anchor: at,
                        canvas,
                    }))
                } else if self.editor.annotator.modifiers.shift() {
                    publish(Message::Viewport(viewport::Event::Pan(Vector::new(y, x))))
                } else {
                    publish(Message::Viewport(viewport::Event::Pan(Vector::new(x, y))))
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
                publish(Message::Viewport(viewport::Event::Pan(at - from)))
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                publish(Message::Pointer(
                    Pointer::Pressed(to_image(local?)),
                    scale(self.editor, canvas),
                ))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if self.gesture() => {
                let at = cursor.position_from(bounds.position())?;
                publish(Message::Pointer(
                    Pointer::Moved(to_image(at)),
                    scale(self.editor, canvas),
                ))
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if self.gesture() => {
                let at = cursor.position_from(bounds.position()).or(state.last)?;
                publish(Message::Pointer(
                    Pointer::Released(to_image(at)),
                    scale(self.editor, canvas),
                ))
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let hover = local.map(to_image);
                (hover.map(pixel) != self.editor.hover.map(pixel))
                    .then(|| Action::publish(Message::Hover(hover)))
            }
            _ if canvas != self.editor.canvas => Some(Action::publish(Message::Canvas(canvas))),
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let editor = self.editor;
        let mut frame = Frame::new(renderer, bounds.size());
        let Some((image, _)) = &editor.image else {
            return vec![frame.into_geometry()];
        };
        let transform = editor.viewport.transform(self.content(), bounds.size());
        let layer = match self.part {
            Part::Input => return vec![frame.into_geometry()],
            Part::Marks(layer) => Some(layer),
            Part::Chrome => None,
        };
        if editor.crop().is_none() || layer.is_some() {
            let kept = to_rectangle(transform.transform_rect_bbox(editor.shown_canvas()));
            let clip = editor
                .crop()
                .map_or(kept, |_| Rectangle::with_size(bounds.size()));
            let picked = frame.with_clip(clip, |frame| {
                self.draw_marks(frame, image, transform, layer)
            });
            if layer.is_some() {
                return vec![frame.into_geometry()];
            }
            self.draw_chrome(&mut frame, &picked, transform, bounds.size());
        } else if let Some(crop) = editor.crop() {
            draw_crop(&mut frame, crop, transform, bounds.size(), editor);
        }
        if editor.annotator.sampling()
            && let Some(local) = cursor.position_in(bounds)
        {
            let at = editor
                .viewport
                .image_point(local, self.content(), bounds.size());
            let (x, y) = pixel(at);
            let lens = Lens::sample(image, x, y);
            let label = match lens.center() {
                Some(color) => format!("{x}, {y}   {}", loupe::hex(color)),
                None => format!("{x}, {y}"),
            };
            loupe::draw(
                &mut frame,
                &lens,
                &label,
                local,
                bounds.size(),
                editor.palette,
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
        if self.part == Part::Input {
            self.interaction(state, bounds, cursor)
        } else {
            Interaction::default()
        }
    }
}

impl Board<'_> {
    /// The outlines of the picked marks, `picked` as they are shown, and a marquee.
    fn draw_chrome(
        &self,
        frame: &mut Frame,
        picked: &[Shape],
        transform: kurbo::Affine,
        size: Size,
    ) {
        let editor = self.editor;
        let pixel = 1.0 / f64::from(editor.viewport.scale(self.content(), size));
        let accent = editor.palette.accent;
        draw_picked(
            frame,
            picked,
            &editor.annotator.picked,
            editor.document(),
            transform,
            pixel,
            accent,
        );
        if let Some(shape) = editor
            .mark_list
            .hovered()
            .filter(|id| !editor.annotator.picked.contains(id))
            .and_then(|id| editor.document().shape(id))
        {
            draw_listed(frame, shape, editor.document(), transform, accent);
        }
        if let Some(rect) = editor.annotator.draft.as_ref().and_then(Draft::marquee) {
            draw_marquee(frame, rect, transform, accent);
        }
    }

    fn interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> Interaction {
        if state.panning.is_some() {
            return Interaction::Grabbing;
        }
        let Some(local) = cursor.position_in(bounds) else {
            return Interaction::default();
        };
        if self.editor.image.is_none() {
            return Interaction::default();
        }
        let at = self
            .editor
            .viewport
            .image_point(local, self.content(), bounds.size());
        if self.editor.annotator.sampling() {
            return Interaction::Crosshair;
        }
        if let Some(cropping) = &self.editor.cropping {
            let reach = super::crop::HANDLE_REACH / scale(self.editor, bounds.size());
            return crate::annotate::selection::interaction(cropping.hit(at, reach));
        }
        match self.editor.tool {
            Tool::Select => {
                let reach = MARK_REACH / f64::from(scale(self.editor, bounds.size()));
                crate::canvas::cursor::select(
                    self.editor.document(),
                    &self.editor.annotator.picked,
                    self.editor.annotator.draft.as_ref(),
                    at,
                    reach,
                )
            }
            Tool::Text => Interaction::Text,
            _ => Interaction::Crosshair,
        }
    }
}

impl Board<'_> {
    /// Draws the marks of `layer` (none for `None`) and the one in progress; returns the picked
    /// marks as shown.
    fn draw_marks(
        &self,
        frame: &mut Frame,
        image: &::image::RgbaImage,
        transform: kurbo::Affine,
        layer: Option<usize>,
    ) -> Vec<Shape> {
        let editor = self.editor;
        let document = editor.document();
        let source = ImageSource(image);
        let painter = Painter {
            source: &source,
            typesetter: &editor.typesetter,
            patches: &editor.patches,
            pictures: &editor.pictures,
        };
        let preview = editor.annotator.preview();
        painter.draw_marks(
            frame,
            Marks {
                document,
                draft: editor.annotator.draft.as_ref(),
                preview,
                picked: &editor.annotator.picked,
                visible: document.visible(),
                layer: Some(layer.unwrap_or(usize::MAX)),
            },
            transform,
        )
    }
}

/// The image and its shadow, drawn in a layer below the [`Board`]: the renderer draws a layer's
/// images after its shapes, so sharing one canvas would hide the marks under the image.
pub(super) struct Backdrop<'a> {
    pub editor: &'a Editor,
}

impl canvas::Program<Message> for Backdrop<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let editor = self.editor;
        let mut frame = Frame::new(renderer, bounds.size());
        let Some((_, handle)) = &editor.image else {
            return vec![frame.into_geometry()];
        };
        let image = editor.image_size();
        let content = editor.content();
        let transform = editor.viewport.transform(content, bounds.size());
        let scale = editor.viewport.scale(content, bounds.size());
        let canvas = editor.shown_canvas();
        let area = to_rectangle(transform.transform_rect_bbox(canvas));
        let document = editor.document();
        let shown = if editor.cropping.is_some() {
            content
        } else {
            document.visible()
        };
        let kept = to_rectangle(transform.transform_rect_bbox(shown));
        // Without a frame the picture stands alone: anything drawn around it would look like
        // part of it, though it is never saved.
        if shown != canvas {
            let on_screen = area
                .intersection(&Rectangle::with_size(bounds.size()))
                .unwrap_or(area);
            match document.frame.background {
                Some(color) => {
                    frame.fill_rectangle(on_screen.position(), on_screen.size(), color_of(color));
                }
                None => checker(&mut frame, area, on_screen),
            }
            if document.frame.shadow {
                #[allow(clippy::cast_possible_truncation)]
                let radius = document.shadow_radius() as f32;
                soft_shadow(&mut frame, kept, radius * scale);
            }
        }
        let picture = Rectangle::new(
            transform_point(transform, kurbo::Point::ORIGIN),
            Size::new(image.width * scale, image.height * scale),
        );
        let filter = if scale >= 2.0 {
            iced::widget::image::FilterMethod::Nearest
        } else {
            iced::widget::image::FilterMethod::Linear
        };
        frame.with_clip(kept, |frame| {
            frame.draw_image(
                picture,
                canvas::Image::new(handle.clone()).filter_method(filter),
            );
        });
        vec![frame.into_geometry()]
    }
}

fn scale(editor: &Editor, canvas: Size) -> f32 {
    editor.viewport.scale(editor.content(), canvas)
}

/// A checkerboard over the part `shown` of `area`, which shows that it stays transparent.
fn checker(frame: &mut Frame, area: Rectangle, shown: Rectangle) {
    const CELL: f32 = 8.0;
    frame.fill_rectangle(
        shown.position(),
        shown.size(),
        Color::from_rgb8(0xf2, 0xf2, 0xf2),
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cells = |from: f32, to: f32, origin: f32| {
        let first = ((from - origin) / CELL).floor().max(0.0) as u32;
        let last = ((to - origin) / CELL).ceil().max(0.0) as u32;
        first..last
    };
    let dark = Path::new(|path| {
        for row in cells(shown.y, shown.y + shown.height, area.y) {
            for column in cells(shown.x, shown.x + shown.width, area.x) {
                if (row + column) % 2 == 1 {
                    #[allow(clippy::cast_precision_loss)]
                    let cell = Rectangle::new(
                        Point::new(area.x + column as f32 * CELL, area.y + row as f32 * CELL),
                        Size::new(CELL, CELL),
                    );
                    if let Some(cell) = cell.intersection(&shown) {
                        path.rectangle(cell.position(), cell.size());
                    }
                }
            }
        }
    });
    frame.fill(&dark, Color::from_rgb8(0xd9, 0xd9, 0xd9));
}

/// A soft shadow under `picture`, `radius` screen pixels deep, built up from wider and fainter
/// rounded rectangles.
fn soft_shadow(frame: &mut Frame, picture: Rectangle, radius: f32) {
    const STEPS: u8 = 8;
    if radius < 1.0 {
        return;
    }
    for step in 1..=STEPS {
        let spread = radius * f32::from(step) / f32::from(STEPS);
        let at = Point::new(picture.x - spread, picture.y - spread + radius * 0.4);
        let size = Size::new(picture.width + spread * 2.0, picture.height + spread * 2.0);
        frame.fill(
            &Path::rounded_rectangle(at, size, spread.into()),
            Color::BLACK.scale_alpha(0.4 / f32::from(STEPS)),
        );
    }
}

/// Dims what the crop leaves out, and outlines the crop with its handles.
fn draw_crop(
    frame: &mut Frame,
    crop: Rectangle,
    transform: kurbo::Affine,
    canvas: Size,
    editor: &Editor,
) {
    let crop = to_rectangle(transform.transform_rect_bbox(kurbo::Rect::new(
        f64::from(crop.x),
        f64::from(crop.y),
        f64::from(crop.x + crop.width),
        f64::from(crop.y + crop.height),
    )));
    let outside = Path::new(|path| {
        path.rectangle(Point::ORIGIN, canvas);
        path.rectangle(crop.position(), crop.size());
    });
    frame.fill(
        &outside,
        canvas::Fill {
            style: canvas::Style::Solid(Color::BLACK.scale_alpha(0.55)),
            rule: canvas::fill::Rule::EvenOdd,
        },
    );
    let accent = editor.palette.accent;
    frame.stroke(
        &Path::rectangle(crop.position(), crop.size()),
        Stroke::default().with_color(accent).with_width(1.5),
    );
    let dragging = editor.cropping.is_some_and(|cropping| cropping.dragging());
    if !dragging {
        for handle in Handle::ALL {
            let dot = Path::circle(handle.position(crop), CROP_HANDLE);
            frame.fill(&dot, Color::WHITE);
            frame.stroke(&dot, Stroke::default().with_color(accent).with_width(1.5));
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
pub(super) fn to_rectangle(rect: kurbo::Rect) -> Rectangle {
    Rectangle::new(
        Point::new(rect.x0 as f32, rect.y0 as f32),
        Size::new(rect.width() as f32, rect.height() as f32),
    )
}

#[allow(clippy::cast_possible_truncation)]
fn transform_point(transform: kurbo::Affine, point: kurbo::Point) -> Point {
    let point = transform * point;
    Point::new(point.x as f32, point.y as f32)
}
