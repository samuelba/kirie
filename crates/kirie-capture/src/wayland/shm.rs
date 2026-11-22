//! Shared-memory buffers the compositor copies a screen into.
//!
//! The buffer is a memfd. Kirie never maps it: the compositor writes through its own mapping and
//! Kirie reads the pixels back through the file descriptor, which keeps this crate free of
//! `unsafe`.

use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_buffer::WlBuffer, wl_shm, wl_shm::WlShm};

use super::State;
use crate::CaptureError;
use crate::pixels::ShmFormat;

#[derive(Debug)]
pub(super) struct ShmBuffer {
    file: File,
    pub buffer: WlBuffer,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: ShmFormat,
}

impl ShmBuffer {
    pub fn new(
        shm: &WlShm,
        handle: &QueueHandle<State>,
        width: u32,
        height: u32,
        stride: u32,
        format: ShmFormat,
    ) -> Result<Self, CaptureError> {
        let size = u64::from(stride) * u64::from(height);
        let too_big = || CaptureError::Protocol(format!("a {width}×{height} buffer is too big"));
        let pool_size = i32::try_from(size).map_err(|_| too_big())?;
        let fd = memfd_create("kirie-capture", MemfdFlags::CLOEXEC)
            .map_err(|error| CaptureError::Io(error.into()))?;
        let file = File::from(fd);
        file.set_len(size)?;
        let wl_format = wl_shm::Format::try_from(format.to_wayland())
            .map_err(|()| CaptureError::Protocol("unknown shm format".into()))?;
        let pool = shm.create_pool(file.as_fd(), pool_size, handle, ());
        let buffer = pool.create_buffer(
            0,
            i32::try_from(width).map_err(|_| too_big())?,
            i32::try_from(height).map_err(|_| too_big())?,
            i32::try_from(stride).map_err(|_| too_big())?,
            wl_format,
            handle,
            (),
        );
        pool.destroy();
        Ok(Self {
            file,
            buffer,
            width,
            height,
            stride,
            format,
        })
    }

    pub fn read(&self) -> Result<Vec<u8>, CaptureError> {
        let len = usize::try_from(u64::from(self.stride) * u64::from(self.height))
            .map_err(|_| CaptureError::Protocol("the buffer is too big".into()))?;
        let mut bytes = vec![0; len];
        self.file.read_exact_at(&mut bytes, 0)?;
        Ok(bytes)
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}
