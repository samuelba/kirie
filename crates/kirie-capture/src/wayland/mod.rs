//! Capturing screens through the compositor's own Wayland protocols:
//! `ext-image-copy-capture-v1` (COSMIC, newer wlroots and niri) or the older
//! `wlr-screencopy-unstable-v1` (wlroots compositors, Hyprland).

mod cosmic;
mod ext;
mod plasma;
mod shm;
mod toplevels;
mod wlr;

pub use plasma::{Plasma, windows as plasma_windows};
pub use toplevels::{Toplevels, toplevels};

use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry, wl_shm::WlShm};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

use crate::geometry::{Frame, Output, Rect, Transform};
use crate::pixels::{View, to_rgba};
use crate::{CaptureError, Cursor};

pub(crate) const EXT_SOURCES: &str = "ext_output_image_capture_source_manager_v1";
pub(crate) const EXT_COPY: &str = "ext_image_copy_capture_manager_v1";
pub(crate) const WLR_SCREENCOPY: &str = "zwlr_screencopy_manager_v1";
pub(crate) const LAYER_SHELL: &str = "zwlr_layer_shell_v1";

/// What the compositor of this session offers Kirie.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Globals {
    pub ext_image_copy_capture: bool,
    pub wlr_screencopy: bool,
    pub layer_shell: bool,
    pub interfaces: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct OutputState {
    wl: Option<wl_output::WlOutput>,
    name: String,
    description: String,
    transform: Transform,
    scale: i32,
    mode: Option<(i32, i32)>,
    position: (i32, i32),
    logical: Option<Rect>,
}

impl OutputState {
    fn output(&self) -> Output {
        let logical = self.logical.unwrap_or_else(|| {
            let (width, height) = self.mode.unwrap_or_default();
            let scale = self.scale.max(1);
            let (width, height) = match self.transform {
                Transform::Rotate90
                | Transform::Rotate270
                | Transform::Flipped90
                | Transform::Flipped270 => (height, width),
                _ => (width, height),
            };
            Rect::new(
                self.position.0,
                self.position.1,
                width / scale,
                height / scale,
            )
        });
        Output {
            name: self.name.clone(),
            description: self.description.clone(),
            logical,
            transform: self.transform,
        }
    }
}

/// One screen being captured: its buffer, and the result once the compositor answers.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    shm_formats: Vec<u32>,
    size: Option<(u32, u32)>,
    /// The stride wlr-screencopy asks for, per shm format.
    strides: Vec<(u32, u32)>,
    constraints_done: bool,
    buffer: Option<shm::ShmBuffer>,
    y_invert: bool,
    transform: Option<Transform>,
    finished: Option<Result<(), String>>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    outputs: Vec<OutputState>,
    pending: Vec<Pending>,
    toplevels: toplevels::Listing,
    plasma: plasma::Listing,
    /// A time to give up waiting before `ANSWER_TIMEOUT`, for answers Kirie can do without.
    give_up: Option<std::time::Instant>,
}

/// Lists the globals of the session's compositor (`WAYLAND_DISPLAY`).
pub fn globals() -> Result<Globals, CaptureError> {
    let connection = Connection::connect_to_env()?;
    let (list, _queue) = registry_queue_init::<State>(&connection)?;
    Ok(describe(&list))
}

fn describe(list: &GlobalList) -> Globals {
    list.contents().with_list(|list| {
        let offers = |name: &str| list.iter().any(|global| global.interface == name);
        let mut interfaces: Vec<String> =
            list.iter().map(|global| global.interface.clone()).collect();
        interfaces.sort();
        interfaces.dedup();
        Globals {
            ext_image_copy_capture: offers(EXT_SOURCES) && offers(EXT_COPY),
            wlr_screencopy: offers(WLR_SCREENCOPY),
            layer_shell: offers(LAYER_SHELL),
            interfaces,
        }
    })
}

/// The protocol a capture went through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    ExtImageCopyCapture,
    WlrScreencopy,
}

