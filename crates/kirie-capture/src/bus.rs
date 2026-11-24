//! The session bus, with a limit on how long connecting and each method call may take.

use std::time::Duration;

/// The usual D-Bus reply timeout: a service that has not answered by then is stuck.
const REPLY: Duration = Duration::from_secs(25);

/// A new connection to the session bus. Connecting and each method call give up after
/// [`REPLY`].
pub async fn session() -> zbus::Result<zbus::Connection> {
    let connecting = zbus::connection::Builder::session()
        .method_timeout(REPLY)
        .build();
    tokio::time::timeout(REPLY, connecting).await.map_err(|_| {
        zbus::Error::InputOutput(std::io::Error::from(std::io::ErrorKind::TimedOut).into())
    })?
}
