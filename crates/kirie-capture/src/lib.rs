//! Screen capture for Kirie: every screen at its native resolution, laid out in the
//! compositor's logical coordinates.

pub mod geometry;
pub mod pixels;
pub mod wayland;

pub use geometry::{Frame, Output, Rect, Transform};

/// Whether the mouse pointer is drawn into the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Hidden,
    Shown,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no Wayland compositor: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("cannot list the compositor's globals: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    #[error("the Wayland connection failed: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
    #[error("the compositor offers no screen capture protocol Kirie knows")]
    Unsupported,
    #[error("the compositor reports no screens")]
    NoScreens,
    #[error("unexpected answer from the compositor: {0}")]
    Protocol(String),
    #[error("capturing {0}")]
    Failed(String),
    #[error(transparent)]
    Pixels(#[from] pixels::PixelError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
