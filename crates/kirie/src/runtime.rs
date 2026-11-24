//! The async runtime for the work Kirie does outside a UI event loop: D-Bus calls, portals and
//! saves, before and after the overlay or the editor runs.

use std::sync::LazyLock;

use tokio::runtime::Runtime;

/// One runtime for the whole process. Several threads may block on it at once: whichever holds
/// its drivers runs the I/O and timers for all of them. Tasks spawned on it run only while some
/// thread is inside [`block_on`].
static RUNTIME: LazyLock<std::io::Result<Runtime>> = LazyLock::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
});

/// Runs a future to completion.
///
/// # Panics
///
/// When called from within an async context, such as a UI event loop's task.
pub fn block_on<T>(future: impl Future<Output = T>) -> anyhow::Result<T> {
    match &*RUNTIME {
        Ok(runtime) => Ok(runtime.block_on(future)),
        Err(error) => anyhow::bail!("cannot start the async runtime: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn another_thread_takes_the_drivers_over_when_the_first_is_done() {
        let (driving, started) = mpsc::channel();
        let other = std::thread::spawn(move || {
            started.recv().unwrap();
            block_on(async { tokio::time::sleep(Duration::from_millis(50)).await })
        });
        block_on(async {
            driving.send(()).unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        })
        .unwrap();
        other.join().unwrap().unwrap();
    }
}
