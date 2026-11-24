//! Images dropped on the editor's window, and what is on the clipboard.
//!
//! winit, under iced, takes no drops on Wayland, so Kirie takes them itself: it joins the
//! window's own Wayland connection with an event queue and a data device of its own, as
//! smithay-clipboard does for the clipboard, and reads what is dropped as a list of files, or as
//! PNG bytes when a source offers only those.
//!
//! GNOME sends drops, and the clipboard, to only the data device a client made last, and the
//! clipboard iced keeps makes one too. So the editor [renews](Device::renew) its device each
//! time its window loses the keyboard, before anything can be dragged onto it, and reads the
//! clipboard through that device as well ([`Device::read`]).
//!
//! Where no other program may take the clipboard, as on GNOME, the window offers what is copied
//! itself ([`Device::offer`]), which needs the serial of its latest input: the device listens to
//! the pointer and keyboard for it.

use std::collections::HashMap;
use std::io::Read as _;
use std::os::unix::ffi::OsStringExt as _;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};

const URI_LIST: &str = "text/uri-list";
pub const PNG: &str = "image/png";
/// Text types, best first.
pub const TEXT: [&str; 4] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
];
/// Drops larger than this are not read.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// A source quiet for this long has stopped sending.
const STALL_SECONDS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// The data device is ready; it is handed over once, before any drop.
    Ready(Device),
    /// Something the editor can open is dragged over the window (`true`), or no longer is.
    Hovering(bool),
    Files(Vec<PathBuf>),
    Png(Vec<u8>),
}

/// The editor's data device, shared with the thread that listens to it.
#[derive(Clone)]
pub struct Device {
    shared: Arc<Mutex<Shared>>,
    /// Signals that a reader has taken what the window offers, or that it offers nothing.
    handed: Arc<Condvar>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device").finish_non_exhaustive()
    }
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
}

impl Eq for Device {}

struct Shared {
    connection: Connection,
    handle: QueueHandle<State>,
    manager: Option<WlDataDeviceManager>,
    seats: Vec<Seat>,
    /// What is on the clipboard, with the types it is offered as.
    selection: Option<(WlDataOffer, Vec<String>)>,
    /// What the window offers on the clipboard, until something else is copied.
    own: Option<(WlDataSource, Copied)>,
    /// A reader has taken all of what the window offers.
    handed: bool,
    /// The serial of the window's latest input.
    serial: Option<u32>,
}

/// Bytes the window put on the clipboard, as `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Copied {
    kind: String,
    bytes: Arc<[u8]>,
}

impl Copied {
    /// The bytes with their kind, if that is one of `kinds`.
    fn as_one_of(&self, kinds: &[&str]) -> Option<(String, Vec<u8>)> {
        kinds
            .contains(&self.kind.as_str())
            .then(|| (self.kind.clone(), self.bytes.to_vec()))
    }
}

struct Seat {
    proxy: WlSeat,
    device: Option<WlDataDevice>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
}

impl Shared {
    /// Gives every seat a data device once there is a manager to make them.
    fn attach(&mut self) {
        let Some(manager) = &self.manager else {
            return;
        };
        for seat in &mut self.seats {
            if seat.device.is_none() {
                seat.device = Some(manager.get_data_device(&seat.proxy, &self.handle, ()));
            }
        }
    }

    fn forget_selection(&mut self) {
        if let Some((offer, _)) = self.selection.take() {
            offer.destroy();
        }
    }
}

impl Device {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Makes the data devices anew, so that they are the client's newest: GNOME sends drops and
    /// the clipboard to that one alone.
    ///
    /// The old devices are kept, not released: GNOME may already have sent one an offer, and
    /// libwayland drops an event for a released object without the offer it makes, after which
    /// the next object the compositor makes breaks the connection. Each stays as quiet as GNOME
    /// leaves it, and handled as any other.
    pub fn renew(&self) {
        let mut shared = self.lock();
        shared.forget_selection();
        for seat in &mut shared.seats {
            seat.device = None;
        }
        shared.attach();
        if let Err(error) = shared.connection.flush() {
            tracing::debug!(%error, "cannot renew the data device");
        }
    }

