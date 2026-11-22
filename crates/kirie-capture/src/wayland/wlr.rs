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
    wait(state, queue, |pending| {
        pending.finished.is_some()
            || if announces_done {
                pending.constraints_done
            } else {
                pending.size.is_some()
            }
    })?;

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
            } => {
                let format: u32 = format.into();
                // Every shm buffer type of a frame has the same size; only keep readable ones.
                if ShmFormat::from_wayland(format).is_some() {
                    pending.shm_formats.push(format);
                    pending.size = Some((width, height));
                    pending.strides.push((format, stride));
                }
            }
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
