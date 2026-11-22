//! The editor window: a header bar, the tool rail, the canvas and the inspector.

use iced::widget::{Space, column, container, row, rule, stack, svg, text, text_input};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use kirie_annotate::Tool;
use kirie_ui::icon::Icon;
use kirie_ui::widget::{divider, floating, icon_button, primary_button};
use kirie_ui::{Palette, Scheme, space};

use super::canvas::{Backdrop, Board};
use super::{Editor, Message};
use crate::controls;
use crate::picker::{self, Picker};

pub const TEXT_INPUT: &str = "kirie-editor-text";
const APP_ICON: &[u8] =
    include_bytes!("../../../../packaging/icons/hicolor/scalable/apps/dev.samuelba.Kirie.svg");
const INSPECTOR: f32 = 248.0;

impl Editor {
    pub fn view(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let body = row![self.rail(), self.center(), self.inspector()];
        container(column![self.header(), body.height(Length::Fill)])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(palette.background)),
                text_color: Some(palette.text),
                ..container::Style::default()
            })
            .into()
    }

    fn header(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let has_image = self.image.is_some();
        let brand = row![
            svg(svg::Handle::from_memory(APP_ICON)).width(26).height(26),
            text("Kirie").size(16),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        let history = row![
            icon_button(
                Icon::Undo,
                "Undo (Ctrl+Z)",
                palette,
                false,
                self.history.can_undo().then_some(Message::Undo)
            ),
            icon_button(
                Icon::Redo,
                "Redo (Ctrl+Shift+Z)",
                palette,
                false,
                self.history.can_redo().then_some(Message::Redo)
            ),
        ]
        .spacing(2);
        let theme = icon_button(
            match palette.scheme {
                Scheme::Dark => Icon::Light,
                Scheme::Light => Icon::Dark,
            },
            "Switch light and dark",
            palette,
            false,
            Some(Message::ToggleScheme),
        );
        let capture = icon_button(
            Icon::Capture,
            "Take a screenshot",
            palette,
            false,
            Some(Message::Capture),
        );
        let open = icon_button(
            Icon::Open,
            "Open an image (Ctrl+O)",
            palette,
            false,
            Some(Message::Open),
        );
        let mut actions = row![open, capture, divider(palette), history, divider(palette)]
            .spacing(space::S)
            .align_y(Alignment::Center);
        if has_image {
            actions = actions
                .push(icon_button(
                    Icon::Project,
                    "Save editable project (Ctrl+Shift+S)",
                    palette,
                    false,
                    Some(Message::SaveProject),
                ))
                .push(icon_button(
                    Icon::Save,
                    "Save to Pictures (Ctrl+S)",
                    palette,
                    false,
                    Some(Message::Save),
                ))
                .push(primary_button(Icon::Copy, "Copy", palette, Message::Copy));
        }
        actions = actions.push(divider(palette)).push(theme);
        let styles = controls::styles(self.document().style, palette, Message::PickStyle);
        container(
            row![
                brand,
                Space::new().width(Length::Fill),
                styles,
                Space::new().width(Length::Fill),
                actions
            ]
            .align_y(Alignment::Center),
        )
        .padding([space::S, space::L])
        .style(panel(palette))
        .into()
    }

    fn rail(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let tools = Tool::ALL.into_iter().map(|tool| {
            icon_button(
                controls::tool_icon(tool),
                tool.label(),
                palette,
                self.tool == tool,
                Some(Message::PickTool(tool)),
            )
        });
        container(column(tools).spacing(2))
            .padding(space::S)
            .height(Length::Fill)
            .style(panel(palette))
            .into()
    }

    fn center(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let mut layers = if self.image.is_some() {
            stack![
                iced::widget::canvas(Backdrop { editor: self })
                    .width(Length::Fill)
                    .height(Length::Fill),
                iced::widget::canvas(Board { editor: self })
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
        } else {
            stack![self.empty()]
        };
        let mut floating_bits = column![].spacing(space::S).align_x(Alignment::Center);
        if let Some(typing) = &self.typing {
            floating_bits = floating_bits.push(
                container(
                    row![
                        text_input("Type here", &typing.text)
                            .id(TEXT_INPUT)
                            .on_input(Message::TextChanged)
                            .on_submit(Message::TextDone)
                            .width(320)
                            .padding([space::XS + 2.0, space::S]),
                        text("Enter to place").size(12).color(palette.muted),
                    ]
                    .spacing(space::M)
                    .align_y(Alignment::Center),
                )
                .padding(space::S)
                .style(floating(palette)),
            );
        }
        if let Some(toast) = &self.toast {
            let color = if toast.error {
                palette.danger
            } else {
                palette.text
            };
            floating_bits = floating_bits.push(
                container(text(toast.text.clone()).size(13).color(color))
                    .padding([space::S, space::L])
                    .style(floating(palette)),
            );
        }
        layers = layers.push(
            container(floating_bits)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::End)
                .padding(space::XL),
        );
        container(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn empty(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let card = column![
            svg(svg::Handle::from_memory(APP_ICON)).width(96).height(96),
            text("Cut a piece out of your screen").size(22),
            text("Take a screenshot, open an image, or paste one with Ctrl+V.")
                .size(14)
                .color(palette.muted),
            row![
                primary_button(
                    Icon::Capture,
                    "Take a screenshot",
                    palette,
                    Message::Capture
                ),
                icon_button(
                    Icon::Open,
                    "Open an image (Ctrl+O)",
                    palette,
                    false,
                    Some(Message::Open)
                ),
            ]
            .spacing(space::S)
            .align_y(Alignment::Center),
        ]
        .spacing(space::M)
        .align_x(Alignment::Center);
        container(card).center(Length::Fill).into()
    }

    fn inspector(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let section = |title: &'static str, content: Element<'static, Message>| {
            column![text(title).size(12).color(palette.muted), content].spacing(space::S)
        };
        let fill = row![
            icon_button(
                Icon::Fill,
                "Fill shapes",
                palette,
                self.paint.filled,
                Some(Message::ToggleFill)
            ),
            text(if self.paint.filled {
                "Shapes are filled"
            } else {
                "Outlines only"
            })
            .size(13),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        let zoom = row![
            icon_button(
                Icon::Fit,
                "Fit (Ctrl+0)",
                palette,
                self.viewport.is_fitted(),
                Some(Message::ZoomFit)
            ),
            icon_button(
                Icon::Image,
                "Actual size (Ctrl+1)",
                palette,
                false,
                Some(Message::ZoomActual)
            ),
            text("Ctrl+scroll to zoom").size(12).color(palette.muted),
        ]
        .spacing(space::XS)
        .align_y(Alignment::Center);
        let mut colors = column![
            controls::colors(
                &self.swatches,
                self.paint,
                palette,
                self.picker.as_ref().map(Picker::target),
                Message::PickColor,
                Message::EditColor,
            )
            .wrap()
        ]
        .spacing(space::M);
        if let Some(open) = &self.picker {
            colors = colors.push(picker::view(open, palette).map(Message::Picker));
        }
        let mut content = column![
            column![text("Colour").size(12).color(palette.muted), colors].spacing(space::S),
            section(
                "Weight",
                controls::weights(self.paint, palette, Message::PickWeight)
            ),
            section("Fill", fill.into()),
            rule::horizontal(1),
            section("View", zoom.into()),
        ]
        .spacing(space::L);
        if let Some((image, _)) = &self.image {
            content = content.push(section(
                "Image",
                text(format!("{} × {} px", image.width(), image.height()))
                    .size(13)
                    .into(),
            ));
        }
        if self.picked.is_some() {
            content = content.push(icon_button(
                Icon::Delete,
                "Delete mark (Del)",
                palette,
                false,
                Some(Message::Delete),
            ));
        }
        container(content)
            .width(INSPECTOR)
            .height(Length::Fill)
            .padding(Padding::new(space::L))
            .style(panel(palette))
            .into()
    }
}

fn panel(palette: Palette) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(palette.surface[0])),
        border: Border {
            color: palette.border,
            width: 1.0,
            radius: 0.0.into(),
        },
        text_color: Some(palette.text),
        ..container::Style::default()
    }
}