    /// The clipboard's content as the first of `kinds` it is offered as, with that kind; `None`
    /// when this device does not see the clipboard or it holds none of them. Blocks until the
    /// source has sent it all.
    ///
    /// What the window copied itself is read straight from it: GNOME tells this device of it
    /// only once the device is the client's newest.
    pub fn read(&self, kinds: &[&str]) -> Option<std::io::Result<(String, Vec<u8>)>> {
        let (connection, offer, kind) = {
            let shared = self.lock();
            if let Some((_, copied)) = &shared.own {
                return copied.as_one_of(kinds).map(Ok);
            }
            let (offer, types) = shared.selection.as_ref()?;
            let kind = kinds
                .iter()
                .find(|kind| types.iter().any(|offered| offered == *kind))?;
            (shared.connection.clone(), offer.clone(), (*kind).to_owned())
        };
        Some(receive(&connection, &offer, &kind).map(|bytes| (kind, bytes)))
    }

    /// Puts `bytes` on the clipboard as `kind`, offered by the window for as long as it is open.
    /// The window must have the keyboard.
    pub fn offer(&self, bytes: Vec<u8>, kind: &str) -> Result<(), String> {
        let mut shared = self.lock();
        let manager = shared
            .manager
            .as_ref()
            .ok_or("the compositor has no clipboard")?;
        let serial = shared.serial.ok_or("the window has had no input yet")?;
        let device = shared
            .seats
            .iter()
            .find_map(|seat| seat.device.as_ref())
            .ok_or("the window has no seat")?;
        let copied = Copied {
            kind: kind.to_owned(),
            bytes: Arc::from(bytes),
        };
        let source = manager.create_data_source(&shared.handle, Arc::clone(&copied.bytes));
        source.offer(copied.kind.clone());
        device.set_selection(Some(&source), serial);
        shared
            .connection
            .flush()
            .map_err(|error| format!("cannot reach the compositor: {error}"))?;
        shared.own = Some((source, copied));
        shared.handed = false;
        Ok(())
    }

    /// Waits up to `timeout` until a reader has taken what the window offers, as GNOME does at
    /// once to keep it after the window closes; true when it has, or the window offers nothing.
    pub fn wait_handed(&self, timeout: Duration) -> bool {
        let (_shared, waited) = self
            .handed
            .wait_timeout_while(self.lock(), timeout, |shared| {
                shared.own.is_some() && !shared.handed
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !waited.timed_out()
    }

    fn handed(&self, source: &WlDataSource) {
        let mut shared = self.lock();
        if shared.own.as_ref().is_some_and(|(own, _)| own == source) {
            shared.handed = true;
        }
        self.handed.notify_all();
    }
}

/// What is on the clipboard as the first of `kinds` it is offered as, read through `device`;
/// `None` when the device does not see the clipboard or it holds none of them.
pub async fn clipboard(device: Option<Device>, kinds: &'static [&'static str]) -> Option<Vec<u8>> {
    clipboard_kind(device, kinds).await.map(|(_, bytes)| bytes)
}

/// Like [`clipboard`], with the kind it was read as.
pub async fn clipboard_kind(
    device: Option<Device>,
    kinds: &'static [&'static str],
) -> Option<(String, Vec<u8>)> {
    let device = device?;
    let read = tokio::task::spawn_blocking(move || device.read(kinds))
        .await
        .ok()??;
    read.inspect_err(|error| tracing::debug!(%error, "cannot read the clipboard"))
        .ok()
        .filter(|(_, bytes)| !bytes.is_empty())
}

/// The Wayland display of `window`, as an address so it can cross to another thread.
pub fn display(window: &dyn iced::window::Window) -> Option<usize> {
    use iced::window::raw_window_handle::RawDisplayHandle;
    match window.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(handle) => Some(handle.display.as_ptr().expose_provenance()),
        _ => None,
    }
}

