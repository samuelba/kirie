//! Images dropped on the editor's window.
//!
//! winit, under iced, takes no drops on Wayland, so Kirie takes them itself: it joins the
//! window's own Wayland connection with an event queue and a data device of its own, as
//! smithay-clipboard does for the clipboard, and reads what is dropped as a list of files, or as
//! PNG bytes when a source offers only those.

use std::collections::HashMap;
use std::io::Read as _;
use std::os::unix::ffi::OsStringExt as _;
use std::path::PathBuf;

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};

const URI_LIST: &str = "text/uri-list";
const PNG: &str = "image/png";
/// Drops larger than this are not read.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// A source quiet for this long has stopped sending.
const STALL_SECONDS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    Files(Vec<PathBuf>),
    Png(Vec<u8>),
}

/// The Wayland display of `window`, as an address so it can cross to another thread.
pub fn display(window: &dyn iced::window::Window) -> Option<usize> {
    use iced::window::raw_window_handle::RawDisplayHandle;
    match window.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(handle) => Some(handle.display.as_ptr().expose_provenance()),
        _ => None,
    }
}

/// What is dropped on the windows of `display`, for as long as the stream is kept.
pub fn drops(display: usize) -> impl Stream<Item = Dropped> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("kirie-drops".into())
        .spawn(move || {
            if let Err(error) = listen(display, sender) {
                tracing::warn!(%error, "files dropped on the editor cannot be taken");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "files dropped on the editor cannot be taken");
    }
    receiver
}

#[expect(
    unsafe_code,
    reason = "drops reach only the connection that owns the window, which winit holds"
)]
fn connect(display: usize) -> Connection {
    let display = std::ptr::with_exposed_provenance_mut(display);
    // SAFETY: `display` is the wl_display of the editor's window, alive until the app exits,
    // which also ends this thread; wayland-client keeps a queue of its own on it.
    Connection::from_backend(unsafe { Backend::from_foreign_display(display) })
}

fn listen(display: usize, sender: mpsc::UnboundedSender<Dropped>) -> Result<(), String> {
    let connection = connect(display);
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());
    let mut state = State {
        connection: connection.clone(),
        sender,
        manager: None,
        seats: Vec::new(),
        offers: HashMap::new(),
        dragged: None,
    };
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    if state.manager.is_none() {
        return Err("the compositor has no drag and drop".to_owned());
    }
    while !state.sender.is_closed() {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

struct State {
    connection: Connection,
    sender: mpsc::UnboundedSender<Dropped>,
    manager: Option<WlDataDeviceManager>,
    seats: Vec<(WlSeat, Option<WlDataDevice>)>,
    offers: HashMap<ObjectId, Offer>,
    /// The offer of the drag over the window.
    dragged: Option<WlDataOffer>,
}

#[derive(Debug, Default)]
struct Offer {
    types: Vec<String>,
    /// The compositor settled on copying.
    copy: bool,
}

impl State {
    /// Gives every seat a data device once there is a manager to make them.
    fn attach(&mut self, handle: &QueueHandle<Self>) {
        let Some(manager) = &self.manager else {
            return;
        };
        for (seat, device) in &mut self.seats {
            if device.is_none() {
                *device = Some(manager.get_data_device(seat, handle, ()));
            }
        }
    }

    fn forget(&mut self, offer: &WlDataOffer) {
        self.offers.remove(&offer.id());
        offer.destroy();
    }

    /// Reads what was dropped and hands it on.
    fn take(&mut self, offer: &WlDataOffer) {
        let found = self.offers.remove(&offer.id()).unwrap_or_default();
        let read = wanted(&found.types).map(|kind| (kind, receive(&self.connection, offer, kind)));
        if offer.version() >= 3 && found.copy {
            offer.finish();
        }
        offer.destroy();
        let dropped = match read {
            Some((URI_LIST, Ok(bytes))) => {
                let files = files(&String::from_utf8_lossy(&bytes));
                (!files.is_empty()).then_some(Dropped::Files(files))
            }
            Some((_, Ok(bytes))) => Some(Dropped::Png(bytes)),
            Some((_, Err(error))) => {
                tracing::warn!(%error, "cannot read what was dropped");
                None
            }
            None => None,
        };
        if let Some(dropped) = dropped {
            let _ = self.sender.unbounded_send(dropped);
        }
    }
}

/// The kind of data to take from a drop offering `types`, if any.
fn wanted(types: &[String]) -> Option<&'static str> {
    [URI_LIST, PNG]
        .into_iter()
        .find(|kind| types.iter().any(|offered| offered == kind))
}

