//! The settings card: where screenshots are saved, how they are named, and whether saving
//! copies them too. Every change is kept at once.

use std::path::PathBuf;

use iced::widget::{Space, column, container, opaque, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{floating, icon_button, primary_button, secondary_button};
use kirie_ui::{Palette, radius, space};

use super::{Editor, Message};
use crate::settings::{self, Settings};

#[derive(Debug, Clone)]
pub enum Event {
    Open,
    Close,
    /// The file name pattern, as typed.
    Name(String),
    ChooseFolder,
    /// The folder picked in the dialog; `None` when it was canceled.
    Folder(Result<Option<PathBuf>, String>),
    /// Saves into `~/Pictures/Screenshots` again.
    ResetFolder,
    ToggleCopy,
}

/// The card while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    kept: Settings,
    /// The name pattern as typed, kept only once it makes a file name.
    name: String,
}

impl Form {
    fn new(kept: Settings) -> Self {
        Self {
            name: kept.name.clone(),
            kept,
        }
    }

    /// The name a screenshot taken now gets, or why the typed pattern makes none.
    fn example(&self) -> Result<String, String> {
        settings::name(&self.name, &jiff::Zoned::now()).map(|name| format!("{name}.png"))
    }
}

impl Editor {
    pub(super) fn edit_settings(&mut self, event: Event) -> Task<Message> {
        if let Event::Open = event {
            self.settings = Some(Form::new(Settings::load()));
            return Task::none();
        }
        let Some(form) = &mut self.settings else {
            return Task::none();
        };
        let before = form.kept.clone();
        match event {
            Event::Open | Event::Folder(Ok(None)) => {}
            Event::Close => {
                self.settings = None;
                return Task::none();
            }
            Event::Name(name) => {
                if settings::name(&name, &jiff::Zoned::now()).is_ok() {
                    form.kept.name.clone_from(&name);
                }
                form.name = name;
            }
            Event::ChooseFolder => {
                let folder = form.kept.folder();
                return Task::perform(
                    async move { crate::dialog::choose_folder(&folder).await },
                    |picked| Message::Settings(Event::Folder(picked)),
                );
            }
            Event::Folder(Ok(Some(folder))) => form.kept.folder = Some(folder),
            Event::Folder(Err(error)) => return self.toast(Err(error)),
            Event::ResetFolder => form.kept.folder = None,
            Event::ToggleCopy => form.kept.copy_on_save = !form.kept.copy_on_save,
        }
        if form.kept == before {
            return Task::none();
        }
        match form.kept.save() {
            Ok(()) => Task::none(),
            Err(error) => self.toast(Err(error)),
        }
    }

    /// The settings card, over everything else, while it is open.
    pub(super) fn settings_dialog(&self) -> Option<Element<'_, Message>> {
        let form = self.settings.as_ref()?;
        let palette = self.palette;
        let event = |event| Message::Settings(event);
        let folder = form.kept.folder();
        let mut place = row![
            text(settings::shown(&folder)).size(14).width(Length::Fill),
            secondary_button("Choose…", palette, event(Event::ChooseFolder)),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        if form.kept.folder.is_some() {
            place = place.push(secondary_button(
                "Use Pictures",
                palette,
                event(Event::ResetFolder),
            ));
        }
        let name = text_input(settings::DEFAULT_NAME, &form.name)
            .on_input(|name| Message::Settings(Event::Name(name)))
            .size(14)
            .padding([space::XS, space::S])
            .style(move |_, _| text_input::Style {
                background: Background::Color(palette.surface[1]),
                border: Border {
                    color: palette.border,
                    width: 1.0,
                    radius: radius::CONTROL.into(),
                },
                icon: palette.muted,
                placeholder: palette.muted,
                value: palette.text,
                selection: palette.accent.scale_alpha(0.4),
            });
        let example = match form.example() {
            Ok(example) => text(format!("For example {example}"))
                .size(12)
                .color(palette.muted),
            Err(error) => text(error).size(12).color(palette.danger),
        };
        let copy = row![
            icon_button(
                Icon::Copy,
                "Copy when saving",
                palette,
                form.kept.copy_on_save,
                Some(event(Event::ToggleCopy)),
            ),
            text(if form.kept.copy_on_save {
                "Saving puts the picture on the clipboard too"
            } else {
                "Saving only saves"
            })
            .size(14),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center);
        let card = container(
            column![
                text("Settings").size(20),
                section(palette, "Save screenshots in", place.into()),
                section(
                    palette,
                    "File name",
                    column![
                        name,
                        example,
                        text(
                            "%Y year, %m month, %d day, %H hour, %M minute, %S second; \
                             a counter is added when the name is taken"
                        )
                        .size(12)
                        .color(palette.muted),
                    ]
                    .spacing(space::XS)
                    .into(),
                ),
                section(palette, "When saving", copy.into()),
                row![
                    Space::new().width(Length::Fill),
                    primary_button(Icon::Done, "Done", palette, event(Event::Close)),
                ],
            ]
            .spacing(space::L),
        )
        .width(520)
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

fn section<'a>(
    palette: Palette,
    title: &'static str,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    column![text(title).size(12).color(palette.muted), content]
        .spacing(space::S)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_keeps_only_names_that_work() {
        let mut editor = Editor::boot(None).0;
        assert!(editor.settings_dialog().is_none());
        let _ = editor.update(Message::Settings(Event::Open));
        assert!(editor.settings_dialog().is_some());
        let _ = editor.update(Message::Settings(Event::Name("shot %H".into())));
        let form = editor.settings.as_ref().unwrap();
        assert_eq!(form.kept.name, "shot %H");
        let _ = editor.update(Message::Settings(Event::Name("a/b".into())));
        let form = editor.settings.as_ref().unwrap();
        assert_eq!(form.kept.name, "shot %H", "a broken name is not kept");
        assert!(form.example().is_err());
        let _ = editor.update(Message::Settings(Event::ToggleCopy));
        assert!(!editor.settings.as_ref().unwrap().kept.copy_on_save);
        let picked = Event::Folder(Ok(Some("/shots".into())));
        let _ = editor.update(Message::Settings(picked));
        assert_eq!(
            editor.settings.as_ref().unwrap().kept.folder(),
            std::path::Path::new("/shots")
        );
        let _ = editor.update(Message::Escape);
        assert!(editor.settings.is_none(), "Esc closes the card");
    }
}
