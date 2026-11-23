//! One editor at a time: the running editor owns `dev.samuelba.Kirie` on the session bus, and
//! later `kirie edit` calls hand their file to it instead of opening a second window.

use std::path::{Path, PathBuf};

use iced::futures::channel::mpsc::{UnboundedSender, unbounded};
use iced::futures::{SinkExt as _, Stream, StreamExt as _};
use zbus::{connection, fdo, interface, proxy};

pub const BUS_NAME: &str = "dev.samuelba.Kirie";
pub const OBJECT_PATH: &str = "/dev/samuelba/Kirie";

/// What another Kirie asks the running editor to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Open(PathBuf),
    Present,
    /// Brings the editor window to the front with the about card open.
    About,
    /// A capture the editor hid for has ended without handing it anything.
    Resume,
}

#[derive(Debug)]
struct Service {
    requests: UnboundedSender<Request>,
}

impl Service {
    fn send(&self, request: Request) -> fdo::Result<()> {
        self.requests
            .unbounded_send(request)
            .map_err(|_| fdo::Error::Failed("Kirie is closing".into()))
    }
}

#[interface(name = "dev.samuelba.Kirie")]
impl Service {
    /// Opens an image or a `.kirie` project; the path must be absolute.
    fn open(&self, path: &str) -> fdo::Result<()> {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(fdo::Error::InvalidArgs("the path must be absolute".into()));
        }
        self.send(Request::Open(path))
    }

    /// Brings the editor window to the front.
    fn present(&self) -> fdo::Result<()> {
        self.send(Request::Present)
    }

    /// Brings the editor window to the front with the about card open.
    fn about(&self) -> fdo::Result<()> {
        self.send(Request::About)
    }

    /// Shows the editor window again if it hid for a capture.
    fn resume(&self) -> fdo::Result<()> {
        self.send(Request::Resume)
    }
}

#[proxy(
    interface = "dev.samuelba.Kirie",
    default_service = "dev.samuelba.Kirie",
    default_path = "/dev/samuelba/Kirie"
)]
trait Kirie {
    fn open(&self, path: &str) -> zbus::Result<()>;
    fn present(&self) -> zbus::Result<()>;
    fn about(&self) -> zbus::Result<()>;
    fn resume(&self) -> zbus::Result<()>;
}

/// Hands `path` (or nothing, to just raise the window) to a running editor. True when one took
/// it.
pub async fn hand_over(path: Option<&Path>) -> bool {
    let attempt = async {
        let connection = zbus::Connection::session().await?;
        let dbus = fdo::DBusProxy::new(&connection).await?;
        if !dbus.name_has_owner(BUS_NAME.try_into()?).await? {
            return Ok::<_, zbus::Error>(false);
        }
        let editor = KirieProxy::new(&connection).await?;
        match path {
            Some(path) => editor.open(&path.to_string_lossy()).await?,
            None => editor.present().await?,
        }
        Ok(true)
    };
    attempt
        .await
        .inspect_err(|error| tracing::debug!(%error, "no editor to hand over to"))
        .unwrap_or(false)
}

/// Opens the about card in a running editor. True when one did.
pub async fn about() -> bool {
    let attempt = async {
        let connection = zbus::Connection::session().await?;
        let dbus = fdo::DBusProxy::new(&connection).await?;
        if !dbus.name_has_owner(BUS_NAME.try_into()?).await? {
            return Ok::<_, zbus::Error>(false);
        }
        KirieProxy::new(&connection).await?.about().await?;
        Ok(true)
    };
    attempt
        .await
        .inspect_err(|error| tracing::debug!(%error, "no editor to show the about card"))
        .unwrap_or(false)
}

/// Tells a running editor that hid for a capture that the capture ended without an image.
pub async fn resume() {
    let attempt = async {
        let connection = zbus::Connection::session().await?;
        KirieProxy::new(&connection).await?.resume().await
    };
    if let Err(error) = attempt.await {
        tracing::debug!(%error, "no editor to resume");
    }
}

/// Owns the editor's bus name for as long as the stream is polled and yields what other Kirie
/// calls ask for.
pub fn requests() -> impl Stream<Item = Request> {
    iced::stream::channel(16, async |mut output| {
        let (sender, mut receiver) = unbounded();
        let served = connection::Builder::session()
            .name(BUS_NAME)
            .serve_at(OBJECT_PATH, Service { requests: sender })
            .build()
            .await;
        let _connection = match served {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "another editor owns {BUS_NAME}");
                iced::futures::future::pending::<()>().await;
                return;
            }
        };
        while let Some(request) = receiver.next().await {
            if output.send(request).await.is_err() {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_refused() {
        let (sender, mut receiver) = unbounded();
        let service = Service { requests: sender };
        assert!(service.open("relative.png").is_err());
        assert!(service.open("/abs/shot.png").is_ok());
        assert!(service.present().is_ok());
        assert!(service.about().is_ok());
        assert!(service.resume().is_ok());
        assert_eq!(
            receiver.try_recv().ok(),
            Some(Request::Open(PathBuf::from("/abs/shot.png")))
        );
        assert_eq!(receiver.try_recv().ok(), Some(Request::Present));
        assert_eq!(receiver.try_recv().ok(), Some(Request::About));
        assert_eq!(receiver.try_recv().ok(), Some(Request::Resume));
    }
}
