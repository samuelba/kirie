//! What Kirie is, its version, and where to find out more.

use iced::widget::{Space, column, container, opaque, row, svg, text};
use iced::{Alignment, Background, Color, Element, Length};
use kirie_ui::icon::Icon;
use kirie_ui::space;
use kirie_ui::widget::{floating, primary_button, secondary_button};

use super::view::APP_ICON;
use super::{Editor, Message};

pub const WEBSITE: &str = "https://samuelba.dev/kirie";
pub const SOURCE: &str = env!("CARGO_PKG_REPOSITORY");
const VERSION: &str = env!("CARGO_PKG_VERSION");

impl Editor {
    /// The about card, over everything else, while it is open.
    pub(super) fn about_dialog(&self) -> Option<Element<'_, Message>> {
        if !self.about {
            return None;
        }
        let palette = self.palette;
        let links = row![
            secondary_button("Website", palette, Message::OpenLink(WEBSITE)),
            secondary_button("Source code", palette, Message::OpenLink(SOURCE)),
            Space::new().width(Length::Fill),
            primary_button(Icon::Done, "Close", palette, Message::About(false)),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        let card = container(
            column![
                svg(svg::Handle::from_memory(APP_ICON)).width(72).height(72),
                text("Kirie").size(22),
                text(format!("Version {VERSION}"))
                    .size(13)
                    .color(palette.muted),
                text("Kirie (切り絵, Japanese paper-cut art) is a screenshot and annotation tool for Wayland. Cut a piece out of your screen, mark it up right there or later in the editor, and copy or save it.").size(14),
                text("MIT License · © Samuel Bachmann")
                    .size(12)
                    .color(palette.muted),
                links,
            ]
            .spacing(space::M)
            .align_x(Alignment::Center),
        )
        .width(460)
        .padding(space::XL)
        .style(floating(palette));
        Some(opaque(container(card).center(Length::Fill).style(|_| {
            container::Style {
                background: Some(Background::Color(Color::BLACK.scale_alpha(0.4))),
                ..container::Style::default()
            }
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_about_card_opens_and_closes_with_escape() {
        let mut editor = Editor::boot(None).0;
        assert!(editor.about_dialog().is_none());
        let _ = editor.update(Message::About(true));
        assert!(editor.about_dialog().is_some());
        let _ = editor.update(Message::Escape);
        assert!(editor.about_dialog().is_none());
    }

    #[test]
    fn the_links_lead_to_kirie() {
        assert!(WEBSITE.starts_with("https://"));
        assert!(SOURCE.starts_with("https://github.com/"));
    }
}
