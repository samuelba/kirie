//! The editor window: a header bar, the tool rail, the canvas and the inspector.

use iced::widget::{Space, column, container, row, rule, scrollable, stack, svg, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use kirie_annotate::{StyleId, Tool};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{
    divider, floating, icon_button, primary_button, secondary_button, small_icon_button,
    small_pick_list,
};
use kirie_ui::{Palette, Scheme, space};

use super::canvas::{Backdrop, Board};
use super::viewport::Zoom;
use super::{Editor, Message};
use crate::controls;
use crate::picker::{self, Picker};

pub const TEXT_INPUT: &str = "kirie-editor-text";
const APP_ICON: &[u8] =
    include_bytes!("../../../../packaging/icons/hicolor/scalable/apps/dev.samuelba.Kirie.svg");
const INSPECTOR: f32 = 248.0;

impl Editor {
    pub fn view(&self, _: iced::window::Id) -> Element<'_, Message> {
        let palette = self.palette;
        let body = row![self.rail(), self.center(), self.inspector()];
        let window = container(column![
            self.header(),
            body.height(Length::Fill),
            self.status_bar()
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.background)),
            text_color: Some(palette.text),
            ..container::Style::default()
        });
        match self.closing_dialog() {
            Some(dialog) => stack![window, dialog].into(),
            None => window.into(),
        }
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
        let mut actions = row![open, capture, divider(palette), history]
            .spacing(space::S)
            .align_y(Alignment::Center);
        if has_image {
            actions = actions
                .push(divider(palette))
                .push(icon_button(
                    Icon::Project,
                    "Save editable project, with the original under the marks (Ctrl+Shift+S)",
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
        let document = self.document();
        let styles = controls::styles(document.style, palette, Message::PickStyle);
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
                self.tool == tool && self.cropping.is_none(),
                Some(Message::PickTool(tool)),
            )
        });
        let crop = icon_button(
            Icon::Crop,
            "Crop (C)",
            palette,
            self.cropping.is_some(),
            self.image.is_some().then_some(Message::Crop),
        );
        container(
            column(tools)
                .push(container(rule::horizontal(1)).width(kirie_ui::widget::CONTROL))
                .push(crop)
                .spacing(2),
        )
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
        if self.cropping.is_some() {
            floating_bits = floating_bits.push(self.crop_bar());
        }
        if let Some(typing) = &self.typing {
            floating_bits = floating_bits.push(
                container(
                    row![
                        crate::typing::text_box(
                            typing,
                            TEXT_INPUT,
                            palette,
                            Message::TextEdit,
                            Message::TextDone,
                        ),
                        column![
                            text("Enter to place").size(12).color(palette.muted),
                            text("Shift+Enter for a new line")
                                .size(12)
                                .color(palette.muted),
                        ],
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

    /// What cropping is about, and the buttons that end it.
    fn crop_bar(&self) -> Element<'_, Message> {
        let palette = self.palette;
        container(
            row![
                column![
                    text("Drag the frame to keep a part").size(12),
                    text("Enter to crop, Esc to cancel")
                        .size(12)
                        .color(palette.muted),
                ],
                secondary_button("Whole image", palette, Message::CropReset),
                secondary_button("Cancel", palette, Message::CropCancel),
                primary_button(Icon::Crop, "Crop", palette, Message::CropDone),
            ]
            .spacing(space::M)
            .align_y(Alignment::Center),
        )
        .padding(space::S)
        .style(floating(palette))
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
        let mut content = self
            .paint_sections()
            .push(rule::horizontal(1))
            .push(section(palette, "View", zoom.into()));
        if self.image.is_some() {
            let document = self.document();
            content = content.push(section(
                palette,
                "Frame",
                super::frame::view(document.frame, document.unit, palette, Message::Frame),
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
        container(scrollable(
            container(content).padding(Padding::new(space::L)),
        ))
        .width(INSPECTOR)
        .height(Length::Fill)
        .style(panel(palette))
        .into()
    }

    /// The tool, where the pointer is on the image, the zoom and the image's size.
    fn status_bar(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let muted = |words: String| text(words).size(12).color(palette.muted);
        let tool = row![
            svg(if self.cropping.is_some() {
                Icon::Crop.handle()
            } else {
                controls::tool_icon(self.tool).handle()
            })
            .width(14)
            .height(14)
            .style(move |_, _| svg::Style {
                color: Some(palette.muted),
            }),
            muted(
                if self.cropping.is_some() {
                    "Crop"
                } else {
                    self.tool.name()
                }
                .to_owned()
            ),
        ]
        .spacing(space::XS)
        .align_y(Alignment::Center);
        let mut bar = row![tool, Space::new().width(Length::Fill)]
            .spacing(space::L)
            .align_y(Alignment::Center);
        if let Some((image, _)) = &self.image {
            if let Some((x, y)) = self.hover.map(super::pixel) {
                bar = bar.push(muted(format!("{x}, {y} px")));
            }
            #[allow(clippy::cast_possible_truncation)]
            let saved = self.crop().map_or_else(
                || {
                    let canvas = self.document().canvas();
                    (canvas.width() as i64, canvas.height() as i64)
                },
                |crop| (crop.width.round() as i64, crop.height.round() as i64),
            );
            let whole = (i64::from(image.width()), i64::from(image.height()));
            let size = if saved == whole {
                format!("{} × {} px", whole.0, whole.1)
            } else {
                format!(
                    "{} × {} px, from {} × {}",
                    saved.0, saved.1, whole.0, whole.1
                )
            };
            bar = bar.push(self.zoom_control()).push(muted(size));
        }
        container(bar)
            .padding([space::XS, space::L])
            .width(Length::Fill)
            .style(panel(palette))
            .into()
    }

    /// Zoom out, the zoom with a list of levels to pick, and zoom in.
    fn zoom_control(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let scale = self.viewport.scale(self.content(), self.canvas);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let percent = (scale * 100.0).round() as u32;
        let shown = if self.viewport.is_fitted() {
            ZoomChoice::Shown(format!("Fit {percent}%"), Zoom::Fit)
        } else {
            ZoomChoice::Shown(format!("{percent}%"), Zoom::Percent(percent))
        };
        let levels = Zoom::CHOICES.into_iter().map(ZoomChoice::Level).collect();
        row![
            small_icon_button(
                Icon::ZoomOut,
                "Zoom out (Ctrl+-)",
                palette,
                Some(Message::ZoomOut)
            ),
            // A fitted image is never enlarged, so "Fit 100%" is the longest label it shows.
            small_pick_list(levels, shown, "Fit 100%", palette, |choice| {
                Message::ZoomTo(choice.zoom())
            }),
            small_icon_button(
                Icon::ZoomIn,
                "Zoom in (Ctrl++)",
                palette,
                Some(Message::ZoomIn)
            ),
        ]
        .spacing(2)
        .align_y(Alignment::Center)
        .into()
    }

    /// The color, and the text look, the spotlight, or the weight, fill and shadow, of the
    /// picked mark or the next.
    fn paint_sections(&self) -> iced::widget::Column<'_, Message> {
        let palette = self.palette;
        let paint = self.shown_paint();
        let mut colors = column![
            controls::colors(
                &self.swatches,
                self.document().style,
                paint,
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
        let content = column![section(palette, "Color", colors.into())].spacing(space::L);
        if self.texting() {
            let fallback = self.document().style.font();
            return content.push(section(
                palette,
                "Text",
                column![
                    controls::fonts(paint.text, fallback, palette, Message::Look),
                    controls::text_size(paint.text, palette, 150.0, Message::Look),
                    controls::text_layout(paint.text, palette, Message::Look),
                ]
                .spacing(space::S)
                .into(),
            ));
        }
        if self.spotlighting() {
            return content
                .push(section(
                    palette,
                    "Spotlight",
                    controls::spot_shapes(paint, palette, Message::Spot),
                ))
                .push(section(
                    palette,
                    "Brightness around spotlights",
                    controls::spotlight_brightness(
                        self.document().spotlight_brightness,
                        palette,
                        126.0,
                        Message::Brightness,
                    ),
                ));
        }
        let fill = row![
            icon_button(
                Icon::Fill,
                "Fill shapes",
                palette,
                paint.filled,
                Some(Message::ToggleFill)
            ),
            text(if paint.filled {
                "Shapes are filled"
            } else {
                "Outlines only"
            })
            .size(13),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        let document = self.document();
        let clean = document.style == StyleId::Clean;
        let shadow = row![
            controls::shadow(
                document.style,
                document.shadow,
                palette,
                Message::ToggleShadow
            ),
            text(match (clean, document.shadow) {
                (false, _) => "Only in the Clean style",
                (true, true) => "Marks cast a shadow",
                (true, false) => "No shadow",
            })
            .size(13)
            .color(if clean { palette.text } else { palette.muted }),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        content
            .push(section(palette, "Weight", {
                let (presets, slider) = controls::weights(paint, palette, 150.0, Message::Weight);
                column![presets, slider].spacing(space::S).into()
            }))
            .push(section(palette, "Fill", fill.into()))
            .push(section(palette, "Shadow", shadow.into()))
    }
}

/// A level in the zoom list, or the zoom now, which the list shows.
#[derive(Debug, Clone, PartialEq)]
enum ZoomChoice {
    Level(Zoom),
    Shown(String, Zoom),
}

impl ZoomChoice {
    fn zoom(&self) -> Zoom {
        match self {
            ZoomChoice::Level(zoom) | ZoomChoice::Shown(_, zoom) => *zoom,
        }
    }
}

impl std::fmt::Display for ZoomChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZoomChoice::Level(zoom) => zoom.fmt(f),
            ZoomChoice::Shown(label, _) => f.write_str(label),
        }
    }
}

fn section<'a>(
    palette: Palette,
    title: &'static str,
    content: Element<'a, Message>,
) -> iced::widget::Column<'a, Message> {
    column![text(title).size(12).color(palette.muted), content].spacing(space::S)
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