fn receive(connection: &Connection, offer: &WlDataOffer, kind: &str) -> std::io::Result<Vec<u8>> {
    let (read, write) = rustix::pipe::pipe()?;
    offer.receive(kind.to_owned(), std::os::fd::AsFd::as_fd(&write));
    connection.flush().map_err(std::io::Error::other)?;
    drop(write);
    read_all(std::fs::File::from(read))
}

/// Everything until the source closes the pipe; a source that stalls or sends too much is cut
/// off with an error.
fn read_all(mut pipe: std::fs::File) -> std::io::Result<Vec<u8>> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let stalled =
        || std::io::Error::new(std::io::ErrorKind::TimedOut, "the source stopped sending");
    let mut bytes = Vec::new();
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let mut ready = [PollFd::new(&pipe, PollFlags::IN)];
        let wait = Timespec {
            tv_sec: STALL_SECONDS,
            tv_nsec: 0,
        };
        if poll(&mut ready, Some(&wait))? == 0 {
            return Err(stalled());
        }
        let read = pipe.read(&mut chunk)?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > MAX_BYTES {
            return Err(std::io::Error::other("what was dropped is too large"));
        }
    }
}

/// The local files in a `text/uri-list`.
fn files(list: &str) -> Vec<PathBuf> {
    list.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|uri| uri.strip_prefix("file://"))
        .filter_map(|rest| rest.find('/').map(|path| &rest[path..]))
        .map(|path| PathBuf::from(std::ffi::OsString::from_vec(unescape(path))))
        .collect()
}

/// `%xx` escapes turned back into bytes.
fn unescape(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .filter(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escaped {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_data_device_manager" => {
                state.manager = Some(registry.bind(name, version.min(3), handle, ()));
            }
            "wl_seat" => state.seats.push((registry.bind(name, 1, handle, ()), None)),
            _ => return,
        }
        state.attach(handle);
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Offer::default());
            }
            wl_data_device::Event::Enter {
                serial,
                id: Some(offer),
                ..
            } => {
                let types = state.offers.get(&offer.id()).map(|found| &found.types[..]);
                let kind = types.and_then(wanted);
                offer.accept(serial, kind.map(str::to_owned));
                if kind.is_some() && offer.version() >= 3 {
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
                if let Some(old) = state.dragged.replace(offer) {
                    state.forget(&old);
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.dragged.take() {
                    state.forget(&offer);
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(offer) = state.dragged.take() {
                    state.take(&offer);
                }
            }
            wl_data_device::Event::Selection { id: Some(offer) } => state.forget(&offer),
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(found) = state.offers.get_mut(&offer.id()) else {
            return;
        };
        match event {
            wl_data_offer::Event::Offer { mime_type } => found.types.push(mime_type),
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => found.copy = action.contains(DndAction::Copy),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_give_local_files() {
        let list = "# from a file manager\r\n\
                    file:///home/me/Pictures/shot%20one.png\r\n\
                    file://laptop/tmp/a%C3%A9.kirie\r\n\
                    https://example.com/remote.png\r\n\
                    \r\n";
        assert_eq!(
            files(list),
            [
                PathBuf::from("/home/me/Pictures/shot one.png"),
                PathBuf::from("/tmp/aé.kirie")
            ]
        );
        assert!(files("").is_empty());
    }

    #[test]
    fn broken_escapes_stay_as_they_are() {
        assert_eq!(unescape("100%"), b"100%");
        assert_eq!(unescape("%zz%4"), b"%zz%4");
        assert_eq!(unescape("%41b"), b"Ab");
        assert_eq!(unescape("%+4"), b"%+4");
    }

    #[test]
    fn pipes_are_read_to_the_end() {
        let (read, write) = rustix::pipe::pipe().unwrap();
        let mut write = std::fs::File::from(write);
        std::io::Write::write_all(&mut write, b"file:///tmp/a.png\r\n").unwrap();
        drop(write);
        let bytes = read_all(std::fs::File::from(read)).unwrap();
        assert_eq!(bytes, b"file:///tmp/a.png\r\n");
    }

    #[test]
    fn file_lists_win_over_pictures() {
        let types = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(wanted(&types(&[PNG, URI_LIST])), Some(URI_LIST));
        assert_eq!(wanted(&types(&["text/plain", PNG])), Some(PNG));
        assert_eq!(wanted(&types(&["text/plain"])), None);
    }
}
