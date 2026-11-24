//! Kirie's tray icon on desktops that host `StatusNotifierItem`s: COSMIC's Notifications Tray,
//! KDE Plasma's System Tray, and GNOME with the `AppIndicator` extension. It runs in a process
//! of its own, `kirie tray`, which the editor and every capture start when none runs yet, and
//! which stays until Quit is picked or the session ends. It looks for a newer release once a day
//! and offers it in its menu, and starts itself anew from the updated binary once one is installed.

mod item;
mod menu;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use iced::futures::StreamExt;
use iced::futures::channel::mpsc::{self, UnboundedSender};
use semver::Version;
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
/// How often the tray looks whether an update replaced its binary, a newer release is due to be
/// looked for, or the one offered is no longer wanted.
const LOOK_EVERY: Duration = Duration::from_secs(60);

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
    /// A screenshot after this many seconds.
    CaptureIn(u32),
    Settings,
    About,
    /// Opens the about card, which installs the newer release.
    Update,
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
    let (found_sender, mut found) = mpsc::unbounded();
    let mut looks = tokio::time::interval(LOOK_EVERY);
    looks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_check = None;
    // The newest release found, offered while the user wants it.
    let mut newest: Option<Version> = None;
    let mut restart_failed = false;
    loop {
        tokio::select! {
            command = received.next() => {
                match command {
                    None | Some(Command::Quit) => return Ok(()),
                    Some(command) => obey(command, &token).await,
                }
            }
            _ = looks.tick() => {
                if !restart_failed && let Some(binary) = crate::desktop::launch::replaced_binary() {
                    restart_failed = !restart(&connection, binary).await;
                }
                let now = SystemTime::now();
                if crate::release::due(last_check, now) {
                    last_check = Some(now);
                    look_for_release(found_sender.clone());
                }
                offer(&connection, newest.clone().filter(crate::release::wanted)).await;
            }
            Some(update) = found.next() => {
                newest = update;
                offer(&connection, newest.clone().filter(crate::release::wanted)).await;
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
        Command::CaptureIn(seconds) => {
            tokio::time::sleep(MENU_CLOSES).await;
            start(&["capture", "--delay", &seconds.to_string()], token);
        }
        Command::Settings => start(&["settings"], token),
        Command::About | Command::Update => start(&["about"], token),
        Command::Quit => {}
    }
}

/// Runs Kirie with `args` in the background, with the activation `token` that lets its window
/// come to the front.
fn start(args: &[&str], token: Option<String>) {
    let exe = match crate::desktop::launch::exe() {
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
        .env_remove(crate::desktop::activation::ENV);
    if let Some(token) = token {
        command.env(crate::desktop::activation::ENV, token);
    }
    crate::desktop::launch::spawn(command);
}

/// Looks for a newer release in the background, when the user lets Kirie, and sends what it
/// found; a failed look sends nothing, so the menu stays as it is.
fn look_for_release(found: UnboundedSender<Option<Version>>) {
    if !crate::storage::settings::Settings::load().check_updates {
        return;
    }
    tokio::spawn(async move {
        match crate::release::check().await {
            Ok(release) => {
                let _ = found.unbounded_send(release.map(|release| release.version));
            }
            Err(error) => tracing::debug!(%error, "cannot check for a newer release"),
        }
    });
}

/// Shows `update` in the menu, or no release, and tells the panel when that changed it.
async fn offer(connection: &Connection, update: Option<Version>) {
    let Ok(menu) = connection
        .object_server()
        .interface::<_, menu::Menu>(menu::PATH)
        .await
    else {
        return;
    };
    let Some(revision) = menu.get_mut().await.offer(update) else {
        return;
    };
    if let Err(error) = menu::Menu::layout_updated(menu.signal_emitter(), revision, 0).await {
        tracing::debug!(%error, "cannot tell the panel that the menu changed");
    }
}

/// Becomes the tray of the updated `binary`. The icon's name is given up first, so that the new
/// tray finds it free instead of quitting as if another tray showed the icon. False when the
/// tray stays this one.
async fn restart(connection: &Connection, binary: std::path::PathBuf) -> bool {
    use std::os::unix::process::CommandExt as _;

    if !binary.is_file() {
        return false;
    }
    if let Err(error) = connection.release_name(BUS_NAME).await {
        tracing::warn!(%error, "cannot hand the tray icon to the updated Kirie");
        return false;
    }
    tracing::info!("Kirie was updated; starting the tray icon anew");
    let error = std::process::Command::new(binary).arg("tray").exec();
    tracing::warn!(%error, "cannot start the updated tray icon");
    if let Err(error) = connection.request_name(BUS_NAME).await {
        tracing::warn!(%error, "cannot take the tray icon's name back");
    }
    false
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
