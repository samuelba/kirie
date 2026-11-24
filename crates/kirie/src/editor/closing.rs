//! Closing the editor, or opening another image in it: changes not yet saved or copied are
//! offered for saving first.

use iced::widget::{Space, column, container, opaque, row, text};
use iced::{Alignment, Background, Color, Element, Length, Task};
use kirie_ui::icon::Icon;
use kirie_ui::space;
use kirie_ui::widget::{floating, primary_button, secondary_button};

use super::{Editor, Export, Loaded, Message};

/// What the user chose when asked about unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    SaveImage,
    SaveProject,
    Discard,
    Cancel,
}

/// What comes once the question about unsaved changes is answered, or the changes are saved.
#[derive(Debug, Clone, Default)]
pub enum Then {
    /// Close the editor.
    #[default]
    Exit,
    /// Open this image in place of the one in the editor.
    Open(Box<Loaded>),
    /// Put the image away and show the empty editor.
    Clear,
}

/// Where closing the editor stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Closing {
    #[default]
    No,
    /// Asking whether to save the changes.
    Asking,
    /// Saving them, to close once saved.
    Saving,
}

impl Editor {
    /// Whether the image has changes that were neither saved nor copied.
    pub(super) fn unsaved(&self) -> bool {
        self.image.is_some() && self.document() != &self.kept
    }

    /// The window's close button: closes now, or asks first about unsaved changes.
    pub(super) fn close_requested(&mut self) -> Task<Message> {
        if self.closing == Closing::Saving {
            return Task::none();
        }
        self.then = Then::Exit;
        self.annotator.commit_text();
        if self.unsaved() {
            self.closing = Closing::Asking;
            Task::none()
        } else {
            iced::exit()
        }
    }

    /// Puts the image away, asking first about unsaved changes.
    pub(super) fn discard(&mut self) -> Task<Message> {
        self.annotator.commit_text();
        if self.closing != Closing::No || self.image.is_none() {
            return Task::none();
        }
        if self.unsaved() {
            self.then = Then::Clear;
            self.closing = Closing::Asking;
            Task::none()
        } else {
            self.clear();
            Task::none()
        }
    }

    /// Goes on with what was put off for the question about unsaved changes.
    pub(super) fn carry_on(&mut self) -> Task<Message> {
        self.closing = Closing::No;
        match std::mem::take(&mut self.then) {
            Then::Exit => iced::exit(),
            Then::Open(next) => {
                self.open(*next);
                Task::none()
            }
            Then::Clear => {
                self.clear();
                Task::none()
            }
        }
    }

    /// Opens `loaded` in place of the image, asking first about unsaved changes.
    pub(super) fn replace(&mut self, loaded: Loaded) -> Task<Message> {
        self.annotator.commit_text();
        if self.closing != Closing::No {
            return Task::none();
        }
        if self.unsaved() {
            self.then = Then::Open(Box::new(loaded));
            self.closing = Closing::Asking;
        } else {
            self.open(loaded);
        }
        Task::none()
    }

    pub(super) fn answer(&mut self, answer: Answer) -> Task<Message> {
        match answer {
            Answer::Cancel => {
                self.closing = Closing::No;
                self.then = Then::Exit;
                Task::none()
            }
            Answer::Discard => self.carry_on(),
            Answer::SaveImage | Answer::SaveProject => {
                self.closing = Closing::Saving;
                self.export(if answer == Answer::SaveImage {
                    Export::Save
                } else {
                    Export::Project
                })
            }
        }
    }

    /// The question about unsaved changes, over everything else, while it is asked.
    pub(super) fn closing_dialog(&self) -> Option<Element<'_, Message>> {
        let palette = self.palette;
        let choices: Element<'_, Message> = match self.closing {
            Closing::No => return None,
            Closing::Saving => text("Saving…").size(13).color(palette.muted).into(),
            Closing::Asking => row![
                secondary_button("Don't save", palette, Message::Close(Answer::Discard),),
                Space::new().width(Length::Fill),
                secondary_button("Cancel", palette, Message::Close(Answer::Cancel)),
                secondary_button("Save project", palette, Message::Close(Answer::SaveProject)),
                primary_button(
                    Icon::Save,
                    "Save image",
                    palette,
                    Message::Close(Answer::SaveImage)
                ),
            ]
            .spacing(space::S)
            .align_y(Alignment::Center)
            .into(),
        };
        let card = container(
            column![
                text(match self.then {
                    Then::Open(_) => "Save your changes before opening the other image?",
                    Then::Clear => "Save your changes before discarding the image?",
                    Then::Exit => "Save your changes?",
                })
                .size(18),
                text(
                    "The marks on this image are not saved yet. \
                     Save the image to Pictures, or a project to keep editing them later."
                )
                .size(13)
                .color(palette.muted),
                choices,
            ]
            .spacing(space::M),
        )
        .width(560)
        .padding(space::XL)
        .style(floating(palette));
        Some(opaque(container(card).center(Length::Fill).style(|_| {
            container::Style {
                background: Some(Background::Color(Color::BLACK.scale_alpha(0.4))),
                ..container::Style::default()
            }
        })))
    }
}

