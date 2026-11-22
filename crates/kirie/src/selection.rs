//! A region being picked, drawn by dragging, then moved by its inside or resized by its edges
//! and corners: in the overlay, in desktop (logical) coordinates across every screen; in the
//! editor, the part of the image to keep.

use iced::{Point, Rectangle, Size, Vector};

/// Below this distance a press and release count as a click, not a drag.
const CLICK_SLOP: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl Handle {
    pub const ALL: [Handle; 8] = [
        Handle::NorthWest,
        Handle::North,
        Handle::NorthEast,
        Handle::East,
        Handle::SouthEast,
        Handle::South,
        Handle::SouthWest,
        Handle::West,
    ];

    /// Where the handle sits on `rect`.
    pub fn position(self, rect: Rectangle) -> Point {
        let (left, top) = (rect.x, rect.y);
        let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
        let (mid_x, mid_y) = (rect.center_x(), rect.center_y());
        match self {
            Handle::North => Point::new(mid_x, top),
            Handle::South => Point::new(mid_x, bottom),
            Handle::East => Point::new(right, mid_y),
            Handle::West => Point::new(left, mid_y),
            Handle::NorthEast => Point::new(right, top),
            Handle::NorthWest => Point::new(left, top),
            Handle::SouthEast => Point::new(right, bottom),
            Handle::SouthWest => Point::new(left, bottom),
        }
    }

    /// Which edges follow the pointer: (left, top, right, bottom).
    const fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Handle::North => (false, true, false, false),
            Handle::South => (false, false, false, true),
            Handle::East => (false, false, true, false),
            Handle::West => (true, false, false, false),
            Handle::NorthEast => (false, true, true, false),
            Handle::NorthWest => (true, true, false, false),
            Handle::SouthEast => (false, false, true, true),
            Handle::SouthWest => (true, false, false, true),
        }
    }
}

/// What a point on the selection touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Handle(Handle),
    Inside,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Creating { anchor: Point, moved: bool },
    Moving { grab: Point, origin: Rectangle },
    Resizing { handle: Handle, origin: Rectangle },
}

/// What a release ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Release {
    /// A press and release in place, at this point: pick what is under it.
    Click(Point),
    /// A region was drawn, moved or resized.
    Changed,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Selection {
    rect: Option<Rectangle>,
    drag: Option<Drag>,
}

