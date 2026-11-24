//! Screen capture for Kirie: every screen at its native resolution, laid out in the
//! compositor's logical coordinates.

mod activities;
mod bus;
pub mod geometry;
pub mod pixels;
pub mod portal;
pub mod shell;
mod stacking;
pub mod wayland;
pub mod windows;

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
    #[error("the screenshot portal failed: {0}")]
    Portal(String),
    #[error("the GNOME Shell extension failed: {0}")]
    Shell(String),
    #[error("the screenshot was canceled")]
    Canceled,
    #[error(transparent)]
    Pixels(#[from] pixels::PixelError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// How a capture was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Wayland(wayland::Protocol),
    /// Kirie's GNOME Shell extension.
    Shell,
    Portal,
}

/// Captures every screen: through Kirie's GNOME Shell extension when it started this Kirie,
/// through the compositor's capture protocol when it has one, else through the screenshot
/// portal. `KIRIE_CAPTURE=portal` forces the portal.
pub async fn capture(cursor: Cursor) -> Result<(Backend, Vec<Frame>), CaptureError> {
    if shell::launched() {
        // The extension hands its trusted Wayland connection over once (WAYLAND_SOCKET); it
        // must be left for the overlay's windows, so no other Wayland connection is made here.
        return Ok((Backend::Shell, shell::screens(&[]).await?));
    }
    let forced_portal = std::env::var("KIRIE_CAPTURE").is_ok_and(|value| value == "portal");
    if !forced_portal {
        match wayland::capture(cursor) {
            Ok((protocol, frames)) => return Ok((Backend::Wayland(protocol), frames)),
            Err(CaptureError::Unsupported) => {}
            Err(error) => {
                tracing::warn!(%error, "the compositor's capture failed, asking the portal");
            }
        }
    }
    let outputs = wayland::outputs()?;
    let image = portal::screenshot(false).await?;
    let frames = portal::slice(&image, &outputs);
    if frames.is_empty() {
        return Err(CaptureError::NoScreens);
    }
    Ok((Backend::Portal, frames))
}