#[cfg(test)]
mod tests {
    use kirie_annotate::{Kind, Op, Paint};

    use super::*;
    use crate::editor::Loaded;

    fn editor() -> Editor {
        let image = ::image::RgbaImage::new(40, 30);
        Editor::boot(Some(Loaded::image(image, None))).0
    }

    fn mark(editor: &mut Editor) {
        editor.annotator.history.apply(Op::Add {
            kind: Kind::Rectangle {
                rect: kurbo::Rect::new(1.0, 1.0, 20.0, 20.0),
            },
            paint: Paint::default(),
        });
    }

    /// Toasts start a timer, which needs a runtime.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn only_changes_since_the_last_save_count() {
        let runtime = runtime();
        let _inside = runtime.enter();
        let mut editor = editor();
        assert!(!editor.unsaved());
        mark(&mut editor);
        assert!(editor.unsaved());
        let _ = editor.update(Message::Exported(
            "Saved".into(),
            None,
            Box::new(editor.document().clone()),
        ));
        assert!(!editor.unsaved());
        editor.annotator.history.undo();
        assert!(editor.unsaved(), "undoing a saved mark is a change too");
        assert!(
            !Editor::boot(None).0.unsaved(),
            "nothing open, nothing to lose"
        );
    }

    #[test]
    fn closing_with_changes_asks_and_can_be_called_off() {
        let mut editor = editor();
        mark(&mut editor);
        let _ = editor.update(Message::CloseRequested);
        assert_eq!(editor.closing, Closing::Asking);
        assert!(editor.closing_dialog().is_some());
        let _ = editor.update(Message::Escape);
        assert_eq!(editor.closing, Closing::No);
        let _ = editor.update(Message::CloseRequested);
        let _ = editor.update(Message::Close(Answer::Cancel));
        assert_eq!(editor.closing, Closing::No);
        assert!(editor.closing_dialog().is_none());
    }

    #[test]
    fn a_failed_save_keeps_the_editor_open() {
        let runtime = runtime();
        let _inside = runtime.enter();
        let mut editor = editor();
        mark(&mut editor);
        editor.closing = Closing::Saving;
        let _ = editor.update(Message::Done(Err("disk full".into())));
        assert_eq!(editor.closing, Closing::No);
        assert!(editor.unsaved());
    }

    #[test]
    fn opening_another_image_over_changes_asks_first() {
        let runtime = runtime();
        let _inside = runtime.enter();
        let mut editor = editor();
        mark(&mut editor);
        let other = || Loaded::image(::image::RgbaImage::new(8, 8), None);
        let _ = editor.update(Message::Opened(Ok(other())));
        assert_eq!(editor.closing, Closing::Asking);
        assert_eq!(editor.document().width, 40, "nothing replaced yet");
        let _ = editor.update(Message::Close(Answer::Cancel));
        assert_eq!(editor.document().width, 40);
        let _ = editor.update(Message::Opened(Ok(other())));
        let _ = editor.update(Message::Close(Answer::Discard));
        assert_eq!(editor.document().width, 8);
        assert_eq!(editor.closing, Closing::No);
        let _ = editor.update(Message::Opened(Ok(Loaded::image(
            ::image::RgbaImage::new(5, 5),
            None,
        ))));
        assert_eq!(editor.document().width, 5, "no changes, no question");
    }

    #[test]
    fn discarding_an_untouched_image_empties_the_editor_at_once() {
        let mut editor = editor();
        let _ = editor.update(Message::Discard);
        assert!(editor.image.is_none());
        assert_eq!(editor.closing, Closing::No);
        assert!(!editor.unsaved());
    }

    #[test]
    fn discarding_changes_asks_first() {
        let runtime = runtime();
        let _inside = runtime.enter();
        let mut editor = editor();
        mark(&mut editor);
        let _ = editor.update(Message::Discard);
        assert_eq!(editor.closing, Closing::Asking);
        assert!(editor.image.is_some(), "nothing discarded yet");
        let _ = editor.update(Message::Close(Answer::Cancel));
        assert!(editor.image.is_some());
        let _ = editor.update(Message::Discard);
        let _ = editor.update(Message::Close(Answer::Discard));
        assert!(editor.image.is_none());
        assert_eq!(editor.closing, Closing::No);
        assert_eq!(editor.document().shapes().len(), 0);
    }

    #[test]
    fn discarding_after_saving_empties_the_editor() {
        let runtime = runtime();
        let _inside = runtime.enter();
        let mut editor = editor();
        mark(&mut editor);
        let _ = editor.update(Message::Discard);
        editor.closing = Closing::Saving;
        let saved = Box::new(editor.document().clone());
        let _ = editor.update(Message::Exported("Saved".into(), None, saved));
        assert!(editor.image.is_none());
        assert_eq!(editor.closing, Closing::No);
    }

    #[test]
    fn a_save_on_its_way_ignores_more_closing() {
        let mut editor = editor();
        mark(&mut editor);
        editor.closing = Closing::Saving;
        let _ = editor.update(Message::CloseRequested);
        assert_eq!(editor.closing, Closing::Saving);
    }
}