/// The screens and where they sit, without capturing them.
pub fn outputs() -> Result<Vec<Output>, CaptureError> {
    let connection = Connection::connect_to_env()?;
    let (list, mut queue) = registry_queue_init::<State>(&connection)?;
    let mut state = State::default();
    let handle = queue.handle();
    bind_outputs(&list, &handle, &mut state, &mut queue)?;
    Ok(state.outputs.iter().map(OutputState::output).collect())
}

/// Captures every screen with the best protocol the compositor offers.
pub fn capture(cursor: Cursor) -> Result<(Protocol, Vec<Frame>), CaptureError> {
    let connection = Connection::connect_to_env()?;
    let (list, mut queue) = registry_queue_init::<State>(&connection)?;
    let globals = describe(&list);
    let protocol = if globals.ext_image_copy_capture {
        Protocol::ExtImageCopyCapture
    } else if globals.wlr_screencopy {
        Protocol::WlrScreencopy
    } else {
        return Err(CaptureError::Unsupported);
    };
    let mut state = State::default();
    let handle = queue.handle();
    bind_outputs(&list, &handle, &mut state, &mut queue)?;
    let shm: WlShm = list
        .bind(&handle, 1..=1, ())
        .map_err(|error| CaptureError::Protocol(format!("no wl_shm: {error}")))?;
    match protocol {
        Protocol::ExtImageCopyCapture => {
            ext::capture(&list, &shm, &handle, &mut state, &mut queue, cursor)?;
        }
        Protocol::WlrScreencopy => {
            wlr::capture(&list, &shm, &handle, &mut state, &mut queue, cursor)?;
        }
    }
    Ok((protocol, frames(&state)?))
}

fn bind_outputs(
    list: &GlobalList,
    handle: &QueueHandle<State>,
    state: &mut State,
    queue: &mut EventQueue<State>,
) -> Result<(), CaptureError> {
    let found = list.contents().with_list(|globals| {
        globals
            .iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version))
            .collect::<Vec<_>>()
    });
    for (index, (name, version)) in found.into_iter().enumerate() {
        let wl =
            list.registry()
                .bind::<wl_output::WlOutput, _, _>(name, version.min(4), handle, index);
        state.outputs.push(OutputState {
            wl: Some(wl),
            scale: 1,
            ..OutputState::default()
        });
    }
    if state.outputs.is_empty() {
        return Err(CaptureError::NoScreens);
    }
    if let Ok(manager) =
        list.bind::<zxdg_output_manager_v1::ZxdgOutputManagerV1, _, _>(handle, 2..=3, ())
    {
        for (index, output) in state.outputs.iter().enumerate() {
            if let Some(wl) = &output.wl {
                manager.get_xdg_output(wl, handle, index);
            }
        }
    }
    queue.roundtrip(state)?;
    queue.roundtrip(state)?;
    Ok(())
}

/// How long a compositor may take to answer a capture.
const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Waits until the compositor has answered every pending capture, or gives up after
/// `ANSWER_TIMEOUT` so a compositor that never answers cannot hang Kirie.
fn wait(
    state: &mut State,
    queue: &mut EventQueue<State>,
    done: impl Fn(&Pending) -> bool,
) -> Result<(), CaptureError> {
    wait_until(state, queue, |state| state.pending.iter().all(&done))
}

/// Dispatches until `done` holds, or fails after `ANSWER_TIMEOUT`.
fn wait_until(
    state: &mut State,
    queue: &mut EventQueue<State>,
    done: impl Fn(&State) -> bool,
) -> Result<(), CaptureError> {
    let deadline = std::time::Instant::now() + ANSWER_TIMEOUT;
    let deadline = state
        .give_up
        .map_or(deadline, |give_up| give_up.min(deadline));
    loop {
        queue.dispatch_pending(state)?;
        if done(state) {
            return Ok(());
        }
        queue
            .flush()
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(CaptureError::Failed(
                "the compositor did not answer in time".into(),
            ));
        }
        let timeout = rustix::event::Timespec {
            tv_sec: i64::try_from(left.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(left.subsec_nanos()),
        };
        let fd = guard.connection_fd();
        let mut fds = [rustix::event::PollFd::new(
            &fd,
            rustix::event::PollFlags::IN | rustix::event::PollFlags::ERR,
        )];
        let ready = rustix::event::poll(&mut fds, Some(&timeout))
            .map_err(|error| CaptureError::Io(error.into()))?;
        if ready > 0 {
            guard
                .read()
                .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        }
    }
}

