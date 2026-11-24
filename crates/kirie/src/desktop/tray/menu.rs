//! The tray icon's menu, served as `com.canonical.dbusmenu`. It changes only when a newer release
//! is found or gone: then the revision goes up and the menu tells the panel.

#![allow(clippy::used_underscore_binding)]

use std::collections::HashMap;

use iced::futures::channel::mpsc::UnboundedSender;
use semver::Version;
use serde::{Deserialize, Serialize};
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant, Type, Variant};

use super::Command;

pub const PATH: &str = "/MenuBar";

const ROOT: i32 = 0;
/// The ids of the entry for a newer release and the separator after it, apart from the others'
/// so that theirs stay the same whether it shows or not.
const UPDATE: i32 = 100;
const UPDATE_SEPARATOR: i32 = 101;

/// One menu entry and, for the root, the entries under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Variant, OwnedVariant)]
pub struct Layout {
    id: i32,
    properties: HashMap<String, OwnedVariant>,
    children: Vec<OwnedVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// Opens the about card, which installs the newer release.
    Update,
    Open,
    Capture,
    /// A screenshot after this many seconds.
    Delayed(u32),
    Separator,
    Settings,
    About,
    Quit,
}

const ENTRIES: [Entry; 8] = [
    Entry::Open,
    Entry::Capture,
    Entry::Delayed(3),
    Entry::Delayed(10),
    Entry::Separator,
    Entry::Settings,
    Entry::About,
    Entry::Quit,
];

impl Entry {
    /// The entry with this id, while the menu shows a newer release if `update` is given.
    fn from_id(id: i32, update: Option<&Version>) -> Option<Self> {
        shown(update)
            .into_iter()
            .find_map(|(shown, entry)| (shown == id).then_some(entry))
    }

    fn label(self, update: Option<&Version>) -> Option<String> {
        match self {
            Self::Update => update.map(|version| format!("Update to Kirie {version}…")),
            Self::Open => Some("Open Kirie".to_owned()),
            Self::Capture => Some("Take a Screenshot".to_owned()),
            Self::Delayed(seconds) => Some(format!("Take a Screenshot in {seconds} Seconds")),
            Self::Separator => None,
            Self::Settings => Some("Settings…".to_owned()),
            Self::About => Some("About Kirie".to_owned()),
            Self::Quit => Some("Quit".to_owned()),
        }
    }

    fn command(self) -> Option<Command> {
        match self {
            Self::Update => Some(Command::Update),
            Self::Open => Some(Command::Open),
            Self::Capture => Some(Command::Capture),
            Self::Delayed(seconds) => Some(Command::CaptureIn(seconds)),
            Self::Separator => None,
            Self::Settings => Some(Command::Settings),
            Self::About => Some(Command::About),
            Self::Quit => Some(Command::Quit),
        }
    }

    fn properties(self, update: Option<&Version>) -> HashMap<String, OwnedVariant> {
        match self.label(update) {
            Some(label) => HashMap::from([property("label", label)]),
            None => HashMap::from([property("type", "separator")]),
        }
    }
}

fn property<'a>(name: &str, value: impl Into<Variant<'a>>) -> (String, OwnedVariant) {
    let value =
        OwnedVariant::try_from(value.into()).expect("plain values have no file descriptors");
    (name.to_owned(), value)
}

fn root_properties() -> HashMap<String, OwnedVariant> {
    HashMap::from([property("children-display", "submenu")])
}

/// The entries the menu shows, with their ids: the newer release first, if there is one.
fn shown(update: Option<&Version>) -> Vec<(i32, Entry)> {
    let release = update.map(|_| {
        [
            (UPDATE, Entry::Update),
            (UPDATE_SEPARATOR, Entry::Separator),
        ]
    });
    release
        .into_iter()
        .flatten()
        .chain((1..).zip(ENTRIES))
        .collect()
}

/// The properties of entry `id`, or `None` for an id the menu does not have.
fn properties_of(id: i32, update: Option<&Version>) -> Option<HashMap<String, OwnedVariant>> {
    if id == ROOT {
        return Some(root_properties());
    }
    Entry::from_id(id, update).map(|entry| entry.properties(update))
}

