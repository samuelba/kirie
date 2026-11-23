//! The card in the canvas's corner that tells about a newer release. It updates installs made by
//! `install.sh` in place and links other installs to the release page. The about card shows the
//! same release, and checks for one on request.

use iced::widget::{Space, column, container, opaque, row, text};
use iced::{Alignment, Element, Length, Task};
use kirie_ui::icon::Icon;
use kirie_ui::space;
use kirie_ui::widget::{floating, primary_button, secondary_button, small_icon_button};

use super::{Editor, Message};
use crate::release::{self, CURRENT_VERSION, InstallKind, Installed, Release};

const CARD_WIDTH: f32 = 340.0;
/// How often an open editor looks at the clock to tell whether a look for a release is due.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// What the corner card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Card {
    Hidden,
    Available,
    Updating,
    /// The new binary is in place; new screenshots run it, this editor once opened again.
    Installed(Installed),
    Failed(String),
}

/// Looking for newer releases, and what was found.
#[derive(Debug)]
pub struct Updates {
    /// Kirie looks on its own, at start and once a day.
    check: bool,
    /// When Kirie last looked for a release.
    last_check: Option<std::time::SystemTime>,
    /// The newer release found last.
    release: Option<Release>,
    card: Card,
    lookup: Lookup,
    install: InstallKind,
}

impl Updates {
    pub fn new() -> Self {
        Self {
            check: crate::settings::Settings::load().check_updates,
            last_check: None,
            release: None,
            card: Card::Hidden,
            lookup: Lookup::Idle,
            install: release::install_kind(),
        }
    }

    pub fn check(&self) -> bool {
        self.check
    }
}

/// How the last look for a release went, for the about card.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Lookup {
    Idle,
    Checking,
    UpToDate,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Look for a newer release if the last look was a day ago.
    Tick,
    /// Look for a newer release; `manual` when the user asked, so a failure is told.
    Check {
        manual: bool,
    },
    Checked {
        manual: bool,
        result: Result<Option<Release>, String>,
    },
    /// Update a script install; open the release page for other installs.
    Update,
    Updated(Result<Installed, String>),
    /// The release page, with its notes.
    OpenRelease,
    Skip,
    /// Hides the card until the next check.
    Dismiss,
}

