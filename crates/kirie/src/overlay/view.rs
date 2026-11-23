//! What each screen's surface shows: the frozen image, the marks, and the floating toolbar
//! next to the selection, or wherever it was dragged by its grip.

use iced::widget::{Space, column, container, image, mouse_area, pin, row, stack, svg, text};
use iced::{Element, Length, Point, Rectangle, Size, mouse, window};
use kirie_annotate::Tool;
use kirie_ui::icon::Icon;
use kirie_ui::space;
use kirie_ui::widget::{divider, floating, icon_button, primary_button, secondary_button};

use super::{Action, Message, Overlay, canvas};
use crate::controls;
use crate::picker::{self, Picker};

pub const TEXT_INPUT: &str = "kirie-text";
/// Room the toolbar takes; used to keep it on screen.
const TOOLBAR: iced::Size = iced::Size::new(1072.0, 92.0);
/// How much wider "Back to the editor" is than "Copy" on the primary button.
const BACK_TO_EDITOR: f32 = 96.0;
const GAP: f32 = 12.0;
/// A swatch and the space after it.
const SWATCH: f32 = 30.0;
/// How much wider the text controls are than the weight and fill controls they replace.
const TEXT_CONTROLS: f32 = 130.0;
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
            // Under the words, so the box never hides what is typed.
            let words = self
                .document()
                .bounds(&typing.preview(self.document(), self.paint));
            #[allow(clippy::cast_possible_truncation)]
            let desktop = Point::new(typing.at.x as f32, (words.y1 + 8.0) as f32);
            if screen.area.contains(desktop) {
                let at = Point::new(desktop.x - screen.area.x, desktop.y - screen.area.y);
                layers = layers.push(
                    pin(crate::typing::text_box(
                        typing,
                        TEXT_INPUT,
                        self.palette,
                        Message::TextEdit,
                        Message::TextDone,
                        None,
                    ))
                    .position(at),
                );
            }
        }
        if self.selection.rect().is_none() {
            layers = layers.push(self.hint(index));
        }
        if let Some(at) = self.toolbar_position(screen.area) {
            layers = layers.push(pin(self.toolbar()).position(at));
        }
        layers.into()
    }

    /// What a click and a drag do, at the foot of each screen until something is selected.
    /// Where windows cover a whole screen, a click takes a window, so a button takes the screen.
    fn hint(&self, screen: usize) -> Element<'_, Message> {
        let words = if self.windows.is_empty() {
            "Drag to cut a region  ·  Click to take the screen  ·  Esc to cancel"
        } else {
            "Drag to cut a region  ·  Click to take a window  ·  Esc to cancel"
        };
        let mut content = row![text(words).size(13)]
            .spacing(space::M)
            .align_y(iced::Alignment::Center);
        if !self.windows.is_empty() {
            content = content.push(secondary_button(
                "Take this screen",
                self.palette,
                Message::SelectScreen(screen),
            ));
        }
        container(
            container(content)
                .padding([space::S, space::M])
                .style(floating(self.palette)),
        )
        .center_x(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(iced::Padding::ZERO.bottom(space::XL * 2.0))
        .into()
    }

    /// Where the toolbar goes on this screen, if on this one.
    fn toolbar_position(&self, screen: Rectangle) -> Option<Point> {
        let (at, home) = self.toolbar_place()?;
        (home == screen).then(|| Point::new(at.x - home.x, at.y - home.y))
    }

    /// Where the toolbar goes, in desktop coordinates, and the screen it is on.
    pub(super) fn toolbar_place(&self) -> Option<(Point, Rectangle)> {
        let selection = self.selection.rect()?;
        if self.selection.dragging() {
            return None;
        }
        let screens: Vec<Rectangle> = self.screens.iter().map(|screen| screen.area).collect();
        place_toolbar(selection, &screens, self.toolbar_size(), self.toolbar_at)
    }

    fn toolbar_size(&self) -> iced::Size {
        #[allow(clippy::cast_precision_loss)]
        let extra = self.swatches.custom().len().saturating_sub(ROOM_FOR_CUSTOM) as f32 * SWATCH;
        let picker = if self.picker.is_some() {
            picker::HEIGHT
        } else {
            0.0
        };
        let text = if self.texting() { TEXT_CONTROLS } else { 0.0 };
        let label = if self.primary == Action::Edit {
            BACK_TO_EDITOR
        } else {
            0.0
        };
        iced::Size::new(
            TOOLBAR.width + extra + text + label,
            TOOLBAR.height + picker,
        )
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let palette = self.palette;
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
        let remember = icon_button(
            Icon::Pin,
            if self.remembered.is_some() {
                "Remembered for the next screenshot until it changes; click to forget it"
            } else {
                "Remember this region for the next screenshot"
            },
            palette,
            self.remembered.is_some(),
            Some(Message::ToggleRemember),
        );
        let main = row![
            tools,
            divider(palette),
            history,
            divider(palette),
            remember,
            divider(palette),
            self.actions()
        ]
        .spacing(space::S)
        .align_y(iced::Alignment::Center);

        let properties = self.properties();

        let mut panel = column![].spacing(space::XS);
        if let Some(open) = &self.picker {
            panel = panel.push(
                container(picker::view(open, palette).map(Message::Picker))
                    .padding([space::XS, 0.0]),
            );
        }
        let grip = mouse_area(
            container(
                svg(Icon::Grip.handle())
                    .width(16)
                    .height(16)
                    .style(move |_, _| svg::Style {
                        color: Some(palette.muted),
                    }),
            )
            .center_y(Length::Fill),
        )
        .on_move(Message::GripHover)
        .on_press(Message::ToolbarGrab)
        .interaction(if self.toolbar_drag.is_some() {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Grab
        });
        let content = row![grip, panel.push(properties).push(main)]
            .spacing(space::XS)
            .height(Length::Shrink);
        container(content)
            .padding(space::S)
            .style(floating(palette))
            .into()
    }

    /// Edit, save, copy and cancel; the action Enter takes is the primary button.
    fn actions(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let save = row![
            icon_button(
                Icon::SaveAs,
                "Save as… (Ctrl+Shift+S)",
                palette,
                false,
                Some(Message::Finish(Action::SaveAs)),
            ),
            icon_button(
                Icon::Save,
                "Save to Pictures (Ctrl+S)",
                palette,
                false,
                Some(Message::Finish(Action::Save)),
            ),
        ]
        .spacing(space::XS);
        let edit = Message::Finish(Action::Edit);
        let copy = Message::Finish(Action::Copy);
        let actions = match self.primary {
            Action::Edit => row![
                save,
                icon_button(Icon::Copy, "Copy (Ctrl+C)", palette, false, Some(copy)),
                primary_button(Icon::Edit, "Back to the editor", palette, edit),
            ],
            Action::Copy | Action::Save | Action::SaveAs => row![
                icon_button(
                    Icon::Edit,
                    "Open in the editor (Ctrl+E)",
                    palette,
                    false,
                    Some(edit)
                ),
                save,
                primary_button(Icon::Copy, "Copy", palette, copy),
            ],
        };
        actions
            .push(enter_key(palette))
            .push(icon_button(
                Icon::Close,
                "Cancel (Esc)",
                palette,
                false,
                Some(Message::Escape),
            ))
            .spacing(space::XS)
            .align_y(iced::Alignment::Center)
            .into()
    }

    /// Style, color, and the text look, the spotlight, or the weight, fill and shadow.
    fn properties(&self) -> Element<'_, Message> {
        let palette = self.palette;
        let style = self.style();
        let paint = self.shown_paint();
        let styles = controls::styles(style, palette, Message::PickStyle);
        let colors = controls::colors(
            &self.swatches,
            style,
            paint,
            palette,
            self.picker.as_ref().map(Picker::target),
            Message::PickColor,
            Message::EditColor,
        );
        let mut properties = row![styles, divider(palette), colors, divider(palette)]
            .spacing(space::S)
            .align_y(iced::Alignment::Center);
        if self.texting() {
            properties = properties
                .push(controls::fonts(
                    paint.text,
                    style.font(),
                    palette,
                    Message::Look,
                ))
                .push(divider(palette))
                .push(controls::text_size(
                    paint.text,
                    palette,
                    96.0,
                    Message::Look,
                ))
                .push(controls::text_layout(paint.text, palette, Message::Look));
        } else if self.spotlighting() {
            properties = properties
                .push(controls::spot_shapes(paint, palette, Message::Spot))
                .push(divider(palette))
                .push(controls::spotlight_brightness(
                    self.document().spotlight_brightness,
                    palette,
                    96.0,
                    Message::Brightness,
                ));
        } else {
            properties = properties
                .push({
                    let (presets, slider) =
                        controls::weights(paint, palette, 72.0, Message::Weight);
                    row![presets, slider]
                        .spacing(space::S)
                        .align_y(iced::Alignment::Center)
                })
                .push(divider(palette))
                .push(
                    row![
                        icon_button(
                            Icon::Fill,
                            "Fill shapes",
                            palette,
                            paint.filled,
                            Some(Message::ToggleFill),
                        ),
                        controls::shadow(
                            style,
                            self.document().shadow,
                            palette,
                            Message::ToggleShadow,
                        ),
                    ]
                    .spacing(2),
                );
        }
        properties.into()
    }
}

