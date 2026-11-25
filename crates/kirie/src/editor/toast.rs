//! Short notes at the foot of the editor: what was saved or copied, and what went wrong.

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{container, row, text};
use iced::{Alignment, Element, Padding, Task};
use kirie_ui::widget::{floating, secondary_button};
use kirie_ui::{Palette, space};

use super::{Editor, Message, OutsideEvent};

/// How long a toast stays.
const SHOWN: Duration = Duration::from_secs(4);
/// How long a toast with a button stays: a little longer, to reach the button.
const SHOWN_WITH_FILE: Duration = Duration::from_secs(8);

#[derive(Debug)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
    /// The file just saved, to show in the file manager.
    file: Option<PathBuf>,
}

impl Toast {
    fn color(&self, palette: Palette) -> iced::Color {
        if self.error {
            palette.danger
        } else {
            palette.text
        }
    }
}

/// The toast shown, if any; a new one takes the place of the last.
#[derive(Debug, Default)]
pub(super) struct Toasts {
    shown: Option<Toast>,
    /// How many toasts were shown, which numbers the next.
    count: u64,
}

impl Toasts {
    /// Shows `text` in place of the toast shown; returns its id and how long it stays.
    fn show(&mut self, text: String, error: bool, file: Option<PathBuf>) -> (u64, Duration) {
        self.count += 1;
        let stays = if file.is_some() {
            SHOWN_WITH_FILE
        } else {
            SHOWN
        };
        self.shown = Some(Toast {
            id: self.count,
            text,
            error,
            file,
        });
        (self.count, stays)
    }

    /// Takes away toast `id`, unless another took its place already.
    pub(super) fn expire(&mut self, id: u64) {
        if self.shown.as_ref().is_some_and(|toast| toast.id == id) {
            self.shown = None;
        }
    }

    /// The toast shown, if any, with a button to show its file.
    pub(super) fn view(&self, palette: Palette) -> Option<Element<'_, Message>> {
        let toast = self.shown.as_ref()?;
        let mut content = row![
            text(toast.text.clone())
                .size(13)
                .color(toast.color(palette))
        ]
        .spacing(space::M)
        .align_y(Alignment::Center);
        let mut padding = Padding::from([space::S, space::L]);
        if let Some(file) = &toast.file {
            content = content.push(secondary_button(
                "Show in Files",
                palette,
                Message::Outside(OutsideEvent::ShowInFiles(file.clone())),
            ));
            padding = Padding::from(space::S).left(space::L);
        }
        Some(
            container(content)
                .padding(padding)
                .style(floating(palette))
                .into(),
        )
    }
}

impl Editor {
    pub(super) fn toast(&mut self, result: Result<String, String>) -> Task<Message> {
        match result {
            Ok(text) => self.show_toast(text, false, None),
            Err(text) => self.show_toast(text, true, None),
        }
    }

    /// Tells that the image was saved to `file`, or copied without one.
    pub(super) fn toast_file(&mut self, text: String, file: Option<PathBuf>) -> Task<Message> {
        self.show_toast(text, false, file)
    }

    fn show_toast(&mut self, text: String, error: bool, file: Option<PathBuf>) -> Task<Message> {
        let (id, stays) = self.toasts.show(text, error, file);
        Task::perform(tokio::time::sleep(stays), move |()| {
            Message::ToastExpired(id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_toast_outlives_the_timer_of_the_last() {
        let mut toasts = Toasts::default();
        let (first, stays) = toasts.show("Copied".into(), false, None);
        assert_eq!(stays, SHOWN);
        let (second, stays) = toasts.show("Saved".into(), false, Some("a.png".into()));
        assert_eq!(stays, SHOWN_WITH_FILE);
        assert_eq!((first, second), (1, 2));
        toasts.expire(first);
        assert_eq!(
            toasts.shown.as_ref().map(|toast| toast.text.as_str()),
            Some("Saved")
        );
        toasts.expire(second);
        assert!(toasts.shown.is_none());
        assert!(toasts.view(Palette::DARK).is_none());
    }

    #[test]
    fn errors_show_in_the_danger_color() {
        let mut toasts = Toasts::default();
        toasts.show("disk full".into(), true, None);
        let shown = |toasts: &Toasts| toasts.shown.as_ref().unwrap().color(Palette::DARK);
        assert_eq!(shown(&toasts), Palette::DARK.danger);
        toasts.show("Copied".into(), false, None);
        assert_eq!(shown(&toasts), Palette::DARK.text);
        assert!(toasts.view(Palette::DARK).is_some());
    }
}
