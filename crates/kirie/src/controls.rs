//! Annotation controls the overlay toolbar and the editor share: tool icons, style, color and
//! weight pickers.

use iced::Element;
use iced::widget::{Row, container, mouse_area, row, text};
use kirie_annotate::{Align, Font, HIGHLIGHT, Paint, Rgba, StyleId, TextLook, Tool, Weight};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{icon_button, pill, swatch, swatch_button, value_slider};
use kirie_ui::{Palette, space};

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

/// The color to draw with after switching tools: the highlighter takes the highlight color
/// while the color is still the default, and gives it back when put down.
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

/// The swatches to draw with, then a button to add a color. A right click on a swatch
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
            "Add a color (right-click a color to change it)",
            palette,
            editing == Some(Target::New),
            on_edit(Target::New),
        ))
        .spacing(2)
        .align_y(iced::Alignment::Center)
}

/// A change to the weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WeightEvent {
    Pick(Weight),
    Slide(f64),
    /// The slider was let go.
    Done,
}

/// The three usual weights, and a slider for any other; `width` is the slider's.
pub fn weights<'a, Message: Clone + 'a>(
    paint: Paint,
    palette: Palette,
    width: f32,
    on: impl Fn(WeightEvent) -> Message + 'a,
) -> (Element<'a, Message>, Element<'a, Message>) {
    let presets = row([
        ("S", Weight::THIN),
        ("M", Weight::MEDIUM),
        ("L", Weight::THICK),
    ]
    .into_iter()
    .map(|(label, weight)| {
        pill(
            label,
            paint.weight == weight,
            palette,
            on(WeightEvent::Pick(weight)),
        )
    }))
    .spacing(2)
    .into();
    let done = on(WeightEvent::Done);
    let slider = row![
        container(value_slider(
            Weight::MIN..=Weight::MAX,
            paint.weight.base(),
            palette,
            move |width| on(WeightEvent::Slide(width)),
            done,
        ))
        .width(width),
        text(format!("{:.1} px", paint.weight.base()))
            .size(12)
            .color(palette.muted)
            .width(42),
    ]
    .spacing(space::S)
    .align_y(iced::Alignment::Center)
    .into();
    (presets, slider)
}

/// A change to how text looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextEvent {
    Font(Font),
    Size(f64),
    /// The size slider was let go.
    SizeDone,
    Align(Align),
    ToggleOutline,
}

impl TextEvent {
    /// `look` changed by this event.
    pub fn apply(self, look: TextLook) -> TextLook {
        match self {
            TextEvent::Font(font) => TextLook {
                font: Some(font),
                ..look
            },
            TextEvent::Size(size) => TextLook {
                size: size.clamp(TextLook::MIN_SIZE, TextLook::MAX_SIZE),
                ..look
            },
            TextEvent::Align(align) => TextLook { align, ..look },
            TextEvent::ToggleOutline => TextLook {
                outlined: !look.outlined,
                ..look
            },
            TextEvent::SizeDone => look,
        }
    }

    /// Whether the event is one step of a drag, to be undone with the rest of it.
    pub fn gradual(self) -> bool {
        matches!(self, TextEvent::Size(_))
    }
}

/// The faces to set text in; `fallback` is the style's face, shown while none is picked.
pub fn fonts<'a, Message: Clone + 'a>(
    look: TextLook,
    fallback: Font,
    palette: Palette,
    on: impl Fn(TextEvent) -> Message,
) -> Element<'a, Message> {
    let current = look.font.unwrap_or(fallback);
    row(Font::ALL.into_iter().map(|font| {
        pill(
            font.name(),
            font == current,
            palette,
            on(TextEvent::Font(font)),
        )
    }))
    .spacing(2)
    .into()
}

/// A slider for the text size, with the size beside it.
pub fn text_size<'a, Message: Clone + 'a>(
    look: TextLook,
    palette: Palette,
    width: f32,
    on: impl Fn(TextEvent) -> Message + 'a,
) -> Element<'a, Message> {
    let done = on(TextEvent::SizeDone);
    row![
        container(value_slider(
            TextLook::MIN_SIZE..=TextLook::MAX_SIZE,
            look.size,
            palette,
            move |size| on(TextEvent::Size(size)),
            done,
        ))
        .width(width),
        text(format!("{:.0} px", look.size))
            .size(12)
            .color(palette.muted)
            .width(42),
    ]
    .spacing(space::S)
    .align_y(iced::Alignment::Center)
    .into()
}

/// How the lines line up, and whether the text is outlined.
pub fn text_layout<'a, Message: Clone + 'a>(
    look: TextLook,
    palette: Palette,
    on: impl Fn(TextEvent) -> Message,
) -> Element<'a, Message> {
    let aligns = [
        (Align::Left, Icon::AlignLeft, "Align left"),
        (Align::Center, Icon::AlignCenter, "Center"),
        (Align::Right, Icon::AlignRight, "Align right"),
    ]
    .into_iter()
    .map(|(align, icon, label)| {
        icon_button(
            icon,
            label,
            palette,
            look.align == align,
            Some(on(TextEvent::Align(align))),
        )
    });
    row(aligns)
        .push(icon_button(
            Icon::TextOutline,
            "Outline the text",
            palette,
            look.outlined,
            Some(on(TextEvent::ToggleOutline)),
        ))
        .spacing(2)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kirie_annotate::PALETTE;

    #[test]
    fn the_highlighter_brings_its_own_color() {
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
    fn text_events_change_only_their_part() {
        let look = TextLook::default();
        let mono = TextEvent::Font(Font::Mono).apply(look);
        assert_eq!(mono.font, Some(Font::Mono));
        assert!((mono.size - look.size).abs() < f64::EPSILON);
        let huge = TextEvent::Size(1000.0).apply(look);
        assert!((huge.size - TextLook::MAX_SIZE).abs() < f64::EPSILON);
        assert_eq!(
            TextEvent::Align(Align::Right).apply(look).align,
            Align::Right
        );
        assert!(!TextEvent::ToggleOutline.apply(look).outlined);
        assert_eq!(TextEvent::SizeDone.apply(look), look);
        assert!(TextEvent::Size(9.0).gradual() && !TextEvent::SizeDone.gradual());
    }

    #[test]
    fn every_tool_has_its_own_icon() {
        let mut icons: Vec<Icon> = Tool::ALL.into_iter().map(tool_icon).collect();
        icons.dedup();
        assert_eq!(icons.len(), Tool::ALL.len());
    }
}
