//! The drawing options last used, kept for the next screenshot and the next session: the paint,
//! the style, the marks' shadow, the spotlight brightness and the editor's frame.

use kirie_annotate::{Document, Frame, Paint, StyleId, TextLook, Tool};
use serde::{Deserialize, Serialize};

use crate::annotate::swatches::Swatches;

const FILE: &str = "drawing.ron";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Drawing {
    /// In the shades of `style`.
    pub paint: Paint,
    pub style: StyleId,
    pub shadow: bool,
    pub spotlight_brightness: f64,
    /// The margin in logical pixels, so it suits images of any density.
    pub frame: Frame,
}

impl Default for Drawing {
    fn default() -> Self {
        Self {
            paint: Paint::default(),
            style: StyleId::default(),
            shadow: true,
            spotlight_brightness: Document::SPOTLIGHT_BRIGHTNESS,
            frame: Frame::default(),
        }
    }
}

impl Drawing {
    /// The options last saved, or the usual ones.
    pub fn load() -> Self {
        crate::storage::store::state_path(FILE)
            .and_then(|path| crate::storage::store::read::<Self>(&path))
            .map(Self::checked)
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let saved = crate::storage::store::state_path(FILE)
            .ok_or_else(|| "cannot find the state folder".to_owned())
            .and_then(|path| crate::storage::store::save(&path, self));
        if let Err(error) = saved {
            tracing::warn!(%error, "drawing options not saved");
        }
    }

    /// Values a hand-edited file may put out of range, brought back in.
    fn checked(self) -> Self {
        let text = self.paint.text;
        let size = if text.size.is_finite() {
            text.size.clamp(TextLook::MIN_SIZE, TextLook::MAX_SIZE)
        } else {
            TextLook::default().size
        };
        let margin = if self.frame.margin.is_finite() {
            self.frame.margin.clamp(0.0, Frame::MAX_MARGIN)
        } else {
            0.0
        };
        Self {
            paint: Paint {
                text: TextLook { size, ..text },
                ..self.paint
            },
            spotlight_brightness: Document::spotlight_brightness(self.spotlight_brightness),
            frame: Frame {
                margin,
                ..self.frame
            },
            ..self
        }
    }

    /// The paint to draw with `tool`: the highlighter keeps its own color.
    pub fn paint_for(&self, tool: Option<Tool>, swatches: &Swatches) -> Paint {
        Paint {
            color: crate::annotate::controls::tool_color(
                tool,
                self.paint.color,
                swatches.first(self.style),
            ),
            ..self.paint
        }
    }

    /// These options with `paint` and the marks' options of `document`.
    #[must_use]
    pub fn with_marks(self, paint: Paint, document: &Document) -> Self {
        Self {
            paint,
            style: document.style,
            shadow: document.shadow,
            spotlight_brightness: document.spotlight_brightness,
            ..self
        }
    }

    /// These options with the frame of `document`.
    #[must_use]
    pub fn with_frame(self, document: &Document) -> Self {
        Self {
            frame: Frame {
                margin: document.frame.margin / document.unit,
                ..document.frame
            },
            ..self
        }
    }

    /// `document` drawn in these options' style, shadow and spotlight brightness.
    pub fn styled(&self, mut document: Document) -> Document {
        document.style = self.style;
        document.shadow = self.shadow;
        document.spotlight_brightness = self.spotlight_brightness;
        document
    }

    /// `document` in these options' frame.
    pub fn framed(&self, mut document: Document) -> Document {
        document.frame = Frame {
            margin: self.frame.margin * document.unit,
            ..self.frame
        };
        document
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kirie_annotate::{Font, Rgba, Weight};

    fn used() -> Drawing {
        Drawing {
            paint: Paint {
                color: Rgba::rgb(0x14, 0xb8, 0xa6),
                weight: Weight::THICK,
                filled: true,
                text: TextLook {
                    font: Some(Font::Mono),
                    outlined: false,
                    ..TextLook::default()
                },
                ..Paint::default()
            },
            style: StyleId::Sketch,
            shadow: false,
            spotlight_brightness: 0.2,
            frame: Frame {
                margin: 40.0,
                background: Some(Rgba::rgb(0, 0, 0)),
                shadow: true,
                overflow: true,
            },
        }
    }

    #[test]
    fn options_survive_a_round_trip() {
        let text = ron::to_string(&used()).unwrap();
        assert_eq!(ron::from_str::<Drawing>(&text).unwrap(), used());
    }

    #[test]
    fn a_partial_file_keeps_the_rest_usual() {
        let drawing: Drawing = ron::from_str("(style: Ink)").unwrap();
        assert_eq!(drawing.style, StyleId::Ink);
        assert_eq!(drawing.paint, Paint::default());
        assert!(drawing.shadow);
    }

    #[test]
    fn values_out_of_range_are_brought_back() {
        let wild = Drawing {
            paint: Paint {
                text: TextLook {
                    size: 9000.0,
                    ..TextLook::default()
                },
                ..Paint::default()
            },
            spotlight_brightness: f64::NAN,
            frame: Frame {
                margin: -3.0,
                ..Frame::default()
            },
            ..Drawing::default()
        }
        .checked();
        assert!((wild.paint.text.size - TextLook::MAX_SIZE).abs() < f64::EPSILON);
        assert!((wild.spotlight_brightness - Document::SPOTLIGHT_BRIGHTNESS).abs() < f64::EPSILON);
        assert!(wild.frame.margin.abs() < f64::EPSILON);
    }

    #[test]
    fn documents_take_the_options_and_give_them_back() {
        let dense = Document::new(400, 300, 2.0);
        let styled = used().framed(used().styled(dense));
        assert_eq!(styled.style, StyleId::Sketch);
        assert!(!styled.shadow);
        assert!((styled.frame.margin - 80.0).abs() < f64::EPSILON);
        let back = Drawing::default()
            .with_marks(used().paint, &styled)
            .with_frame(&styled);
        assert_eq!(back, used());
    }

    #[test]
    fn the_highlighter_keeps_its_own_color() {
        let swatches = Swatches::default();
        let highlighting = Drawing {
            paint: Paint {
                color: kirie_annotate::HIGHLIGHT,
                ..Paint::default()
            },
            ..Drawing::default()
        };
        assert_eq!(
            highlighting.paint_for(None, &swatches).color,
            swatches.first(StyleId::Clean)
        );
        assert_eq!(
            highlighting
                .paint_for(Some(Tool::Highlighter), &swatches)
                .color,
            kirie_annotate::HIGHLIGHT
        );
    }
}
