//! Cropping the image: a rectangle drawn over all of it, kept in whole pixels once done.

use iced::{Event as IcedEvent, Point, Rectangle, Size, event, keyboard, window};
use kirie_annotate::Op;

use super::viewport::Viewport;
use super::{Editor, Message, Pointer, canvas::to_rectangle, to_point};
use crate::annotate::selection::{Hit, Release, Selection};

/// How close to a crop handle a press grabs it, in screen pixels.
pub const HANDLE_REACH: f32 = 8.0;

#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// Starts cropping, or keeps the crop when already cropping.
    Start,
    Done,
    /// Keeps the whole image.
    Reset,
    Cancel,
}

/// The crop being drawn, in image coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Cropping {
    selection: Selection,
    /// The crop before the last press, kept when that press turns out to be a click.
    before: Selection,
}

impl Cropping {
    fn new(rect: Rectangle) -> Self {
        let selection = Selection::selected(rect);
        Self {
            selection,
            before: selection,
        }
    }

    fn rect(&self) -> Option<Rectangle> {
        self.selection.rect()
    }

    pub fn dragging(&self) -> bool {
        self.selection.dragging()
    }

    pub fn hit(&self, at: kurbo::Point, reach: f32) -> Option<Hit> {
        self.selection.hit(to_point(at), reach)
    }

    /// Follows the pointer over an image of `size`, shown at `scale` canvas pixels per image
    /// pixel. A click, or a drag that leaves nothing, puts back the crop from before.
    fn follow(&mut self, pointer: Pointer, scale: f32, size: Size) {
        let reach = HANDLE_REACH / scale.max(0.01);
        match pointer {
            Pointer::Pressed(at) => {
                self.before = self.selection;
                self.selection = self.selection.press(to_point(at), reach);
            }
            Pointer::Moved(at) => {
                let bounds = Rectangle::new(Point::ORIGIN, size);
                self.selection = self.selection.drag_to(to_point(at), bounds);
            }
            Pointer::Released(at) => {
                let (selection, release) = self.selection.release(to_point(at));
                self.selection = match release {
                    Release::Click(_) => self.before,
                    _ if selection.rect().is_none() => self.before,
                    _ => selection,
                };
            }
        }
    }
}

impl Editor {
    pub(super) fn crop_event(&mut self, event: Event) {
        match event {
            Event::Cancel if self.cropping.is_some() => {
                self.cropping = None;
                self.viewport = Viewport::fitted();
            }
            Event::Start if self.cropping.is_some() => self.keep_crop(),
            Event::Start if self.image.is_some() => {
                self.annotator.commit_text();
                self.annotator.draft = None;
                self.annotator.picked.clear();
                self.cropping = Some(Cropping::new(to_rectangle(self.document().visible())));
                self.viewport = Viewport::fitted();
            }
            Event::Done => self.keep_crop(),
            Event::Reset => {
                let whole = Rectangle::new(Point::ORIGIN, self.image_size());
                if let Some(cropping) = &mut self.cropping {
                    cropping.selection = Selection::selected(whole);
                }
            }
            Event::Start | Event::Cancel => {}
        }
    }

    /// Ends cropping, keeping the crop drawn, in whole pixels; all of the image is no crop.
    pub(super) fn keep_crop(&mut self) {
        let Some(cropping) = self.cropping.take() else {
            return;
        };
        self.viewport = Viewport::fitted();
        let Some(rect) = cropping.rect() else {
            return;
        };
        let crop = kept(rect, self.image_size());
        if crop != self.document().crop {
            self.annotator.history.apply(Op::Crop(crop));
        }
    }

    /// Follows the pointer while cropping.
    pub(super) fn crop_pointer(&mut self, pointer: Pointer, scale: f32) {
        let size = self.image_size();
        if let Some(cropping) = &mut self.cropping {
            cropping.follow(pointer, scale, size);
        }
    }

    /// The crop being drawn, while cropping.
    pub(super) fn crop(&self) -> Option<Rectangle> {
        self.cropping.and_then(|cropping| cropping.rect())
    }
}

/// The crop that keeps `rect` of an image of `size`, in whole pixels: none for all of it.
fn kept(rect: Rectangle, size: Size) -> Option<kurbo::Rect> {
    let rect = kurbo::Rect::new(
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.x + rect.width),
        f64::from(rect.y + rect.height),
    )
    .round();
    let whole = kurbo::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
    (rect != whole && rect.area() >= 1.0).then_some(rect)
}

/// Enter keeps the crop; every other key works as usual.
pub fn key(event: IcedEvent, status: event::Status, id: window::Id) -> Option<Message> {
    if let IcedEvent::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Enter),
        ..
    }) = event
    {
        return Some(Message::Crop(Event::Done));
    }
    super::key(event, status, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size::new(200.0, 100.0);

    #[test]
    fn crops_keep_whole_pixels_and_all_of_the_image_is_no_crop() {
        let part = Rectangle::new(Point::new(10.4, 20.6), Size::new(50.2, 30.0));
        assert_eq!(
            kept(part, SIZE),
            Some(kurbo::Rect::new(10.0, 21.0, 61.0, 51.0))
        );
        let all = Rectangle::new(Point::new(0.2, -0.3), Size::new(199.9, 100.1));
        assert_eq!(kept(all, SIZE), None);
        let sliver = Rectangle::new(Point::new(5.0, 5.0), Size::new(0.3, 40.0));
        assert_eq!(kept(sliver, SIZE), None);
    }

    #[test]
    fn a_click_keeps_the_crop_from_before() {
        let start = Rectangle::new(Point::new(20.0, 20.0), Size::new(100.0, 50.0));
        let mut cropping = Cropping::new(start);
        let outside = kurbo::Point::new(180.0, 90.0);
        cropping.follow(Pointer::Pressed(outside), 1.0, SIZE);
        cropping.follow(Pointer::Released(outside), 1.0, SIZE);
        assert_eq!(cropping.rect(), Some(start));
    }

    #[test]
    fn a_drag_draws_a_new_crop_inside_the_image() {
        let mut cropping = Cropping::new(Rectangle::new(Point::ORIGIN, Size::new(10.0, 10.0)));
        cropping.follow(Pointer::Pressed(kurbo::Point::new(50.0, 40.0)), 1.0, SIZE);
        cropping.follow(Pointer::Moved(kurbo::Point::new(400.0, 70.0)), 1.0, SIZE);
        assert!(cropping.dragging());
        cropping.follow(Pointer::Released(kurbo::Point::new(400.0, 70.0)), 1.0, SIZE);
        assert!(!cropping.dragging());
        assert_eq!(
            cropping.rect(),
            Some(Rectangle::new(
                Point::new(50.0, 40.0),
                Size::new(150.0, 30.0)
            ))
        );
    }
}
