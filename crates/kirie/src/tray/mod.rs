//! Kirie's tray icon on desktops that host `StatusNotifierItem`s: COSMIC's Notifications Tray,
//! KDE Plasma's System Tray, and GNOME with the `AppIndicator` extension. It runs in a process
//! of its own, `kirie tray`, which the editor and every capture start when none runs yet, and
//! which stays until Quit is picked or the session ends.

mod item;
mod menu;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::StreamExt;
use iced::futures::channel::mpsc;
use zbus::fdo::{self, RequestNameFlags, RequestNameReply};
use zbus::{Connection, proxy};

/// The name the running tray owns, so that only one runs.
pub const BUS_NAME: &str = "dev.samuelba.Kirie.Tray";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
/// Time for the menu to close before a screenshot is taken.
const MENU_CLOSES: Duration = Duration::from_millis(300);
/// Time for the panel's activation token to arrive: it is sent just before the click, on its
/// own call.
const TOKEN_ARRIVES: Duration = Duration::from_millis(30);

/// The activation token the panel gave for the click being handled.
pub type Token = Arc<Mutex<Option<String>>>;

/// The token, which may be used once.
fn take(token: &Token) -> Option<String> {
    token.lock().ok().and_then(|mut slot| slot.take())
}

/// What the tray icon asks Kirie to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    Capture,
    About,
    Quit,
}

#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait Watcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

/// Shows the tray icon until Quit is picked; returns at once when another Kirie shows it.
pub fn run() -> anyhow::Result<()> {
    crate::block_on(serve())?
}

async fn serve() -> anyhow::Result<()> {
    let (sender, mut received) = mpsc::unbounded();
    let token = Token::default();
    let connection = zbus::connection::Builder::session()
        .serve_at(item::PATH, item::Item::new(sender.clone(), token.clone()))
        .serve_at(menu::PATH, menu::Menu::new(sender))
        .build()
        .await?;
    let owned = connection
        .request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)
        .await;
    if !matches!(
        owned,
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner)
    ) {
        tracing::debug!("the tray icon is shown already");
        return Ok(());
    }
    let bus = fdo::DBusProxy::new(&connection).await?;
    let mut watchers = bus
        .receive_name_owner_changed_with_args(&[(0, WATCHER)])
        .await?;
    register(&connection).await;
    loop {
        tokio::select! {
            command = received.next() => {
                match command {
                    None | Some(Command::Quit) => return Ok(()),
                    Some(command) => obey(command, &token).await,
                }
            }
            changed = watchers.next() => {
                let Some(changed) = changed else { return Ok(()) };
                if changed.args().is_ok_and(|args| args.new_owner().is_some()) {
                    register(&connection).await;
                }
            }
        }
    }
}

async fn obey(command: Command, token: &Token) {
    tokio::time::sleep(TOKEN_ARRIVES).await;
    let token = take(token);
    match command {
        Command::Open => start(&[], token),
        Command::Capture => {
            tokio::time::sleep(MENU_CLOSES).await;
            start(&["capture"], token);
        }
        Command::About => start(&["about"], token),
        Command::Quit => {}
    }
}

/// Runs Kirie with `args` in the background, with the activation `token` that lets its window
/// come to the front.
fn start(args: &[&str], token: Option<String>) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            tracing::warn!(%error, "cannot find Kirie to start it");
            return;
        }
    };
    let mut command = std::process::Command::new(exe);
    // Only a Kirie the GNOME Shell extension starts itself is trusted by it.
    command
        .args(args)
        .env_remove(kirie_capture::shell::LAUNCHED_ENV)
        .env_remove(crate::activation::ENV);
    if let Some(token) = token {
        command.env(crate::activation::ENV, token);
    }
    crate::launch::spawn(command);
}

/// Hands the icon to the panel's watcher, if one runs; a panel started later announces itself.
async fn register(connection: &Connection) {
    let Some(name) = connection.unique_name() else {
        return;
    };
    let result = async {
        WatcherProxy::new(connection)
            .await?
            .register_status_notifier_item(name.as_str())
            .await
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(%error, "no tray to show the icon in yet");
    }
}

/// Starts the tray icon in the background, unless one is shown already.
pub async fn ensure() {
    let shown = async {
        let connection = Connection::session().await?;
        fdo::DBusProxy::new(&connection)
            .await?
            .name_has_owner(BUS_NAME.try_into()?)
            .await
    }
    .await;
    match shown {
        Ok(false) => start(&["tray"], None),
        Ok(true) => {}
        Err(error) => tracing::debug!(%error, "cannot tell whether the tray icon is shown"),
    }
}
