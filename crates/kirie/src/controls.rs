//! Annotation controls the overlay toolbar and the editor share: tool icons, style, colour and
//! weight pickers.

use iced::Element;
use iced::widget::{Row, mouse_area, row};
use kirie_annotate::{HIGHLIGHT, Paint, Rgba, StyleId, Tool, Weight};
use kirie_ui::Palette;
use kirie_ui::icon::Icon;
use kirie_ui::widget::{pill, swatch, swatch_button};

use crate::paint::color_of;
use crate::picker::Target;
use crate::swatches::{Slot, Swatches};

pub fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Select => Icon::Select,
        Tool::Arrow => Icon::Arrow,
        Tool::Line => Icon::Line,
        Tool::Rectangle => Icon::Rectangle,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Pen => Icon::Pen,
        Tool::Highlighter => Icon::Highlighter,
        Tool::Text => Icon::Text,
        Tool::Counter => Icon::Counter,
        Tool::Blur => Icon::Blur,
        Tool::Pixelate => Icon::Pixelate,
        Tool::Redact => Icon::Redact,
        Tool::Spotlight => Icon::Spotlight,
    }
}

/// The colour to draw with after switching tools: the highlighter takes the highlight colour
/// while the colour is still the default, and gives it back when put down.
pub fn tool_color(tool: Option<Tool>, color: Rgba, default: Rgba) -> Rgba {
    match tool {
        Some(Tool::Highlighter) if color == default => HIGHLIGHT,
        Some(Tool::Highlighter) => color,
        _ if color == HIGHLIGHT => default,
        _ => color,
    }
}

pub fn styles<'a, Message: Clone + 'a>(
    current: StyleId,
    palette: Palette,
    on_pick: impl Fn(StyleId) -> Message,
) -> Element<'a, Message> {
    row(StyleId::ALL
        .into_iter()
        .map(|id| pill(id.name(), id == current, palette, on_pick(id))))
    .spacing(2)
    .into()
}

/// The swatches to draw with, then a button to add a colour. A right click on a swatch
/// changes it; `editing` is what the picker is open for, if it is.
pub fn colors<'a, Message: Clone + 'a>(
    swatches: &Swatches,
    paint: Paint,
    palette: Palette,
    editing: Option<Target>,
    on_pick: impl Fn(Rgba) -> Message,
    on_edit: impl Fn(Target) -> Message,
) -> Row<'a, Message> {
    let defaults = swatches
        .defaults()
        .iter()
        .enumerate()
        .map(|(index, &color)| (Slot::Default(index), color));
    let custom = swatches
        .custom()
        .iter()
        .enumerate()
        .map(|(index, &color)| (Slot::Custom(index), color));
    let swatches = defaults.chain(custom).map(|(slot, color)| {
        let selected = editing.map_or(paint.color == color, |editing| {
            editing == Target::Change(slot)
        });
        mouse_area(swatch(color_of(color), selected, palette, on_pick(color)))
            .on_right_press(on_edit(Target::Change(slot)))
            .into()
    });
    row(swatches)
        .push(swatch_button(
            Icon::Plus,
            "Add a colour (right-click a colour to change it)",
            palette,
            editing == Some(Target::New),
            on_edit(Target::New),
        ))
        .spacing(2)
        .align_y(iced::Alignment::Center)
}

pub fn weights<'a, Message: Clone + 'a>(
    paint: Paint,
    palette: Palette,
    on_pick: impl Fn(Weight) -> Message,
) -> Element<'a, Message> {
    row([
        ("S", Weight::Thin),
        ("M", Weight::Medium),
        ("L", Weight::Thick),
    ]
    .into_iter()
    .map(|(label, weight)| pill(label, paint.weight == weight, palette, on_pick(weight))))
    .spacing(2)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kirie_annotate::PALETTE;

    #[test]
    fn the_highlighter_brings_its_own_colour() {
        let default = PALETTE[0];
        assert_eq!(
            tool_color(Some(Tool::Highlighter), default, default),
            HIGHLIGHT
        );
        let blue = PALETTE[3];
        assert_eq!(tool_color(Some(Tool::Highlighter), blue, default), blue);
        assert_eq!(tool_color(Some(Tool::Arrow), HIGHLIGHT, default), default);
        assert_eq!(tool_color(None, blue, default), blue);
    }

    #[test]
    fn every_tool_has_its_own_icon() {
        let mut icons: Vec<Icon> = Tool::ALL.into_iter().map(tool_icon).collect();
        icons.dedup();
        assert_eq!(icons.len(), Tool::ALL.len());
    }
}
