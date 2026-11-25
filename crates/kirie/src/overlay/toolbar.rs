//! Moving the toolbar by its grip, across the screens.

use iced::{Event as IcedEvent, Point, Vector, event, mouse, window};
use kirie_ui::space;

use super::Message;

/// Moving the toolbar by its grip.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// The pointer over the toolbar's grip, in the grip's coordinates.
    Hover(Point),
    /// The grip was pressed.
    Grab,
    /// The pointer while the toolbar is dragged, in the coordinates of a surface.
    Moved(window::Id, Point),
    Dropped,
}

/// Where the toolbar was dragged to, and the drag under way.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Drag {
    /// Where the toolbar was dragged to, in desktop coordinates.
    at: Option<Point>,
    /// While the toolbar is dragged: where it is held, from its top left.
    held: Option<Vector>,
    /// The pointer over the grip, from the grip's top left.
    grip: Point,
}

impl Drag {
    /// Where the toolbar was dragged to, in desktop coordinates, if it was.
    pub fn at(&self) -> Option<Point> {
        self.at
    }

    /// Whether the toolbar is held by its grip.
    pub fn dragging(&self) -> bool {
        self.held.is_some()
    }

    /// Notes where the pointer is over the grip, in the grip's coordinates.
    pub fn hover(&mut self, grip: Point) {
        self.grip = grip;
    }

    /// Takes the toolbar, placed at `placed` in desktop coordinates, by the grip under the
    /// pointer; a toolbar not shown cannot be taken.
    pub fn grab(&mut self, placed: Option<Point>) {
        if let Some(placed) = placed {
            self.at = Some(placed);
            // The grip sits inside the toolbar's padding.
            self.held = Some(Vector::new(self.grip.x + space::S, self.grip.y + space::S));
        }
    }

    /// Follows the pointer, at `pointer` in desktop coordinates.
    pub fn follow(&mut self, pointer: Point) {
        if let Some(held) = self.held {
            self.at = Some(pointer - held);
        }
    }

    /// Lets go of the toolbar, which stays where it was `placed` on its screen.
    pub fn release(&mut self, placed: Option<Point>) {
        self.held = None;
        self.at = placed;
    }
}

/// The pointer moving and letting go while the toolbar is dragged.
#[allow(clippy::needless_pass_by_value)]
pub fn dragging(event: IcedEvent, _: event::Status, id: window::Id) -> Option<Message> {
    match event {
        IcedEvent::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::Toolbar(Event::Moved(id, position)))
        }
        IcedEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::Toolbar(Event::Dropped))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbar_follows_the_point_it_is_held_by() {
        let mut drag = Drag::default();
        drag.hover(Point::new(4.0, 20.0));
        drag.grab(Some(Point::new(100.0, 500.0)));
        assert!(drag.dragging());
        assert_eq!(drag.at(), Some(Point::new(100.0, 500.0)));
        drag.follow(Point::new(300.0, 300.0));
        assert_eq!(
            drag.at(),
            Some(Point::new(
                300.0 - (4.0 + space::S),
                300.0 - (20.0 + space::S)
            ))
        );
        drag.hover(Point::new(9.0, 9.0));
        drag.follow(Point::new(300.0, 300.0));
        assert_eq!(
            drag.at(),
            Some(Point::new(
                300.0 - (4.0 + space::S),
                300.0 - (20.0 + space::S)
            )),
            "the hold stays as it was grabbed"
        );
        drag.release(Some(Point::new(12.0, 12.0)));
        assert!(!drag.dragging());
        assert_eq!(drag.at(), Some(Point::new(12.0, 12.0)));
        drag.release(None);
        assert_eq!(drag.at(), None, "a toolbar no longer shown is placed anew");
    }

    #[test]
    fn a_toolbar_not_shown_is_not_taken() {
        let mut drag = Drag::default();
        drag.grab(None);
        assert!(!drag.dragging());
        drag.follow(Point::new(300.0, 300.0));
        assert_eq!(drag, Drag::default());
    }
}
