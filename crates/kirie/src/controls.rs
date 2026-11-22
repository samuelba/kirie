//! Annotation controls the overlay toolbar and the editor share: tool icons, style, colour and
//! weight pickers.

use iced::Element;
use iced::widget::row;
use kirie_annotate::{Paint, Rgba, StyleId, Tool, Weight};
use kirie_ui::Palette;
use kirie_ui::icon::Icon;
use kirie_ui::widget::{pill, swatch};

use crate::paint::color_of;

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

pub fn colors<'a, Message: Clone + 'a>(
    style: StyleId,
    paint: Paint,
    palette: Palette,
    on_pick: impl Fn(Rgba) -> Message,
) -> Element<'a, Message> {
    row(style.palette().into_iter().map(|color| {
        swatch(
            color_of(color),
            paint.color == color,
            palette,
            on_pick(color),
        )
    }))
    .spacing(2)
    .into()
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

    #[test]
    fn every_tool_has_its_own_icon() {
        let mut icons: Vec<Icon> = Tool::ALL.into_iter().map(tool_icon).collect();
        icons.dedup();
        assert_eq!(icons.len(), Tool::ALL.len());
    }
}
