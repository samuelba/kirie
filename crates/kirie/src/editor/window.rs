//! The editor's window: taking a screenshot from it, bringing it to the front, and its keyboard.

use std::time::Duration;

use iced::{Event as IcedEvent, Size, Task, event, window};

use super::{Editor, Message, files};

/// What happens to the editor's window.
#[derive(Debug, Clone)]
pub enum Event {
    Ipc(crate::desktop::ipc::Request),
    /// Hides the window and opens the overlay; the capture comes back over D-Bus.
    Capture,
    /// The window's size and state, taken just before it closes for a capture.
    Hide(Placement),
    /// Window `id`, placed so, is to be opened anew to come to the front.
    Remap(window::Id, Placement),
    CaptureEnded(Result<(), String>),
    /// The window lost the keyboard.
    Unfocused,
    /// The window got the keyboard back, perhaps after the overlay ran in a process of its own.
    Focused,
}

/// How big the window is, to open it again the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    size: Size,
    maximized: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            size: Size::new(1280.0, 840.0),
            maximized: false,
        }
    }
}

impl Editor {
    pub(super) fn window_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Ipc(crate::desktop::ipc::Request::Open(path, token)) => self
                .raise(token)
                .chain(Task::perform(files::load_async(path), Message::Opened)),
            Event::Ipc(crate::desktop::ipc::Request::Present(token)) => self.raise(token),
            Event::Ipc(crate::desktop::ipc::Request::About(token)) => {
                self.about = true;
                Task::batch([self.raise(token), self.check_on_about()])
            }
            Event::Ipc(crate::desktop::ipc::Request::Settings(token)) => {
                let _ = self.edit_settings(super::settings::Event::Open);
                self.raise(token)
            }
            Event::Ipc(crate::desktop::ipc::Request::Resume) if self.window.is_none() => {
                self.raise(None)
            }
            Event::Capture => self.window.map_or_else(Task::none, |id| {
                placement(id).map(|value| Message::Window(Event::Hide(value)))
            }),
            Event::Remap(id, placement) if self.window == Some(id) => {
                self.placement = placement;
                self.reopen(Some(id), None)
            }
            Event::Hide(placement) => {
                let Some(id) = self.window.take() else {
                    return Task::none();
                };
                self.placement = placement;
                window::close(id).chain(Task::perform(capture(), |value| {
                    Message::Window(Event::CaptureEnded(value))
                }))
            }
            Event::CaptureEnded(Err(error)) => self.raise(None).chain(self.toast(Err(error))),
            Event::Focused => {
                self.reload_shared();
                Task::none()
            }
            // Made anew before anything can be dragged from another window onto this one.
            // Once per window: iced's clipboard makes a device of its own when a window opens.
            Event::Unfocused => {
                if self.drops.renewed_for != self.window
                    && let Some(device) = &self.drops.device
                {
                    device.renew();
                    self.drops.renewed_for = self.window;
                }
                Task::none()
            }
            // The overlay asks for the window back when it is done, through `Resume` or `Open`.
            Event::Ipc(crate::desktop::ipc::Request::Resume)
            | Event::Remap(..)
            | Event::CaptureEnded(Ok(())) => Task::none(),
        }
    }

    /// Brings the window to the front. On Wayland a window cannot raise itself: it takes an
    /// activation `token` from whatever the user clicked. Without one, the window is opened
    /// anew, which most compositors put in front.
    fn raise(&mut self, token: Option<String>) -> Task<Message> {
        let remap =
            |id| placement(id).map(move |placement| Message::Window(Event::Remap(id, placement)));
        match (self.window, token) {
            (Some(id), Some(token)) => {
                activate(id, token).then(move |done| if done { Task::none() } else { remap(id) })
            }
            (Some(id), None) => remap(id),
            (None, token) => self.reopen(None, token),
        }
    }

    /// Opens the window as it was placed, closing `old` first, and brings it to the front with
    /// `token` if given.
    fn reopen(&mut self, old: Option<window::Id>, token: Option<String>) -> Task<Message> {
        let closed = old.map_or_else(Task::none, window::close);
        let (id, opened) = window::open(settings(self.placement));
        self.window = Some(id);
        let opened = match token {
            Some(token) => opened.then(move |id| activate(id, token.clone())).discard(),
            None => opened.discard(),
        };
        closed.chain(opened)
    }
}

