//! The editor window: a header bar, the tool rail, the canvas and the inspector.

use iced::widget::{Space, button, column, container, row, rule, scrollable, stack, svg, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};
use kirie_annotate::{StyleId, Tool};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{
    divider, floating, hint, icon_button, primary_button, secondary_button, small_icon_button,
    small_pick_list,
};
use kirie_ui::{Palette, Scheme, space};

use super::annotate;
use super::canvas::{Backdrop, Board};
use super::viewport::{self, Zoom};
use super::{Editor, FinishEvent, Message, OutsideEvent, window};
use super::{crop, files};
use crate::annotate::annotator::Change;
use crate::annotate::controls;
use crate::annotate::paint::Part;
use crate::annotate::picker::{self, Picker};
use kirie_annotate::marks::Restack;

pub const TEXT_INPUT: &str = "kirie-editor-text";
pub const APP_ICON: &[u8] =
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
        let dialog = self
            .closing_dialog()
            .or_else(|| self.about_dialog())
            .or_else(|| self.settings_dialog());
        let mut layers = stack![window];
        if let Some(dialog) = dialog {
            layers = layers.push(dialog);
        }
        if self.drops.hovered {
            layers = layers.push(super::drop_zone::view(palette));
        }
        layers.into()
    }

    fn header(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let has_image = self.image.is_some();
        let brand = self.brand();
        let history = row![
            icon_button(
                Icon::Undo,
                "Undo (Ctrl+Z)",
                palette,
                false,
                self.annotator
                    .history
                    .can_undo()
                    .then_some(Message::Annotate(Change::Undo))
            ),
            icon_button(
                Icon::Redo,
                "Redo (Ctrl+Shift+Z)",
                palette,
                false,
                self.annotator
                    .history
                    .can_redo()
                    .then_some(Message::Annotate(Change::Redo))
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
            Some(Message::Window(window::Event::Capture)),
        );
        let open = icon_button(
            Icon::Open,
            "Open an image (Ctrl+O)",
            palette,
            false,
            Some(Message::File(files::Event::Open)),
        );
        let mut actions = row![open, capture, divider(palette), history]
            .spacing(space::S)
            .align_y(Alignment::Center);
        if has_image {
            actions = actions.push(divider(palette)).push(self.image_actions());
        }
        let settings = icon_button(
            Icon::Settings,
            "Settings (Ctrl+,)",
            palette,
            false,
            Some(Message::Settings(super::settings::Event::Open)),
        );
        actions = actions.push(divider(palette)).push(settings).push(theme);
        let document = self.document();
        let styles = controls::styles(document.style, palette, annotate(Change::PickStyle));
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

    /// Discarding, saving and copying the image.
    fn image_actions(&self) -> Element<'_, Message> {
        let palette = self.palette;
        row![]
            .push(icon_button(
                Icon::Delete,
                "Discard the image (Ctrl+W)",
                palette,
                false,
                Some(Message::Finish(FinishEvent::Discard)),
            ))
            .push(icon_button(
                Icon::Project,
                "Save editable project to Pictures, with the original under the marks \
                     (Ctrl+Alt+S)",
                palette,
                false,
                Some(Message::File(files::Event::SaveProject)),
            ))
            .push(icon_button(
                Icon::SaveAs,
                "Save as… (Ctrl+Shift+S)",
                palette,
                false,
                Some(Message::File(files::Event::SaveAs)),
            ))
            .push(icon_button(
                Icon::Save,
                "Save to Pictures and copy (Ctrl+S or Shift+Enter)",
                palette,
                false,
                Some(Message::File(files::Event::Save)),
            ))
            .push(primary_button(
                Icon::Copy,
                "Copy",
                palette,
                Message::File(files::Event::Copy),
            ))
            .spacing(space::S)
            .align_y(Alignment::Center)
            .into()
    }

    /// Kirie's icon and name, which open the about card.
    fn brand(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let brand = button(
            row![
                svg(svg::Handle::from_memory(APP_ICON)).width(26).height(26),
                text("Kirie").size(16),
            ]
            .spacing(space::S)
            .align_y(Alignment::Center),
        )
        .padding([space::XS, space::S])
        .on_press(Message::Outside(OutsideEvent::About(true)))
        .style(move |_, status| button::Style {
            background: match status {
                button::Status::Hovered => Some(Background::Color(palette.surface[1])),
                button::Status::Pressed => Some(Background::Color(palette.surface[2])),
                _ => None,
            },
            text_color: palette.text,
            border: Border {
                radius: kirie_ui::radius::CONTROL.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
        hint(brand.into(), "About Kirie", palette)
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
            "Crop (Shift+C)",
            palette,
            self.cropping.is_some(),
            self.image
                .is_some()
                .then_some(Message::Crop(crop::Event::Start)),
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
            let board = |part| {
                iced::widget::canvas(Board { editor: self, part })
                    .width(Length::Fill)
                    .height(Length::Fill)
            };
            Part::all(self.document()).fold(
                stack![
                    iced::widget::canvas(Backdrop { editor: self })
                        .width(Length::Fill)
                        .height(Length::Fill)
                ],
                |layers, part| layers.push(board(part)),
            )
        } else {
            stack![self.empty()]
        };
        let mut floating_bits = column![].spacing(space::S).align_x(Alignment::Center);
        if self.cropping.is_some() {
            floating_bits = floating_bits.push(self.crop_bar());
        }
        if let Some(typing) = &self.annotator.typing {
            floating_bits = floating_bits.push(
                container(
                    row![
                        crate::annotate::typing::text_box(
                            typing,
                            TEXT_INPUT,
                            &self.annotator.emojis,
                            palette,
                            annotate(Change::TextEdit),
                            Message::Annotate(Change::TextDone),
                            Some(Message::PasteText),
                            annotate(Change::Choose),
                        ),
                        column![
                            text("Enter to place").size(12).color(palette.muted),
                            text(": or the smile for emoji")
                                .size(12)
                                .color(palette.muted),
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
            let mut content = row![text(toast.text.clone()).size(13).color(color)]
                .spacing(space::M)
                .align_y(Alignment::Center);
            let mut padding = Padding::from([space::S, space::L]);
            if let Some(file) = &toast.file {
                content = content.push(secondary_button(
                    "Show in Files",
                    palette,
                    Message::Outside(OutsideEvent::ShowInFiles(file.clone())),
                ));
                padding = Padding::from(space::S).left(space::L);
            }
            floating_bits =
                floating_bits.push(container(content).padding(padding).style(floating(palette)));
        }
        layers = layers.push(
            container(floating_bits)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::End)
                .padding(space::XL),
        );
        if let Some(list) = self.mark_list() {
            layers = layers.push(
                container(list)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::Start)
                    .padding(space::L),
            );
        }
        if let Some(card) = self.release_card() {
            layers = layers.push(card);
        }
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
                secondary_button("Whole image", palette, Message::Crop(crop::Event::Reset)),
                secondary_button("Cancel", palette, Message::Crop(crop::Event::Cancel)),
                primary_button(
                    Icon::Crop,
                    "Crop",
                    palette,
                    Message::Crop(crop::Event::Done)
                ),
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
                    Message::Window(window::Event::Capture)
                ),
                icon_button(
                    Icon::Open,
                    "Open an image (Ctrl+O)",
                    palette,
                    false,
                    Some(Message::File(files::Event::Open))
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
                Some(Message::Viewport(viewport::Event::Fit))
            ),
            icon_button(
                Icon::Image,
                "Actual size (Ctrl+1)",
                palette,
                false,
                Some(Message::Viewport(viewport::Event::Actual))
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
        if !self.annotator.picked.is_empty() {
            content = content.push(rule::horizontal(1)).push(section(
                palette,
                "Selection",
                self.mark_actions(),
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

    /// What the picked marks can do: come to the front, go to the back, go away.
    fn mark_actions(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let count = self.annotator.picked.len();
        let several = count > 1;
        let action = |icon, tip, message| icon_button(icon, tip, palette, false, Some(message));
        row![
            action(
                Icon::BringToFront,
                "Bring to front (Ctrl+Shift+]; one step: Ctrl+])",
                Message::Annotate(Change::Restack(Restack::Front)),
            ),
            action(
                Icon::SendToBack,
                "Send to back (Ctrl+Shift+[; one step: Ctrl+[)",
                Message::Annotate(Change::Restack(Restack::Back)),
            ),
            action(
                Icon::Delete,
                if several {
                    "Delete marks (Del)"
                } else {
                    "Delete mark (Del)"
                },
                Message::Annotate(Change::Delete),
            ),
            text(if several {
                format!("{count} marks picked")
            } else {
                "1 mark picked".to_owned()
            })
            .size(12)
            .color(palette.muted),
        ]
        .spacing(space::XS)
        .align_y(Alignment::Center)
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
                Some(Message::Viewport(viewport::Event::ZoomOut))
            ),
            // A fitted image is never enlarged, so "Fit 100%" is the longest label it shows.
            small_pick_list(levels, shown, "Fit 100%", palette, |choice| {
                Message::Viewport(viewport::Event::ZoomTo(choice.zoom()))
            }),
            small_icon_button(
                Icon::ZoomIn,
                "Zoom in (Ctrl++)",
                palette,
                Some(Message::Viewport(viewport::Event::ZoomIn))
            ),
        ]
        .spacing(2)
        .align_y(Alignment::Center)
        .into()
    }

    /// The emoji the sticker tool places, or the picked stickers show.
    fn sticker_section(&self) -> iced::widget::Column<'_, Message> {
        let current = kirie_annotate::marks::sticker_shown(self.document(), &self.annotator.picked)
            .unwrap_or_else(|| self.annotator.emojis.chosen());
        section(
            self.palette,
            "Sticker",
            self.annotator
                .emojis
                .view(current, self.palette, Length::Fill)
                .map(annotate(Change::Emoji)),
        )
    }

    /// The color, and the text look, the spotlight, or the weight, fill and shadow, of the
    /// picked mark or the next.
    fn paint_sections(&self) -> iced::widget::Column<'_, Message> {
        let palette = self.palette;
        let paint = self.annotator.shown_paint();
        let mut colors = column![
            controls::colors(
                &self.annotator.swatches,
                self.document().style,
                paint,
                palette,
                self.annotator.picker.as_ref().map(Picker::target),
                annotate(Change::PickColor),
                annotate(Change::EditColor),
            )
            .wrap()
        ]
        .spacing(space::M);
        if let Some(open) = &self.annotator.picker {
            colors = colors.push(picker::view(open, palette).map(annotate(Change::Picker)));
        }
        let content = column![section(palette, "Color", colors.into())].spacing(space::L);
        if self.annotator.stickering(Some(self.tool)) {
            return self.sticker_section();
        }
        if self.annotator.texting(Some(self.tool)) {
            let fallback = self.document().style.font();
            return content.push(section(
                palette,
                "Text",
                column![
                    controls::fonts(paint.text, fallback, palette, annotate(Change::Look)),
                    controls::text_size(paint.text, palette, 150.0, annotate(Change::Look)),
                    controls::text_layout(paint.text, palette, annotate(Change::Look)),
                ]
                .spacing(space::S)
                .into(),
            ));
        }
        if self.annotator.spotlighting(Some(self.tool)) {
            return content
                .push(section(
                    palette,
                    "Spotlight",
                    controls::spot_shapes(paint, palette, annotate(Change::Spot)),
                ))
                .push(section(
                    palette,
                    "Brightness around spotlights",
                    controls::spotlight_brightness(
                        self.document().spotlight_brightness,
                        palette,
                        126.0,
                        annotate(Change::Brightness),
                    ),
                ));
        }
        let fill = row![
            icon_button(
                Icon::Fill,
                "Fill shapes",
                palette,
                paint.filled,
                Some(Message::Annotate(Change::ToggleFill))
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
                Message::Annotate(Change::ToggleShadow)
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
                let (presets, slider) =
                    controls::weights(paint, palette, 150.0, annotate(Change::Weight));
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