/// A key cap that says Enter does what the button before it does.
fn enter_key<'a>(palette: kirie_ui::Palette) -> Element<'a, Message> {
    container(text("Enter").size(11).color(palette.muted))
        .padding([1.0, space::XS + 1.0])
        .style(move |_| container::Style {
            border: iced::Border {
                color: palette.border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Where a toolbar of `size` goes, in desktop coordinates, and the screen it is on: where it
/// was `moved` to, else under the selection, else above it, else inside its bottom edge;
/// always kept on its screen.
fn place_toolbar(
    selection: Rectangle,
    screens: &[Rectangle],
    size: Size,
    moved: Option<Point>,
) -> Option<(Point, Rectangle)> {
    let on_screen = |at: Point, home: Rectangle| {
        let x = at.x.clamp(
            home.x + GAP,
            (home.x + home.width - size.width - GAP).max(home.x + GAP),
        );
        let y = at.y.clamp(
            home.y + GAP,
            (home.y + home.height - size.height - GAP).max(home.y + GAP),
        );
        (Point::new(x, y), home)
    };
    if let Some(at) = moved {
        let middle = Point::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
        let home = screens
            .iter()
            .find(|area| area.contains(middle))
            .or_else(|| screens.iter().find(|area| area.contains(at)))
            .or_else(|| screens.first())?;
        return Some(on_screen(at, *home));
    }
    let below = selection.y + selection.height + GAP;
    let above = selection.y - GAP - size.height;
    let anchor = Point::new(selection.center_x(), selection.y + selection.height);
    let home = *screens
        .iter()
        .find(|area| area.contains(anchor))
        .or_else(|| screens.iter().find(|area| area.intersects(&selection)))?;
    let y = if below + size.height <= home.y + home.height {
        below
    } else if above >= home.y {
        above
    } else {
        selection.y + selection.height - size.height - GAP
    };
    let x = (selection.center_x() - size.width / 2.0).clamp(
        home.x + GAP,
        (home.x + home.width - size.width - GAP).max(home.x + GAP),
    );
    Some((Point::new(x, y.max(home.y)), home))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size::new(400.0, 90.0);

    fn screens() -> [Rectangle; 2] {
        [
            Rectangle::new(Point::ORIGIN, Size::new(1920.0, 1080.0)),
            Rectangle::new(Point::new(1920.0, 0.0), Size::new(1920.0, 1080.0)),
        ]
    }

    #[test]
    fn the_toolbar_sits_under_the_selection_or_inside_it() {
        let selection = Rectangle::new(Point::new(100.0, 100.0), Size::new(600.0, 300.0));
        let (at, home) = place_toolbar(selection, &screens(), SIZE, None).unwrap();
        assert_eq!(home, screens()[0]);
        assert!((at.y - (400.0 + GAP)).abs() < f32::EPSILON);
        let whole = screens()[0];
        let (inside, _) = place_toolbar(whole, &screens(), SIZE, None).unwrap();
        assert!(inside.y + SIZE.height <= whole.height - GAP + f32::EPSILON);
    }

    #[test]
    fn a_moved_toolbar_stays_put_on_its_screen() {
        let selection = screens()[0];
        let moved = Point::new(2500.0, 30.0);
        let (at, home) = place_toolbar(selection, &screens(), SIZE, Some(moved)).unwrap();
        assert_eq!(home, screens()[1]);
        assert_eq!(at, moved);
        let (kept, home) = place_toolbar(
            selection,
            &screens(),
            SIZE,
            Some(Point::new(3800.0, 1050.0)),
        )
        .unwrap();
        assert_eq!(home, screens()[1]);
        assert!(kept.x + SIZE.width <= 3840.0 - GAP && kept.y + SIZE.height <= 1080.0 - GAP);
        let (lost, home) = place_toolbar(
            selection,
            &screens(),
            SIZE,
            Some(Point::new(-900.0, -900.0)),
        )
        .unwrap();
        assert_eq!(home, screens()[0]);
        assert_eq!(lost, Point::new(GAP, GAP));
    }
}