impl Editor {
    pub(super) fn release_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Tick => {
                if release::due(self.updates.last_check, std::time::SystemTime::now()) {
                    return self.check_release(false);
                }
            }
            Event::Check { manual } => return self.check_release(manual),
            Event::Checked { manual, result } => self.release_checked(manual, result),
            Event::Update => return self.start_update(),
            Event::Updated(Ok(installed)) => {
                tracing::info!(?installed, "installed the new release");
                if let (InstallKind::Script { installed, .. }, Some(release)) =
                    (&mut self.updates.install, &self.updates.release)
                {
                    *installed = Some(release.version.clone());
                }
                self.updates.card = Card::Installed(installed);
            }
            Event::Updated(Err(error)) => {
                tracing::warn!(%error, "the update failed");
                self.updates.card = Card::Failed(error);
            }
            Event::OpenRelease => {
                if let Some(release) = &self.updates.release {
                    crate::launch::open(&release.url);
                }
            }
            Event::Skip => {
                if let Some(release) = &self.updates.release {
                    release::skip(&release.version);
                }
                self.updates.card = Card::Hidden;
            }
            Event::Dismiss => self.updates.card = Card::Hidden,
        }
        Task::none()
    }

    /// Starts looking for a newer release, unless one is being looked for already.
    pub(super) fn check_release(&mut self, manual: bool) -> Task<Message> {
        if self.updates.lookup == Lookup::Checking {
            return Task::none();
        }
        self.updates.lookup = Lookup::Checking;
        self.updates.last_check = Some(std::time::SystemTime::now());
        Task::perform(release::check(), move |result| {
            Message::Release(Event::Checked { manual, result })
        })
    }

    /// Lets Kirie look for releases on its own, or not; turned on, it looks at once unless it
    /// found one already.
    pub(super) fn set_release_checks(&mut self, on: bool) -> Task<Message> {
        let turned_on = on && !self.updates.check;
        self.updates.check = on;
        if turned_on && self.updates.release.is_none() {
            self.check_release(false)
        } else {
            Task::none()
        }
    }

    /// Looks for a release as the about card opens, when the user lets Kirie look on its own
    /// and no look has told anything yet.
    pub(super) fn check_on_about(&mut self) -> Task<Message> {
        if self.updates.check
            && self.updates.lookup == Lookup::Idle
            && self.updates.release.is_none()
        {
            self.check_release(false)
        } else {
            Task::none()
        }
    }

    fn release_checked(&mut self, manual: bool, result: Result<Option<Release>, String>) {
        let found = match result {
            Ok(Some(found)) => found,
            Ok(None) => {
                tracing::debug!("this is the latest release");
                self.updates.lookup = Lookup::UpToDate;
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "cannot check for a newer release");
                self.updates.lookup = if manual {
                    Lookup::Failed(error)
                } else {
                    Lookup::Idle
                };
                return;
            }
        };
        tracing::info!(version = %found.version, "a newer release is available");
        self.updates.lookup = Lookup::Idle;
        let skipped = release::is_skipped(&found.version);
        self.updates.card =
            card_after_check(&self.updates.card, &self.updates.install, &found, skipped);
        self.updates.release = Some(found);
    }

    fn start_update(&mut self) -> Task<Message> {
        let Some(release) = &self.updates.release else {
            return Task::none();
        };
        if !matches!(self.updates.install, InstallKind::Script { .. }) {
            crate::launch::open(&release.url);
            return Task::none();
        }
        if matches!(self.updates.card, Card::Updating | Card::Installed(_)) {
            return Task::none();
        }
        self.updates.card = Card::Updating;
        Task::perform(release::install(release.version.clone()), |result| {
            Message::Release(Event::Updated(result))
        })
    }

    /// What the about card says about updates under the version, with its button.
    pub(super) fn release_status(&self) -> (Option<String>, Option<(String, Event)>) {
        if let Some(release) = &self.updates.release {
            let version = &release.version;
            return match (&self.updates.card, &self.updates.install) {
                (Card::Updating, _) => (Some(format!("Updating to {version}…")), None),
                (Card::Installed(_), _) => (
                    None,
                    Some((
                        format!("Version {version} is installed"),
                        Event::OpenRelease,
                    )),
                ),
                (_, InstallKind::Script { .. }) => (
                    None,
                    Some((format!("Update to version {version}"), Event::Update)),
                ),
                (_, InstallKind::Other) => (
                    None,
                    Some((format!("Download version {version}"), Event::Update)),
                ),
            };
        }
        let check = || {
            (
                "Check for updates".to_owned(),
                Event::Check { manual: true },
            )
        };
        match &self.updates.lookup {
            Lookup::Idle => (None, Some(check())),
            Lookup::Checking => (Some("Checking for updates…".to_owned()), None),
            Lookup::UpToDate => (Some("Kirie is up to date".to_owned()), None),
            Lookup::Failed(error) => (
                Some(format!("Cannot check for updates: {error}")),
                Some(check()),
            ),
        }
    }

    /// The card in the canvas's bottom-right corner, while it has something to tell.
    pub(super) fn release_card(&self) -> Option<Element<'_, Message>> {
        let release = self.updates.release.as_ref()?;
        let palette = self.palette;
        let notes = || {
            secondary_button(
                "Release notes",
                palette,
                Message::Release(Event::OpenRelease),
            )
        };
        let version = &release.version;
        let say = |words: String| text(words).size(13).color(palette.muted);
        let (title, body, buttons): (String, Element<'_, Message>, Option<Element<'_, Message>>) =
            match &self.updates.card {
                Card::Hidden => return None,
                Card::Available => {
                    let (icon, label) = match self.updates.install {
                        InstallKind::Script { .. } => (Icon::Done, "Update"),
                        InstallKind::Other => (Icon::Open, "Download"),
                    };
                    let main =
                        primary_button(icon, label, palette, Message::Release(Event::Update));
                    let skip = secondary_button(
                        "Skip this version",
                        palette,
                        Message::Release(Event::Skip),
                    );
                    (
                        format!("Kirie {version} is available"),
                        say(format!("You have {CURRENT_VERSION}.")).into(),
                        Some(row![main, notes(), skip].spacing(space::S).into()),
                    )
                }
                Card::Updating => (
                    format!("Updating to {version}…"),
                    say("Downloading and installing it. You can keep working.".to_owned()).into(),
                    None,
                ),
                Card::Installed(installed) => {
                    let mut body = column![say(
                        "New screenshots use it already. Open the editor again to use it here."
                            .to_owned()
                    )]
                    .spacing(space::XS);
                    if installed.extension_changed {
                        body = body.push(say(
                            "Log out and back in so GNOME Shell loads the new extension."
                                .to_owned(),
                        ));
                    }
                    (
                        format!("Kirie {version} is installed"),
                        body.into(),
                        Some(notes()),
                    )
                }
                Card::Failed(error) => (
                    "The update failed".to_owned(),
                    text(error.clone()).size(13).color(palette.danger).into(),
                    Some(notes()),
                ),
            };
        let mut header =
            row![text(title).size(15), Space::new().width(Length::Fill)].align_y(Alignment::Center);
        if self.updates.card != Card::Updating {
            header = header.push(small_icon_button(
                Icon::Close,
                "Dismiss",
                palette,
                Some(Message::Release(Event::Dismiss)),
            ));
        }
        let card = container(column![header, body, buttons].spacing(space::S))
            .width(CARD_WIDTH)
            .padding(space::L)
            .style(floating(palette));
        Some(
            container(opaque(card))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::End)
                .padding(space::L)
                .into(),
        )
    }
}

