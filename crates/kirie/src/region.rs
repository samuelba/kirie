//! The region the overlay remembers, selected again in the next screenshot until it changes.

use iced::{Point, Rectangle, Size};
use serde::{Deserialize, Serialize};

const FILE: &str = "region.ron";

/// A region in desktop (logical) coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Stored {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl From<Rectangle> for Stored {
    fn from(rect: Rectangle) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

impl From<Stored> for Rectangle {
    fn from(stored: Stored) -> Self {
        Rectangle::new(
            Point::new(stored.x, stored.y),
            Size::new(stored.width, stored.height),
        )
    }
}

/// The remembered region, if there is one.
pub fn load() -> Option<Rectangle> {
    let path = crate::store::state_path(FILE)?;
    crate::store::read::<Stored>(&path).map(Rectangle::from)
}

pub fn remember(region: Rectangle) {
    let saved = crate::store::state_path(FILE)
        .ok_or_else(|| "cannot find the state folder".to_owned())
        .and_then(|path| crate::store::save(&path, &Stored::from(region)));
    if let Err(error) = saved {
        tracing::warn!(%error, "region not remembered");
    }
}

pub fn forget() {
    if let Some(path) = crate::store::state_path(FILE)
        && let Err(error) = crate::store::remove(&path)
    {
        tracing::warn!(%error, "region not forgotten");
    }
}

/// `region` if all of it lies on `desktop`, so a region from other screens is not selected.
pub fn within(region: Rectangle, desktop: Rectangle) -> Option<Rectangle> {
    let fits = region.width >= 1.0
        && region.height >= 1.0
        && region.x >= desktop.x
        && region.y >= desktop.y
        && region.x + region.width <= desktop.x + desktop.width
        && region.y + region.height <= desktop.y + desktop.height;
    fits.then_some(region)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 3840.0,
        height: 1080.0,
    };

    #[test]
    fn only_regions_on_the_desktop_are_selected() {
        let region = Rectangle::new(Point::new(2000.0, 100.0), Size::new(800.0, 600.0));
        assert_eq!(within(region, DESKTOP), Some(region));
        assert_eq!(within(DESKTOP, DESKTOP), Some(DESKTOP));
        let beyond = Rectangle::new(Point::new(3500.0, 100.0), Size::new(800.0, 600.0));
        assert_eq!(within(beyond, DESKTOP), None);
        let above = Rectangle::new(Point::new(10.0, -5.0), Size::new(80.0, 60.0));
        assert_eq!(within(above, DESKTOP), None);
        let empty = Rectangle::new(Point::new(10.0, 10.0), Size::new(0.0, 60.0));
        assert_eq!(within(empty, DESKTOP), None);
    }

    #[test]
    fn regions_survive_a_round_trip() {
        let region = Rectangle::new(Point::new(12.5, 40.0), Size::new(1280.0, 720.0));
        let text = ron::to_string(&Stored::from(region)).unwrap();
        let back: Stored = ron::from_str(&text).unwrap();
        assert_eq!(Rectangle::from(back), region);
    }
}
