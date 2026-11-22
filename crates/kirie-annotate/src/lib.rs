//! Annotations for Kirie: a document of marks over an image, styles that turn marks into
//! paths, and a renderer that draws them, shared by the on-screen overlay and the editor.

pub mod document;
pub mod edit;
pub mod effects;
pub mod primitive;
pub mod project;
pub mod render;
pub mod shape;
pub mod style;
pub mod text;
pub mod tools;

pub use document::{Document, History, Op};
pub use primitive::{Effect, FillRule, Font, Primitive};
pub use shape::{Kind, Paint, Rgba, Shape, ShapeId, Weight};
pub use style::StyleId;
pub use text::Typesetter;
pub use tools::{Draft, Press, Tool};
