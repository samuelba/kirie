//! Bringing the editor's window to the front with an XDG activation token.
//!
//! On Wayland a window comes to the front only with a token from whatever the user just clicked:
//! the launcher, the file manager or the tray hands it over in `XDG_ACTIVATION_TOKEN`, and
//! another Kirie passes it on over D-Bus. iced has no way to use one, so Kirie joins the
//! window's own Wayland connection and asks the compositor itself, as it does for drops.

use iced::window::raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

/// The variable a launcher puts the token in.
pub const ENV: &str = "XDG_ACTIVATION_TOKEN";

/// The token this Kirie was started with, if any.
pub fn from_env() -> Option<String> {
    std::env::var(ENV).ok().filter(|token| !token.is_empty())
}

/// Asks the compositor to bring `window` to the front with `token`. False when it cannot be
/// asked: not on Wayland, or without the activation protocol.
pub fn activate(window: &dyn iced::window::Window, token: &str) -> bool {
    match request(window, token) {
        Ok(()) => true,
        Err(error) => {
            tracing::debug!(%error, "cannot bring the window to the front with a token");
            false
        }
    }
}

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore XdgActivationV1);

fn request(window: &dyn iced::window::Window, token: &str) -> Result<(), String> {
    let RawDisplayHandle::Wayland(display) = window
        .display_handle()
        .map_err(|error| error.to_string())?
        .as_raw()
    else {
        return Err("not on Wayland".to_owned());
    };
    let RawWindowHandle::Wayland(surface) = window
        .window_handle()
        .map_err(|error| error.to_string())?
        .as_raw()
    else {
        return Err("not on Wayland".to_owned());
    };
    let (connection, surface) = join(display.display.as_ptr(), surface.surface.as_ptr())?;
    let (globals, queue) =
        registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
    let activation: XdgActivationV1 = globals
        .bind(&queue.handle(), 1..=1, ())
        .map_err(|error| error.to_string())?;
    activation.activate(token.to_owned(), &surface);
    activation.destroy();
    connection.flush().map_err(|error| error.to_string())
}

#[expect(
    unsafe_code,
    reason = "the window's display and surface are reached only through the window, which holds them"
)]
fn join(
    display: *mut std::ffi::c_void,
    surface: *mut std::ffi::c_void,
) -> Result<(Connection, WlSurface), String> {
    use wayland_client::backend::{Backend, ObjectId};

    // SAFETY: `display` is the wl_display of a window that is open while this runs; wayland-client
    // keeps a queue of its own on it.
    let connection =
        Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
    // SAFETY: `surface` is that window's wl_surface, a proxy on the same display.
    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }
        .map_err(|error| error.to_string())?;
    let surface = WlSurface::from_id(&connection, id).map_err(|error| error.to_string())?;
    Ok((connection, surface))
}