impl Selection {
    pub fn rect(&self) -> Option<Rectangle> {
        self.rect
            .filter(|rect| rect.width >= 1.0 && rect.height >= 1.0)
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn selected(rect: Rectangle) -> Self {
        Self {
            rect: Some(rect),
            drag: None,
        }
    }

    /// What `point` would grab, with handles `tolerance` wide.
    pub fn hit(&self, point: Point, tolerance: f32) -> Option<Hit> {
        let rect = self.rect()?;
        let near =
            |a: Point| (a.x - point.x).abs() <= tolerance && (a.y - point.y).abs() <= tolerance;
        if let Some(handle) = Handle::ALL
            .into_iter()
            .find(|handle| near(handle.position(rect)))
        {
            return Some(Hit::Handle(handle));
        }
        rect.contains(point).then_some(Hit::Inside)
    }

    #[must_use]
    pub fn press(self, point: Point, tolerance: f32) -> Self {
        let drag = match (self.hit(point, tolerance), self.rect()) {
            (Some(Hit::Handle(handle)), Some(origin)) => Drag::Resizing { handle, origin },
            (Some(Hit::Inside), Some(origin)) => Drag::Moving {
                grab: point,
                origin,
            },
            _ => Drag::Creating {
                anchor: point,
                moved: false,
            },
        };
        let rect = match drag {
            Drag::Creating { .. } => None,
            _ => self.rect,
        };
        Self {
            rect,
            drag: Some(drag),
        }
    }

    /// Follows the pointer while dragging, keeping the region inside `bounds`.
    #[must_use]
    pub fn drag_to(self, point: Point, bounds: Rectangle) -> Self {
        let Some(drag) = self.drag else {
            return self;
        };
        let point = clamp_point(point, bounds);
        let (rect, drag) = match drag {
            Drag::Creating { anchor, moved } => {
                let moved = moved || distance(anchor, point) > CLICK_SLOP;
                let rect = moved.then(|| from_corners(anchor, point));
                (rect, Drag::Creating { anchor, moved })
            }
            Drag::Moving { grab, origin } => {
                let moved = origin + (point - grab);
                (Some(clamp_rect(moved, bounds)), drag)
            }
            Drag::Resizing { handle, origin } => (Some(resize(origin, handle, point)), drag),
        };
        Self {
            rect,
            drag: Some(drag),
        }
    }

    #[must_use]
    pub fn release(self, point: Point) -> (Self, Release) {
        let released = Self {
            rect: self.rect,
            drag: None,
        };
        match self.drag {
            Some(Drag::Creating { moved: false, .. }) => (released, Release::Click(point)),
            Some(_) => (released, Release::Changed),
            None => (released, Release::Nothing),
        }
    }

    /// Moves the finished region by whole logical pixels, as the arrow keys do.
    #[must_use]
    pub fn nudge(self, by: Vector, bounds: Rectangle) -> Self {
        match self.rect() {
            Some(rect) if self.drag.is_none() => Self::selected(clamp_rect(rect + by, bounds)),
            _ => self,
        }
    }
}

/// The pointer that shows what a press on `hit` would do.
pub fn interaction(hit: Option<Hit>) -> iced::mouse::Interaction {
    use iced::mouse::Interaction;
    match hit {
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

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn from_corners(a: Point, b: Point) -> Rectangle {
    Rectangle::new(
        Point::new(a.x.min(b.x), a.y.min(b.y)),
        Size::new((a.x - b.x).abs(), (a.y - b.y).abs()),
    )
}

fn resize(origin: Rectangle, handle: Handle, point: Point) -> Rectangle {
    let (left, top, right, bottom) = handle.edges();
    let corner_a = Point::new(
        if left { point.x } else { origin.x },
        if top { point.y } else { origin.y },
    );
    let corner_b = Point::new(
        if right {
            point.x
        } else {
            origin.x + origin.width
        },
        if bottom {
            point.y
        } else {
            origin.y + origin.height
        },
    );
    from_corners(corner_a, corner_b)
}

fn clamp_point(point: Point, bounds: Rectangle) -> Point {
    Point::new(
        point.x.clamp(bounds.x, bounds.x + bounds.width),
        point.y.clamp(bounds.y, bounds.y + bounds.height),
    )
}

fn clamp_rect(rect: Rectangle, bounds: Rectangle) -> Rectangle {
    let x = rect.x.clamp(
        bounds.x,
        (bounds.x + bounds.width - rect.width).max(bounds.x),
    );
    let y = rect.y.clamp(
        bounds.y,
        (bounds.y + bounds.height - rect.height).max(bounds.y),
    );
    Rectangle { x, y, ..rect }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1000.0,
        height: 800.0,
    };

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(Point::new(x, y), Size::new(width, height))
    }

    fn drawn(from: Point, to: Point) -> Selection {
        Selection::default()
            .press(from, 6.0)
            .drag_to(to, BOUNDS)
            .release(to)
            .0
    }

    #[test]
    fn dragging_draws_a_region_in_any_direction() {
        let selection = drawn(Point::new(300.0, 200.0), Point::new(100.0, 50.0));
        assert_eq!(selection.rect(), Some(rect(100.0, 50.0, 200.0, 150.0)));
        assert!(!selection.dragging());
    }

    #[test]
    fn a_press_without_moving_is_a_click() {
        let (selection, release) = Selection::default()
            .press(Point::new(10.0, 10.0), 6.0)
            .drag_to(Point::new(11.0, 11.0), BOUNDS)
            .release(Point::new(11.0, 11.0));
        assert_eq!(release, Release::Click(Point::new(11.0, 11.0)));
        assert_eq!(selection.rect(), None);
        assert_eq!(
            Selection::default().release(Point::ORIGIN).1,
            Release::Nothing
        );
    }

    #[test]
    fn the_inside_moves_and_stays_on_screen() {
        let selection = drawn(Point::new(100.0, 100.0), Point::new(200.0, 200.0));
        assert_eq!(
            selection.hit(Point::new(150.0, 150.0), 6.0),
            Some(Hit::Inside)
        );
        let (moved, release) = selection
            .press(Point::new(150.0, 150.0), 6.0)
            .drag_to(Point::new(5000.0, 160.0), BOUNDS)
            .release(Point::new(5000.0, 160.0));
        assert_eq!(release, Release::Changed);
        assert_eq!(moved.rect(), Some(rect(900.0, 110.0, 100.0, 100.0)));
    }

    #[test]
    fn handles_resize_and_flip_over() {
        let selection = drawn(Point::new(100.0, 100.0), Point::new(200.0, 200.0));
        assert_eq!(
            selection.hit(Point::new(202.0, 198.0), 6.0),
            Some(Hit::Handle(Handle::SouthEast))
        );
        let flipped = selection
            .press(Point::new(200.0, 150.0), 6.0)
            .drag_to(Point::new(50.0, 999.0), BOUNDS);
        assert_eq!(flipped.rect(), Some(rect(50.0, 100.0, 50.0, 100.0)));
        let taller = selection
            .press(Point::new(150.0, 100.0), 6.0)
            .drag_to(Point::new(0.0, 20.0), BOUNDS);
        assert_eq!(taller.rect(), Some(rect(100.0, 20.0, 100.0, 180.0)));
    }

    #[test]
    fn pressing_outside_starts_a_new_region() {
        let selection = drawn(Point::new(100.0, 100.0), Point::new(200.0, 200.0));
        let fresh = selection.press(Point::new(500.0, 500.0), 6.0);
        assert_eq!(fresh.rect(), None);
        assert!(fresh.dragging());
        assert_eq!(fresh.nudge(Vector::new(1.0, 0.0), BOUNDS), fresh);
    }

    #[test]
    fn nudging_moves_inside_bounds() {
        let selection = drawn(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        let nudged = selection.nudge(Vector::new(-1.0, 1.0), BOUNDS);
        assert_eq!(nudged.rect(), Some(rect(0.0, 1.0, 10.0, 10.0)));
        assert_eq!(
            Selection::default()
                .nudge(Vector::new(1.0, 0.0), BOUNDS)
                .rect(),
            None
        );
    }

    #[test]
    fn every_handle_has_its_own_spot() {
        let square = rect(0.0, 0.0, 10.0, 10.0);
        let spots: Vec<Point> = Handle::ALL
            .iter()
            .map(|handle| handle.position(square))
            .collect();
        for (index, spot) in spots.iter().enumerate() {
            assert!(!spots[index + 1..].contains(spot));
        }
    }
}
