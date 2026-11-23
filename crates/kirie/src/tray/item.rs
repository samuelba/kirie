//! The tray icon itself, served as `org.kde.StatusNotifierItem`.

#![allow(clippy::used_underscore_binding)]

use iced::futures::channel::mpsc::UnboundedSender;
use zbus::ObjectPath;
use zbus::interface;

use super::{Command, Token};

pub const PATH: &str = "/StatusNotifierItem";

const ICON_NAME: &str = crate::APP_ID;
const TITLE: &str = "Kirie";

type Pixmap = (i32, i32, Vec<u8>);

#[derive(Debug)]
pub struct Item {
    commands: UnboundedSender<Command>,
    token: Token,
}

impl Item {
    pub fn new(commands: UnboundedSender<Command>, token: Token) -> Self {
        Self { commands, token }
    }
}

#[allow(clippy::unused_self)]
#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    /// A click on the icon, where the panel does not show the menu for it.
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.commands.unbounded_send(Command::Open);
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.commands.unbounded_send(Command::Capture);
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    /// The panel's activation token for the click that follows, which lets the window it opens
    /// come to the front.
    fn provide_xdg_activation_token(&self, token: String) {
        if let Ok(mut slot) = self.token.lock() {
            *slot = Some(token);
        }
    }

    #[zbus(property)]
    fn category(&self) -> &'static str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &'static str {
        "kirie"
    }

    #[zbus(property)]
    fn title(&self) -> &'static str {
        TITLE
    }

    #[zbus(property)]
    fn status(&self) -> &'static str {
        "Active"
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> &'static str {
        ICON_NAME
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        Vec::new()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> &'static str {
        ""
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        (String::new(), Vec::new(), TITLE.to_owned(), String::new())
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked(super::menu::PATH)
    }
}

#[cfg(test)]
mod tests {
    use iced::futures::channel::mpsc;

    use super::*;

    #[test]
    fn a_click_opens_kirie_and_a_middle_click_takes_a_screenshot() {
        let (commands, mut received) = mpsc::unbounded();
        let item = Item::new(commands, Token::default());
        item.activate(0, 0);
        item.secondary_activate(0, 0);
        assert_eq!(received.try_recv().expect("queued"), Command::Open);
        assert_eq!(received.try_recv().expect("queued"), Command::Capture);
    }

    #[test]
    fn the_panel_s_token_is_kept_for_the_next_click() {
        let (commands, _received) = mpsc::unbounded();
        let token = Token::default();
        let item = Item::new(commands, token.clone());
        item.provide_xdg_activation_token("abc".into());
        assert_eq!(super::super::take(&token).as_deref(), Some("abc"));
        assert_eq!(super::super::take(&token), None, "a token is used once");
    }
}