/// The layout from `parent` down, with its children when `depth` allows any.
pub fn layout(parent: i32, depth: i32, update: Option<&Version>) -> Option<Layout> {
    let properties = properties_of(parent, update)?;
    let children = if parent == ROOT && depth != 0 {
        shown(update)
            .into_iter()
            .map(|(id, entry)| {
                let child = Layout {
                    id,
                    properties: entry.properties(update),
                    children: Vec::new(),
                };
                OwnedVariant::try_from(Variant::from(child)).expect("layouts hold plain values")
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(Layout {
        id: parent,
        properties,
        children,
    })
}

/// What clicking entry `id` asks Kirie to do.
pub fn command_for(id: i32, event: &str, update: Option<&Version>) -> Option<Command> {
    (event == "clicked")
        .then(|| Entry::from_id(id, update))
        .flatten()
        .and_then(Entry::command)
}

fn unknown(id: i32) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(format!("no menu entry {id}"))
}

#[derive(Debug)]
pub struct Menu {
    commands: UnboundedSender<Command>,
    /// The newer release the menu offers.
    update: Option<Version>,
    revision: u32,
}

impl Menu {
    pub fn new(commands: UnboundedSender<Command>) -> Self {
        Self {
            commands,
            update: None,
            revision: 1,
        }
    }

    /// Offers `update`, or no release; the menu's new revision when that changed it.
    pub fn offer(&mut self, update: Option<Version>) -> Option<u32> {
        if self.update == update {
            return None;
        }
        self.update = update;
        self.revision += 1;
        Some(self.revision)
    }

    fn dispatch(&self, id: i32, event: &str) {
        if let Some(command) = command_for(id, event, self.update.as_ref()) {
            let _ = self.commands.unbounded_send(command);
        }
    }
}

#[allow(clippy::unused_self)]
#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let layout = layout(parent_id, recursion_depth, self.update.as_ref())
            .ok_or_else(|| unknown(parent_id))?;
        Ok((self.revision, layout))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let update = self.update.as_ref();
        let ids = if ids.is_empty() {
            std::iter::once(ROOT)
                .chain(shown(update).into_iter().map(|(id, _)| id))
                .collect()
        } else {
            ids
        };
        ids.into_iter()
            .filter_map(|id| properties_of(id, update).map(|properties| (id, properties)))
            .collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedVariant> {
        properties_of(id, self.update.as_ref())
            .ok_or_else(|| unknown(id))?
            .remove(name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: Variant<'_>, _timestamp: u32) {
        self.dispatch(id, event_id);
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedVariant, u32)>) -> Vec<i32> {
        let mut missing = Vec::new();
        for (id, event, _data, _timestamp) in events {
            if properties_of(id, self.update.as_ref()).is_none() {
                missing.push(id);
            }
            self.dispatch(id, &event);
        }
        missing
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    /// Tells the panel that the entries under `parent` changed.
    #[zbus(signal)]
    pub async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &'static str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use iced::futures::channel::mpsc;

    use super::*;

    fn label(properties: &HashMap<String, OwnedVariant>) -> Option<String> {
        properties
            .get("label")
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
    }

    #[test]
    fn the_root_lists_open_capture_separator_about_and_quit() {
        let root = layout(ROOT, -1, None).expect("root");
        let children: Vec<Layout> = root
            .children
            .into_iter()
            .map(|child| Layout::try_from(child).expect("a layout"))
            .collect();
        let labels: Vec<Option<String>> = children
            .iter()
            .map(|child| label(&child.properties))
            .collect();
        assert_eq!(
            labels,
            [
                Some("Open Kirie".to_owned()),
                Some("Take a Screenshot".to_owned()),
                Some("Take a Screenshot in 3 Seconds".to_owned()),
                Some("Take a Screenshot in 10 Seconds".to_owned()),
                None,
                Some("Settings…".to_owned()),
                Some("About Kirie".to_owned()),
                Some("Quit".to_owned()),
            ]
        );
        let ids: Vec<i32> = children.iter().map(|child| child.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(children[4].properties.contains_key("type"));
    }

    #[test]
    fn a_depth_of_zero_leaves_out_the_children() {
        assert_eq!(layout(ROOT, 0, None).expect("root").children, []);
    }

    #[test]
    fn unknown_entries_have_no_layout() {
        assert!(layout(42, -1, None).is_none());
        assert!(layout(-1, -1, None).is_none());
        assert!(layout(UPDATE, -1, None).is_none());
    }

    #[test]
    fn a_newer_release_comes_first_and_leaves_the_other_ids_be() {
        let version = Version::new(9, 9, 9);
        let root = layout(ROOT, -1, Some(&version)).expect("root");
        let children: Vec<Layout> = root
            .children
            .into_iter()
            .map(|child| Layout::try_from(child).expect("a layout"))
            .collect();
        let ids: Vec<i32> = children.iter().map(|child| child.id).collect();
        assert_eq!(ids, [UPDATE, UPDATE_SEPARATOR, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            label(&children[0].properties).as_deref(),
            Some("Update to Kirie 9.9.9…")
        );
        assert_eq!(
            command_for(UPDATE, "clicked", Some(&version)),
            Some(Command::Update)
        );
        assert_eq!(command_for(UPDATE, "clicked", None), None);
        assert_eq!(
            command_for(2, "clicked", Some(&version)),
            Some(Command::Capture)
        );
    }

    #[test]
    fn offering_a_release_moves_the_revision_on_once() {
        let (commands, _received) = mpsc::unbounded();
        let mut menu = Menu::new(commands);
        assert_eq!(menu.offer(Some(Version::new(9, 9, 9))), Some(2));
        assert_eq!(menu.offer(Some(Version::new(9, 9, 9))), None);
        assert_eq!(menu.offer(None), Some(3));
    }

    #[test]
    fn clicks_map_to_commands() {
        assert_eq!(command_for(1, "clicked", None), Some(Command::Open));
        assert_eq!(command_for(2, "clicked", None), Some(Command::Capture));
        assert_eq!(command_for(3, "clicked", None), Some(Command::CaptureIn(3)));
        assert_eq!(
            command_for(4, "clicked", None),
            Some(Command::CaptureIn(10))
        );
        assert_eq!(command_for(6, "clicked", None), Some(Command::Settings));
        assert_eq!(command_for(7, "clicked", None), Some(Command::About));
        assert_eq!(command_for(8, "clicked", None), Some(Command::Quit));
    }

    #[test]
    fn other_events_and_entries_do_nothing() {
        assert_eq!(command_for(5, "clicked", None), None);
        assert_eq!(command_for(1, "hovered", None), None);
        assert_eq!(command_for(ROOT, "clicked", None), None);
        assert_eq!(command_for(9, "clicked", None), None);
    }

    #[test]
    fn a_click_is_forwarded() {
        let (commands, mut received) = mpsc::unbounded();
        let menu = Menu::new(commands);
        let missing = menu.event_group(vec![(2, "clicked".into(), OwnedVariant::from(0_i32), 0)]);
        assert_eq!(missing, Vec::<i32>::new());
        assert_eq!(received.try_recv().expect("queued"), Command::Capture);
    }
}
