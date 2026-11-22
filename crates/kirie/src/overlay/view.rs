//! What each screen's surface shows: the frozen image, the marks, and the floating toolbar
//! next to the selection.

use iced::widget::{Space, column, container, image, pin, row, stack, text_input};
use iced::{Element, Length, Point, Rectangle, window};
use kirie_annotate::Tool;
use kirie_ui::icon::Icon;
use kirie_ui::widget::{divider, floating, icon_button, primary_button};
use kirie_ui::{radius, space};

use super::{Action, Message, Overlay, canvas};
use crate::controls;
use crate::picker::{self, Picker};

pub const TEXT_INPUT: &str = "kirie-text";
/// Room the toolbar takes; used to keep it on screen.
const TOOLBAR: iced::Size = iced::Size::new(800.0, 92.0);
const GAP: f32 = 12.0;
/// A swatch and the space after it.
const SWATCH: f32 = 30.0;
/// Custom swatches that fit in [`TOOLBAR`] before it grows.
const ROOM_FOR_CUSTOM: usize = 4;

impl Overlay {
    pub fn view(&self, id: window::Id) -> Element<'_, Message> {
        let Some(&index) = self.surfaces.get(&id) else {
            return Space::new().into();
        };
        let screen = &self.screens[index];
        let mut layers = stack![
            image(screen.handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Fill)
                .filter_method(image::FilterMethod::Linear),
            iced::widget::canvas(canvas::Surface {
                overlay: self,
                area: screen.area,
                scale: screen.scale,
            })
            .width(Length::Fill)
            .height(Length::Fill),
        ];
        if let Some(typing) = &self.typing {
            #[allow(clippy::cast_possible_truncation)]
            let desktop = Point::new(typing.at.x as f32, typing.at.y as f32);
            if screen.area.contains(desktop) {
                let at = Point::new(desktop.x - screen.area.x, desktop.y - screen.area.y);
                layers = layers.push(pin(self.text_box()).position(at));
            }
        }
        if let Some(at) = self.toolbar_position(screen.area) {
            layers = layers.push(pin(self.toolbar()).position(at));
        }
        layers.into()
    }

    /// Where the toolbar goes on this screen, if on this one: under the selection, else above
    /// it, else inside its bottom edge; always kept on screen.
    fn toolbar_position(&self, screen: Rectangle) -> Option<Point> {
        let selection = self.selection.rect()?;
        if self.selection.dragging() {
            return None;
        }
        let toolbar = self.toolbar_size();
        let below = selection.y + selection.height + GAP;
        let above = selection.y - GAP - toolbar.height;
        let anchor = Point::new(selection.center_x(), selection.y + selection.height);
        let areas = || self.screens.iter().map(|screen| screen.area);
        let home = areas()
            .find(|area| area.contains(anchor))
            .or_else(|| areas().find(|area| area.intersects(&selection)))?;
        if home != screen {
            return None;
        }
        let y = if below + toolbar.height <= home.y + home.height {
            below
        } else if above >= home.y {
            above
        } else {
            selection.y + selection.height - toolbar.height - GAP
        };
        let x = (selection.center_x() - toolbar.width / 2.0).clamp(
            home.x + GAP,
            (home.x + home.width - toolbar.width - GAP).max(home.x + GAP),
        );
        Some(Point::new(x - home.x, y.max(home.y) - home.y))
    }

    fn toolbar_size(&self) -> iced::Size {
        #[allow(clippy::cast_precision_loss)]
        let extra = self.swatches.custom().len().saturating_sub(ROOM_FOR_CUSTOM) as f32 * SWATCH;
        let picker = if self.picker.is_some() {
            picker::HEIGHT
        } else {
            0.0
        };
        iced::Size::new(TOOLBAR.width + extra, TOOLBAR.height + picker)
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let style = self.style();
        let tools = row(Tool::ALL.into_iter().map(|tool| {
            let active = self.tool == Some(tool);
            icon_button(
                controls::tool_icon(tool),
                tool.label(),
                palette,
                active,
                Some(Message::PickTool((!active).then_some(tool))),
            )
        }))
        .spacing(2);
        let history = row![
            icon_button(
                Icon::Undo,
                "Undo",
                palette,
                false,
                self.history.can_undo().then_some(Message::Undo)
            ),
            icon_button(
                Icon::Redo,
                "Redo",
                palette,
                false,
                self.history.can_redo().then_some(Message::Redo)
            ),
        ]
        .spacing(2);
        let actions = row![
            icon_button(
                Icon::Edit,
                "Open in the editor (Ctrl+E)",
                palette,
                false,
                Some(Message::Finish(Action::Edit))
            ),
            icon_button(
                Icon::Save,
                "Save to Pictures",
                palette,
                false,
                Some(Message::Finish(Action::Save))
            ),
            primary_button(Icon::Copy, "Copy", palette, Message::Finish(Action::Copy)),
            icon_button(Icon::Close, "Cancel", palette, false, Some(Message::Escape)),
        ]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center);
        let main = row![tools, divider(palette), history, divider(palette), actions]
            .spacing(space::S)
            .align_y(iced::Alignment::Center);

        let styles = controls::styles(style, palette, Message::PickStyle);
        let colors = controls::colors(
            &self.swatches,
            self.paint,
            palette,
            self.picker.as_ref().map(Picker::target),
            Message::PickColor,
            Message::EditColor,
        );
        let weights = controls::weights(self.paint, palette, Message::PickWeight);
        let properties = row![
            styles,
            divider(palette),
            colors,
            divider(palette),
            weights,
            divider(palette),
            icon_button(
                Icon::Fill,
                "Fill shapes",
                palette,
                self.paint.filled,
                Some(Message::ToggleFill)
            ),
        ]
        .spacing(space::S)
        .align_y(iced::Alignment::Center);

        let mut panel = column![].spacing(space::XS);
        if let Some(open) = &self.picker {
            panel = panel.push(
                container(picker::view(open, palette).map(Message::Picker))
                    .padding([space::XS, 0.0]),
            );
        }
        container(panel.push(properties).push(main))
            .padding(space::S)
            .style(floating(palette))
            .into()
    }

    fn text_box(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let value = self
            .typing
            .as_ref()
            .map_or("", |typing| typing.text.as_str());
        text_input("Type, then Enter", value)
            .id(TEXT_INPUT)
            .on_input(Message::TextChanged)
            .on_submit(Message::TextDone)
            .width(280)
            .size(16)
            .padding([space::XS + 2.0, space::S])
            .style(move |_, _| text_input::Style {
                background: iced::Background::Color(palette.surface[0].scale_alpha(0.92)),
                border: iced::Border {
                    color: palette.accent,
                    width: 1.5,
                    radius: radius::CONTROL.into(),
                },
                icon: palette.muted,
                placeholder: palette.muted,
                value: palette.text,
                selection: palette.accent.scale_alpha(0.4),
            })
            .into()
    }
}
