//! Line icons from Lucide (ISC), drawn in the current text or accent color.

use iced::widget::svg;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($name,)*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$name,)*];

            const fn bytes(self) -> &'static [u8] {
                match self {
                    $(Icon::$name => include_bytes!(concat!("../../../assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    Select => "mouse-pointer-2",
    Arrow => "move-up-right",
    Line => "minus",
    Rectangle => "square",
    Ellipse => "circle",
    Pen => "pencil",
    Highlighter => "highlighter",
    Text => "type",
    Counter => "hash",
    Blur => "blur",
    Pixelate => "pixelate",
    Redact => "eye-off",
    Spotlight => "focus",
    Undo => "undo-2",
    Redo => "redo-2",
    Copy => "copy",
    Save => "download",
    SaveAs => "save",
    Edit => "square-pen",
    Close => "x",
    Done => "check",
    Fill => "paint-bucket",
    Light => "sun",
    Dark => "moon",
    Open => "folder-open",
    Capture => "camera",
    ZoomIn => "zoom-in",
    ZoomOut => "zoom-out",
    Fit => "scan",
    Project => "file-archive",
    Delete => "trash",
    Image => "image",
    Plus => "plus",
    AlignLeft => "text-align-start",
    AlignCenter => "text-align-center",
    AlignRight => "text-align-end",
    TextOutline => "type-outline",
    Shadow => "shadow",
    Grip => "grip-vertical",
    Crop => "crop",
    Expand => "expand",
    Pin => "pin",
    Pipette => "pipette",
    BringToFront => "bring-to-front",
    SendToBack => "send-to-back",
}

impl Icon {
    pub fn handle(self) -> svg::Handle {
        svg::Handle::from_memory(self.bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_an_svg() {
        for icon in Icon::ALL {
            let text = std::str::from_utf8(icon.bytes()).unwrap();
            assert!(text.contains("<svg"), "{icon:?}");
            let _ = icon.handle();
        }
    }
}