/// What is dropped on the windows of `display`, for as long as the stream is kept, after the
/// [`Device`] that takes it.
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
    let device = Device {
        shared: Arc::new(Mutex::new(Shared {
            connection: connection.clone(),
            handle,
            manager: None,
            seats: Vec::new(),
            selection: None,
            own: None,
            handed: false,
            serial: None,
        })),
        handed: Arc::new(Condvar::new()),
    };
    let mut state = State {
        connection,
        sender,
        device: device.clone(),
        offers: HashMap::new(),
        dragged: None,
    };
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    if device.lock().manager.is_none() {
        return Err("the compositor has no drag and drop".to_owned());
    }
    let _ = state.sender.unbounded_send(Dropped::Ready(device));
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
    device: Device,
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
    fn hover(&self, hovering: bool) {
        let _ = self.sender.unbounded_send(Dropped::Hovering(hovering));
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
        let mut shared = state.device.lock();
        match interface.as_str() {
            "wl_data_device_manager" => {
                shared.manager = Some(registry.bind(name, version.min(3), handle, ()));
            }
            "wl_seat" => shared.seats.push(Seat {
                proxy: registry.bind(name, 1, handle, ()),
                device: None,
                pointer: None,
                keyboard: None,
            }),
            _ => return,
        }
        shared.attach();
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        let mut shared = state.device.lock();
        let Some(found) = shared.seats.iter_mut().find(|found| found.proxy == *seat) else {
            return;
        };
        if capabilities.contains(Capability::Pointer) && found.pointer.is_none() {
            found.pointer = Some(seat.get_pointer(handle, ()));
        }
        if capabilities.contains(Capability::Keyboard) && found.keyboard.is_none() {
            found.keyboard = Some(seat.get_keyboard(handle, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Enter { serial, .. } | wl_pointer::Event::Button { serial, .. } =
            event
        {
            state.device.lock().serial = Some(serial);
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { serial, .. } | wl_keyboard::Event::Key { serial, .. } =
            event
        {
            state.device.lock().serial = Some(serial);
        }
    }
}

impl Dispatch<WlDataSource, Arc<[u8]>> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        bytes: &Arc<[u8]>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { fd, .. } => {
                let bytes = Arc::clone(bytes);
                let (device, source) = (state.device.clone(), source.clone());
                // A reader as slow as it likes, even this window pasting, must not hold up
                // the queue.
                let spawned =
                    std::thread::Builder::new()
                        .name("kirie-copy".into())
                        .spawn(move || {
                            use std::io::Write as _;
                            match std::fs::File::from(fd).write_all(&bytes) {
                                Ok(()) => device.handed(&source),
                                Err(error) => {
                                    tracing::debug!(%error, "cannot hand over the clipboard");
                                }
                            }
                        });
                if let Err(error) = spawned {
                    tracing::warn!(%error, "cannot hand over the clipboard");
                }
            }
            wl_data_source::Event::Cancelled => {
                let mut shared = state.device.lock();
                if shared.own.as_ref().is_some_and(|(own, _)| own == source) {
                    shared.own = None;
                }
                drop(shared);
                state.device.handed.notify_all();
                source.destroy();
            }
            _ => {}
        }
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
                state.hover(kind.is_some());
            }
            wl_data_device::Event::Leave => {
                state.hover(false);
                if let Some(offer) = state.dragged.take() {
                    state.forget(&offer);
                }
            }
            wl_data_device::Event::Drop => {
                state.hover(false);
                if let Some(offer) = state.dragged.take() {
                    state.take(&offer);
                }
            }
            wl_data_device::Event::Selection { id } => {
                let types = id
                    .as_ref()
                    .and_then(|offer| state.offers.remove(&offer.id()))
                    .unwrap_or_default()
                    .types;
                let mut shared = state.device.lock();
                shared.forget_selection();
                shared.selection = id.map(|offer| (offer, types));
            }
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
    fn own_copies_paste_only_as_a_wanted_kind() {
        let copied = Copied {
            kind: PNG.to_owned(),
            bytes: Arc::from(&b"png"[..]),
        };
        assert_eq!(
            copied.as_one_of(&[kirie_annotate::marks::MIME, PNG]),
            Some((PNG.to_owned(), b"png".to_vec()))
        );
        assert_eq!(copied.as_one_of(&TEXT), None);
    }

    #[test]
    fn file_lists_win_over_pictures() {
        let types = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(wanted(&types(&[PNG, URI_LIST])), Some(URI_LIST));
        assert_eq!(wanted(&types(&["text/plain", PNG])), Some(PNG));
        assert_eq!(wanted(&types(&["text/plain"])), None);
    }
}