fn frames(state: &State) -> Result<Vec<Frame>, CaptureError> {
    state
        .outputs
        .iter()
        .zip(&state.pending)
        .map(|(output, pending)| {
            let image = pending
                .image()
                .map_err(|reason| CaptureError::Failed(format!("{}: {reason}", output.name)))?;
            let transform = pending.transform.unwrap_or(output.transform);
            Ok(Frame {
                output: output.output(),
                image: transform.apply(&image),
            })
        })
        .collect()
}

impl Pending {
    /// The copied pixels, as the compositor laid them out in the buffer.
    fn image(&self) -> Result<image::RgbaImage, String> {
        self.read(|buffer, bytes| {
            to_rgba(
                buffer.format,
                buffer.width,
                buffer.height,
                buffer.stride,
                bytes,
                self.y_invert,
            )
            .map_err(|error| error.to_string())
        })
    }

    /// Looks at the copied pixels in place, for when only a few of them matter.
    fn view<T>(&self, look: impl FnOnce(&View) -> T) -> Result<T, String> {
        self.read(|buffer, bytes| {
            let view = View::new(
                buffer.format,
                buffer.width,
                buffer.height,
                buffer.stride,
                bytes,
                self.y_invert,
            )
            .map_err(|error| error.to_string())?;
            Ok(look(&view))
        })
    }

    fn read<T>(
        &self,
        convert: impl FnOnce(&shm::ShmBuffer, &[u8]) -> Result<T, String>,
    ) -> Result<T, String> {
        match &self.finished {
            Some(Ok(())) => {}
            Some(Err(reason)) => return Err(reason.clone()),
            None => return Err("no answer".into()),
        }
        let buffer = self.buffer.as_ref().ok_or("no buffer")?;
        let bytes = buffer.read().map_err(|error| error.to_string())?;
        convert(buffer, &bytes)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            wl_output::Event::Geometry {
                x, y, transform, ..
            } => {
                output.position = (x, y);
                if let WEnum::Value(transform) = transform {
                    output.transform = Transform::from_wayland(transform.into());
                }
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => output.mode = Some((width, height)),
            wl_output::Event::Scale { factor } => output.scale = factor,
            wl_output::Event::Name { name } => output.name = name,
            wl_output::Event::Description { description } => output.description = description,
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(*index) else {
            return;
        };
        let logical = output.logical.get_or_insert_with(Rect::default);
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => (logical.x, logical.y) = (x, y),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                (logical.width, logical.height) = (width, height);
            }
            zxdg_output_v1::Event::Name { name } if output.name.is_empty() => output.name = name,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore wayland_client::protocol::wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wayland_client::protocol::wl_buffer::WlBuffer);
delegate_noop!(State: zxdg_output_manager_v1::ZxdgOutputManagerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_without_xdg_output_use_mode_and_scale() {
        let output = OutputState {
            name: "DP-1".into(),
            scale: 2,
            mode: Some((3840, 2160)),
            position: (1920, 0),
            ..OutputState::default()
        };
        assert_eq!(output.output().logical, Rect::new(1920, 0, 1920, 1080));
        let rotated = OutputState {
            transform: Transform::Rotate90,
            mode: Some((1920, 1080)),
            scale: 1,
            ..OutputState::default()
        };
        assert_eq!(rotated.output().logical, Rect::new(0, 0, 1080, 1920));
    }

    #[test]
    fn xdg_output_geometry_wins() {
        let output = OutputState {
            scale: 2,
            mode: Some((3840, 2160)),
            logical: Some(Rect::new(0, 0, 2560, 1440)),
            ..OutputState::default()
        };
        assert_eq!(output.output().logical, Rect::new(0, 0, 2560, 1440));
    }
}
