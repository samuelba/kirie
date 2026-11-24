//! `wlr-screencopy-unstable-v1`: one frame per screen describes its buffer, then copies into it.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use super::shm::ShmBuffer;
use super::{Pending, State, wait};
use crate::pixels::ShmFormat;
use crate::{CaptureError, Cursor};

/// Version 3 announces every buffer type before `buffer_done`; older ones send only `buffer`.
const BUFFER_DONE: u32 = 3;

pub(super) fn capture(
    list: &GlobalList,
    shm: &WlShm,
    handle: &QueueHandle<State>,
    state: &mut State,
    queue: &mut EventQueue<State>,
    cursor: Cursor,
) -> Result<(), CaptureError> {
    let manager: ZwlrScreencopyManagerV1 = list
        .bind(handle, 1..=BUFFER_DONE, ())
        .map_err(|error| CaptureError::Protocol(format!("{error}")))?;
    let overlay_cursor = i32::from(cursor == crate::Cursor::Shown);
    let mut frames = Vec::new();
    for (index, wl) in state
        .outputs
        .iter()
        .enumerate()
        .filter_map(|(index, output)| Some((index, output.wl.as_ref()?)))
    {
        frames.push(manager.capture_output(overlay_cursor, wl, handle, index));
    }
    state.pending = (0..frames.len()).map(|_| Pending::default()).collect();
    let announces_done = manager.version() >= BUFFER_DONE;
    wait(state, queue, |pending| described(pending, announces_done))?;

    for (index, frame) in frames.iter().enumerate() {
        let pending = &mut state.pending[index];
        if pending.finished.is_some() {
            continue;
        }
        let (width, height) = pending
            .size
            .ok_or_else(|| CaptureError::Protocol("no buffer size".into()))?;
        let format = ShmFormat::pick(pending.shm_formats.iter().copied())
            .ok_or_else(|| CaptureError::Protocol("no 32-bit shm format offered".into()))?;
        let stride = pending
            .strides
            .iter()
            .find(|(offered, _)| *offered == format.to_wayland())
            .map(|(_, stride)| *stride)
            .ok_or_else(|| CaptureError::Protocol("no stride for the buffer format".into()))?;
        let buffer = ShmBuffer::new(shm, handle, width, height, stride, format)?;
        frame.copy(&buffer.buffer);
        pending.buffer = Some(buffer);
    }
    wait(state, queue, |pending| pending.finished.is_some())?;

    for frame in frames {
        frame.destroy();
    }
    manager.destroy();
    queue
        .flush()
        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
    Ok(())
}

/// Whether the compositor said all it will about a frame's buffer, or that the frame is over.
/// Before version 3 it says nothing after the first buffer type, readable or not.
fn described(pending: &Pending, announces_done: bool) -> bool {
    pending.finished.is_some()
        || if announces_done {
            pending.constraints_done
        } else {
            pending.size.is_some()
        }
}

/// Notes a buffer type offered for a frame. Every type has the same size; only readable ones
/// are kept to copy into.
fn offer(pending: &mut Pending, format: u32, size: (u32, u32), stride: u32) {
    pending.size = Some(size);
    if ShmFormat::from_wayland(format).is_some() {
        pending.shm_formats.push(format);
        pending.strides.push((format, stride));
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(pending) = state.pending.get_mut(*index) else {
            return;
        };
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => offer(pending, format.into(), (width, height), stride),
            zwlr_screencopy_frame_v1::Event::BufferDone => pending.constraints_done = true,
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: WEnum::Value(flags),
            } => pending.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => pending.finished = Some(Ok(())),
            zwlr_screencopy_frame_v1::Event::Failed => {
                pending.finished = Some(Err("the compositor refused".into()));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ZwlrScreencopyManagerV1);

#[cfg(test)]
mod tests {
    use wayland_client::protocol::wl_shm::Format;

    use super::*;

    #[test]
    fn an_old_compositor_offering_only_an_unreadable_format_answers_at_once() {
        let mut pending = Pending::default();
        offer(&mut pending, Format::Xrgb2101010.into(), (1920, 1080), 7680);
        assert!(described(&pending, false));
        assert!(ShmFormat::pick(pending.shm_formats.iter().copied()).is_none());
    }

    #[test]
    fn readable_formats_are_kept_with_their_stride() {
        let mut pending = Pending::default();
        offer(&mut pending, Format::Xrgb2101010.into(), (640, 480), 2560);
        offer(&mut pending, Format::Xrgb8888.into(), (640, 480), 2560);
        assert_eq!(pending.size, Some((640, 480)));
        assert_eq!(pending.shm_formats, [u32::from(Format::Xrgb8888)]);
        assert_eq!(pending.strides, [(u32::from(Format::Xrgb8888), 2560)]);
        assert!(!described(&pending, true));
        pending.constraints_done = true;
        assert!(described(&pending, true));
    }
}