/// Brings window `id` to the front with an activation `token`; true when the compositor was
/// asked.
pub(super) fn activate(id: window::Id, token: String) -> Task<bool> {
    window::run(id, move |window| {
        crate::desktop::activation::activate(window, &token)
    })
}

/// The size and state of window `id`.
fn placement(id: window::Id) -> Task<Placement> {
    window::is_maximized(id)
        .then(move |maximized| window::size(id).map(move |size| Placement { size, maximized }))
}

pub(super) fn settings(placement: Placement) -> window::Settings {
    window::Settings {
        size: placement.size,
        maximized: placement.maximized,
        min_size: Some(Size::new(760.0, 520.0)),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: crate::APP_ID.to_owned(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    }
}

/// Runs the overlay in its own process, after the window has had time to hide.
async fn capture() -> Result<(), String> {
    tokio::time::sleep(Duration::from_millis(350)).await;
    let exe = crate::desktop::launch::exe().map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        let mut child = std::process::Command::new(exe)
            .args(["capture", "--edit"])
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot take a screenshot: {error}"))?;
        let said = child.stderr.take().map(pass_on);
        let status = child
            .wait()
            .map_err(|error| format!("cannot take a screenshot: {error}"))?;
        if status.success() {
            return Ok(());
        }
        Err(said
            .and_then(|said| last_error(&said))
            .unwrap_or_else(|| "the screenshot failed".to_owned()))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Passes what `stderr` says on to the editor's own, line by line, as they come.
fn pass_on(stderr: impl std::io::Read + Send + 'static) -> std::sync::mpsc::Receiver<String> {
    use std::io::BufRead as _;
    let (sender, said) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            eprintln!("{line}");
            let _ = sender.send(line);
        }
    });
    said
}

/// The error the capture ended with. Programs it started may keep its stderr open, so this
/// stops once it has been quiet for a moment rather than at its end.
fn last_error(said: &std::sync::mpsc::Receiver<String>) -> Option<String> {
    std::iter::from_fn(|| said.recv_timeout(Duration::from_millis(200)).ok())
        .filter_map(|line| line.strip_prefix("Error: ").map(str::to_owned))
        .last()
}

/// The window losing the keyboard, when the data device is made anew to take drops.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn focus(event: IcedEvent, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        IcedEvent::Window(window::Event::Unfocused) => Some(Message::Window(Event::Unfocused)),
        IcedEvent::Window(window::Event::Focused) => Some(Message::Window(Event::Focused)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_capture_tells_its_error() {
        let said = pass_on(std::io::Cursor::new(
            "MESA: warning\nError: the clipboard cannot be reached here\n",
        ));
        assert_eq!(
            last_error(&said).as_deref(),
            Some("the clipboard cannot be reached here")
        );
        assert_eq!(
            last_error(&pass_on(std::io::Cursor::new("MESA: warning\n"))),
            None
        );
    }

    #[test]
    fn getting_the_keyboard_back_reads_what_the_overlay_saved() {
        let id = window::Id::unique();
        let focused = focus(
            IcedEvent::Window(window::Event::Focused),
            event::Status::Ignored,
            id,
        );
        assert!(matches!(focused, Some(Message::Window(Event::Focused))));
    }

    #[test]
    fn the_window_opens_as_it_was_placed() {
        let first = Placement::default();
        assert_eq!(first.size, Size::new(1280.0, 840.0));
        assert!(!first.maximized);
        let placement = Placement {
            size: Size::new(900.0, 700.0),
            maximized: true,
        };
        let opened = settings(placement);
        assert_eq!(opened.size, placement.size);
        assert!(opened.maximized);
        assert_eq!(opened.min_size, Some(Size::new(760.0, 520.0)));
        assert_eq!(
            opened.platform_specific.application_id,
            crate::APP_ID.to_owned()
        );
        assert!(
            !opened.exit_on_close_request,
            "the editor asks about unsaved changes first"
        );
    }
}
