//! The region picked on the desktop, and remembering it: a remembered region is selected again
//! in the next screenshot until it changes.

use iced::{Point, Rectangle, Size, Vector};
use serde::{Deserialize, Serialize};

use super::Pointer;
use crate::annotate::selection::{Hit, Release, Selection};

const FILE: &str = "region.ron";

/// Picking the region by keys and buttons.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// Moves the region by whole logical pixels.
    Nudge(Vector),
    SelectAll,
    /// Selects the screen with this index.
    SelectScreen(usize),
    /// Keeps the region for the next screenshot, or forgets it.
    ToggleRemember,
}

/// The region picked on the desktop, and the one kept for the next screenshot.
#[derive(Debug, Clone, Copy, Default)]
pub struct Region {
    selection: Selection,
    /// The region kept for the next screenshot, while the selection is unchanged.
    remembered: Option<Rectangle>,
}

impl Region {
    /// The remembered region, selected, if all of it lies on `desktop`.
    pub fn recalled(desktop: Rectangle) -> Self {
        let remembered = load().and_then(|region| within(region, desktop));
        Self {
            selection: remembered.map(Selection::selected).unwrap_or_default(),
            remembered,
        }
    }

    /// `area` selected, with nothing remembered and nothing forgotten.
    pub fn of(area: Rectangle) -> Self {
        Self {
            selection: Selection::selected(area),
            remembered: None,
        }
    }

    pub fn rect(&self) -> Option<Rectangle> {
        self.selection.rect()
    }

    pub fn dragging(&self) -> bool {
        self.selection.dragging()
    }

    pub fn hit(&self, at: Point, reach: f32) -> Option<Hit> {
        self.selection.hit(at, reach)
    }

    pub fn is_remembered(&self) -> bool {
        self.remembered.is_some()
    }

    /// Selects `area`, forgetting the remembered region if it differs.
    pub fn select(&mut self, area: Rectangle) {
        self.selection = Selection::selected(area);
        self.keep_remembered();
    }

    /// Moves the region `by` whole logical pixels, keeping it on `desktop`.
    pub fn nudge(&mut self, by: Vector, desktop: Rectangle) {
        self.selection = self.selection.nudge(by, desktop);
        self.keep_remembered();
    }

    /// Follows the pointer: a press grabs a handle or the region, or starts a new one, which a
    /// drag draws on `desktop`. A click selects what `picked` finds under it.
    pub fn follow(
        &mut self,
        pointer: Pointer,
        reach: f32,
        desktop: Rectangle,
        picked: impl FnOnce(Point) -> Option<Rectangle>,
    ) {
        match pointer {
            Pointer::Pressed(at) => self.selection = self.selection.press(at, reach),
            Pointer::Moved(at) => self.selection = self.selection.drag_to(at, desktop),
            Pointer::Released(at) => {
                let (selection, release) = self.selection.release(at);
                self.selection = selection;
                if let Release::Click(at) = release
                    && let Some(area) = picked(at)
                {
                    self.selection = Selection::selected(area);
                }
            }
        }
        self.keep_remembered();
    }

    /// Remembers the region for the next screenshot, or forgets the one remembered.
    pub fn toggle_remember(&mut self) {
        if self.remembered.take().is_some() {
            forget();
        } else if let Some(region) = self.selection.rect() {
            remember(region);
            self.remembered = Some(region);
        }
    }

    /// Forgets the remembered region once the selection is changed.
    fn keep_remembered(&mut self) {
        if self
            .remembered
            .is_some_and(|region| self.selection.rect() != Some(region))
        {
            self.remembered = None;
            forget();
        }
    }
}

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
fn load() -> Option<Rectangle> {
    let path = crate::storage::store::state_path(FILE)?;
    crate::storage::store::read::<Stored>(&path).map(Rectangle::from)
}

fn remember(region: Rectangle) {
    let saved = crate::storage::store::state_path(FILE)
        .ok_or_else(|| "cannot find the state folder".to_owned())
        .and_then(|path| crate::storage::store::save(&path, &Stored::from(region)));
    if let Err(error) = saved {
        tracing::warn!(%error, "region not remembered");
    }
}

fn forget() {
    if let Some(path) = crate::storage::store::state_path(FILE)
        && let Err(error) = crate::storage::store::remove(&path)
    {
        tracing::warn!(%error, "region not forgotten");
    }
}

/// `region` if all of it lies on `desktop`, so a region from other screens is not selected.
fn within(region: Rectangle, desktop: Rectangle) -> Option<Rectangle> {
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

    const REACH: f32 = 8.0;
    const SCREEN: Rectangle = Rectangle {
        x: 1920.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn a_click_takes_what_is_under_it_and_a_drag_draws_a_region() {
        let mut region = Region::default();
        let at = Point::new(2000.0, 500.0);
        region.follow(Pointer::Pressed(at), REACH, DESKTOP, |_| None);
        region.follow(Pointer::Released(at), REACH, DESKTOP, |under| {
            SCREEN.contains(under).then_some(SCREEN)
        });
        assert_eq!(region.rect(), Some(SCREEN));
        let mut region = Region::default();
        region.follow(
            Pointer::Pressed(Point::new(10.0, 10.0)),
            8.0,
            DESKTOP,
            |_| None,
        );
        region.follow(
            Pointer::Moved(Point::new(-50.0, 210.0)),
            8.0,
            DESKTOP,
            |_| None,
        );
        assert!(region.dragging());
        region.follow(
            Pointer::Released(Point::new(-50.0, 210.0)),
            8.0,
            DESKTOP,
            |_| panic!("a drag is not a click"),
        );
        assert_eq!(
            region.rect(),
            Some(Rectangle::new(
                Point::new(0.0, 10.0),
                Size::new(10.0, 200.0)
            ))
        );
    }

    #[test]
    fn a_remembered_region_is_forgotten_once_it_changes() {
        let mut region = Region::of(SCREEN);
        region.toggle_remember();
        assert!(region.is_remembered());
        region.select(SCREEN);
        assert!(region.is_remembered(), "the same region stays remembered");
        region.nudge(Vector::new(-5.0, 0.0), DESKTOP);
        assert_eq!(region.rect().map(|rect| rect.x), Some(1915.0));
        assert!(!region.is_remembered());
        region.toggle_remember();
        region.toggle_remember();
        assert!(!region.is_remembered());
        assert_eq!(region.rect().map(|rect| rect.x), Some(1915.0));
    }

    #[test]
    fn nothing_selected_remembers_nothing() {
        let mut region = Region::default();
        region.toggle_remember();
        assert!(!region.is_remembered());
    }

    #[test]
    fn a_gesture_forgets_the_remembered_region_once_it_moves_it() {
        let mut region = Region::of(SCREEN);
        region.toggle_remember();
        let inside = Point::new(2500.0, 500.0);
        region.follow(Pointer::Pressed(inside), REACH, DESKTOP, |_| None);
        assert!(region.is_remembered(), "a press alone changes nothing");
        region.follow(
            Pointer::Moved(Point::new(2400.0, 500.0)),
            REACH,
            DESKTOP,
            |_| None,
        );
        assert!(!region.is_remembered());
    }

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
