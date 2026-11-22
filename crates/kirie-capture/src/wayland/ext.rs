//! `ext-image-copy-capture-v1`: a session per screen tells the buffer it wants, then one frame
//! copies the screen into it.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};

use super::shm::ShmBuffer;
use super::{Pending, State, wait};
use crate::geometry::Transform;
use crate::pixels::ShmFormat;
use crate::{CaptureError, Cursor};

pub(super) fn capture(
    list: &GlobalList,
    shm: &WlShm,
    handle: &QueueHandle<State>,
    state: &mut State,
    queue: &mut EventQueue<State>,
    cursor: Cursor,
) -> Result<(), CaptureError> {
    let bind_error = |error| CaptureError::Protocol(format!("{error}"));
    let sources: ExtOutputImageCaptureSourceManagerV1 =
        list.bind(handle, 1..=1, ()).map_err(bind_error)?;
    let manager: ExtImageCopyCaptureManagerV1 = list.bind(handle, 1..=1, ()).map_err(bind_error)?;
    let options = match cursor {
        Cursor::Hidden => ext_image_copy_capture_manager_v1::Options::empty(),
        Cursor::Shown => ext_image_copy_capture_manager_v1::Options::PaintCursors,
    };
    let mut sessions = Vec::new();
    for (index, output) in state.outputs.iter().enumerate() {
        let wl = output.wl.as_ref().expect("bound outputs have a wl_output");
        let source = sources.create_source(wl, handle, ());
        sessions.push((
            manager.create_session(&source, options, handle, index),
            source,
        ));
    }
    state.pending = (0..sessions.len()).map(|_| Pending::default()).collect();
    wait(state, queue, |pending| {
        pending.constraints_done || pending.finished.is_some()
    })?;

    for (index, (session, _)) in sessions.iter().enumerate() {
        let pending = &mut state.pending[index];
        if pending.finished.is_some() {
            continue;
        }
        let (width, height) = pending
            .size
            .ok_or_else(|| CaptureError::Protocol("no buffer size".into()))?;
        let format = ShmFormat::pick(pending.shm_formats.iter().copied())
            .ok_or_else(|| CaptureError::Protocol("no 32-bit shm format offered".into()))?;
        let buffer = ShmBuffer::new(shm, handle, width, height, width * 4, format)?;
        let frame = session.create_frame(handle, index);
        frame.attach_buffer(&buffer.buffer);
        frame.damage_buffer(0, 0, i32::MAX, i32::MAX);
        frame.capture();
        pending.buffer = Some(buffer);
    }
    wait(state, queue, |pending| pending.finished.is_some())?;

    for (session, source) in sessions {
        session.destroy();
        source.destroy();
    }
    manager.destroy();
    sources.destroy();
    queue.roundtrip(state)?;
    Ok(())
}

impl Dispatch<ExtImageCopyCaptureSessionV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(pending) = state.pending.get_mut(*index) else {
            return;
        };
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                pending.size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                pending.shm_formats.push(format.into());
            }
            ext_image_copy_capture_session_v1::Event::Done => pending.constraints_done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => {
                pending
                    .finished
                    .get_or_insert(Err("the compositor stopped the capture".into()));
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(pending) = state.pending.get_mut(*index) else {
            return;
        };
        match event {
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: WEnum::Value(transform),
            } => pending.transform = Some(Transform::from_wayland(transform.into())),
            ext_image_copy_capture_frame_v1::Event::Ready => {
                pending.finished = Some(Ok(()));
                frame.destroy();
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                let reason = match reason {
                    WEnum::Value(ext_image_copy_capture_frame_v1::FailureReason::Stopped) => {
                        "the screen went away"
                    }
                    WEnum::Value(
                        ext_image_copy_capture_frame_v1::FailureReason::BufferConstraints,
                    ) => "the buffer no longer fits",
                    _ => "the compositor refused",
                };
                pending.finished = Some(Err(reason.into()));
                frame.destroy();
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ExtImageCaptureSourceV1);
delegate_noop!(State: ExtImageCopyCaptureManagerV1);