/// What the card shows once a check found `found`, newer than the running build.
fn card_after_check(current: &Card, install: &InstallKind, found: &Release, skipped: bool) -> Card {
    if matches!(current, Card::Updating | Card::Installed(_)) {
        return current.clone();
    }
    // The script put this version in place already, by hand or in another Kirie.
    if let InstallKind::Script {
        installed: Some(installed),
    } = install
        && *installed >= found.version
    {
        return Card::Installed(Installed {
            extension_changed: false,
        });
    }
    if skipped {
        Card::Hidden
    } else {
        Card::Available
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::*;

    fn found(version: Version) -> Release {
        Release {
            version,
            url: format!("{}/releases/tag/v9.9.9", env!("CARGO_PKG_REPOSITORY")),
        }
    }

    fn editor_with(install: InstallKind) -> Editor {
        let mut editor = Editor::boot(None).0;
        editor.updates.install = install;
        editor
    }

    fn checked(editor: &mut Editor, manual: bool, result: Result<Option<Release>, String>) {
        let _ = editor.update(Message::Release(Event::Checked { manual, result }));
    }

    #[test]
    fn a_found_release_shows_the_card_unless_skipped_or_in_place() {
        let script = InstallKind::Script { installed: None };
        let v9 = found(Version::new(9, 9, 9));
        assert_eq!(
            card_after_check(&Card::Hidden, &script, &v9, false),
            Card::Available
        );
        assert_eq!(
            card_after_check(&Card::Hidden, &script, &v9, true),
            Card::Hidden
        );
        assert_eq!(
            card_after_check(&Card::Updating, &script, &v9, false),
            Card::Updating
        );
        let in_place = InstallKind::Script {
            installed: Some(Version::new(9, 9, 9)),
        };
        assert!(matches!(
            card_after_check(&Card::Hidden, &in_place, &v9, false),
            Card::Installed(_)
        ));
        assert_eq!(
            card_after_check(&Card::Failed("x".into()), &InstallKind::Other, &v9, false),
            Card::Available
        );
    }

    #[test]
    fn the_card_goes_from_available_to_installed() {
        let mut editor = editor_with(InstallKind::Script { installed: None });
        assert!(editor.release_card().is_none());
        checked(&mut editor, false, Ok(Some(found(Version::new(9, 9, 9)))));
        assert_eq!(editor.updates.card, Card::Available);
        assert!(editor.release_card().is_some());
        let _ = editor.update(Message::Release(Event::Update));
        assert_eq!(editor.updates.card, Card::Updating);
        assert_eq!(editor.release_status().1, None);
        let installed = Installed {
            extension_changed: true,
        };
        let _ = editor.update(Message::Release(Event::Updated(Ok(installed))));
        assert_eq!(editor.updates.card, Card::Installed(installed));
        assert_eq!(
            editor.updates.install,
            InstallKind::Script {
                installed: Some(Version::new(9, 9, 9))
            }
        );
        let _ = editor.update(Message::Release(Event::Dismiss));
        assert!(editor.release_card().is_none());
    }

    #[test]
    fn the_hourly_tick_looks_only_once_a_day_is_up() {
        let mut editor = editor_with(InstallKind::Other);
        editor.updates.last_check = Some(std::time::SystemTime::now());
        let _ = editor.update(Message::Release(Event::Tick));
        assert_eq!(editor.updates.lookup, Lookup::Idle);
        editor.updates.last_check = Some(std::time::SystemTime::now() - release::CHECK_EVERY);
        let _ = editor.update(Message::Release(Event::Tick));
        assert_eq!(editor.updates.lookup, Lookup::Checking);
    }

    #[test]
    fn a_failed_update_says_why() {
        let mut editor = editor_with(InstallKind::Script { installed: None });
        checked(&mut editor, false, Ok(Some(found(Version::new(9, 9, 9)))));
        let _ = editor.update(Message::Release(Event::Update));
        let _ = editor.update(Message::Release(Event::Updated(Err("no network".into()))));
        assert_eq!(editor.updates.card, Card::Failed("no network".into()));
    }

    #[test]
    fn other_installs_are_not_updated_in_place() {
        let mut editor = editor_with(InstallKind::Other);
        checked(&mut editor, false, Ok(Some(found(Version::new(9, 9, 9)))));
        let (_, action) = editor.release_status();
        assert_eq!(action.unwrap().0, "Download version 9.9.9");
    }

    #[test]
    fn the_about_card_tells_how_the_check_went() {
        let mut editor = editor_with(InstallKind::Other);
        assert_eq!(editor.release_status().1.unwrap().0, "Check for updates");
        checked(&mut editor, false, Err("offline".into()));
        assert_eq!(
            editor.updates.lookup,
            Lookup::Idle,
            "a check of its own fails quietly"
        );
        checked(&mut editor, true, Err("offline".into()));
        assert!(editor.release_status().0.unwrap().contains("offline"));
        checked(&mut editor, true, Ok(None));
        assert_eq!(
            editor.release_status(),
            (Some("Kirie is up to date".to_owned()), None)
        );
        assert_eq!(editor.updates.card, Card::Hidden);
    }
}
