//! Renders every mark in every style side by side: `cargo run --example showcase -- out.png`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

use image::{Rgba as Pixel, RgbaImage};
use kirie_annotate::{Document, Kind, Op, Paint, StyleId, Typesetter, Weight, render::render};
use kurbo::{Point, Rect};

const COLUMN: f64 = 420.0;

fn backdrop(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let stripe = (x / 24 + y / 24) % 2 == 0;
        if stripe {
            Pixel([236, 233, 226, 255])
        } else {
            Pixel([214, 220, 230, 255])
        }
    })
}

fn marks(x: f64) -> Vec<(Kind, Paint)> {
    let plain = Paint::default();
    let filled = Paint {
        filled: true,
        ..plain
    };
    let p = |dx: f64, y: f64| Point::new(x + dx, y);
    vec![
        (
            Kind::Arrow {
                from: p(20.0, 120.0),
                to: p(170.0, 50.0),
            },
            plain,
        ),
        (
            Kind::Line {
                from: p(210.0, 50.0),
                to: p(380.0, 110.0),
            },
            Paint {
                weight: Weight::THIN,
                ..plain
            },
        ),
        (
            Kind::Rectangle {
                rect: Rect::new(x + 20.0, 160.0, x + 180.0, 260.0),
            },
            filled,
        ),
        (
            Kind::Ellipse {
                rect: Rect::new(x + 210.0, 160.0, x + 390.0, 260.0),
            },
            plain,
        ),
        (
            Kind::Pen {
                points: (0..40)
                    .map(|i| {
                        let t = f64::from(i) / 39.0;
                        p(20.0 + t * 160.0, 330.0 + (t * 12.0).sin() * 25.0)
                    })
                    .collect(),
            },
            Paint {
                weight: Weight::THICK,
                ..plain
            },
        ),
        (
            Kind::Highlighter {
                points: vec![p(210.0, 330.0), p(390.0, 330.0)],
            },
            Paint {
                color: kirie_annotate::Rgba::rgb(0xf2, 0xb1, 0x34),
                ..plain
            },
        ),
        (
            Kind::Text {
                at: p(20.0, 390.0),
                text: "Click here".into(),
            },
            plain,
        ),
        (
            Kind::Counter {
                at: p(250.0, 410.0),
                number: 1,
            },
            plain,
        ),
        (
            Kind::Counter {
                at: p(310.0, 410.0),
                number: 2,
            },
            Paint {
                weight: Weight::THICK,
                ..plain
            },
        ),
        (
            Kind::Redact {
                rect: Rect::new(x + 20.0, 460.0, x + 180.0, 500.0),
            },
            plain,
        ),
        (
            Kind::Blur {
                rect: Rect::new(x + 210.0, 450.0, x + 300.0, 510.0),
            },
            plain,
        ),
        (
            Kind::Pixelate {
                rect: Rect::new(x + 310.0, 450.0, x + 390.0, 510.0),
            },
            plain,
        ),
    ]
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "showcase.png".into());
    let (width, height) = ((COLUMN * 3.0) as u32, 540);
    let mut image = backdrop(width, height);
    let mut typesetter = Typesetter::new();
    for (column, style) in StyleId::ALL.into_iter().enumerate() {
        let x = COLUMN * column as f64;
        let mut document = Document::new(width, height, 1.0).apply(Op::SetStyle(style));
        for (kind, paint) in marks(x) {
            document = document.apply(Op::Add { kind, paint });
        }
        document = document.apply(Op::Add {
            kind: Kind::Text {
                at: Point::new(x + 20.0, 4.0),
                text: style.name().into(),
            },
            paint: Paint {
                color: kirie_annotate::Rgba::rgb(0x16, 0x18, 0x1d),
                weight: Weight::THIN,
                filled: false,
                text: kirie_annotate::TextLook {
                    size: 16.0,
                    ..kirie_annotate::TextLook::default()
                },
            },
        });
        image = render(&document, &image, &mut typesetter).expect("renders");
    }
    image.save(&out).expect("writes the showcase");
    println!("{out}");
}
