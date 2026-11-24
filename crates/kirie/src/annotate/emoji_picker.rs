//! Choosing an emoji: a search, the emoji by category, and the ones used last, in the skin tone
//! chosen. The ones used last and the tone are kept for the next session.

use iced::widget::{Row, button, column, container, row, scrollable, svg, text, text_input};
use iced::{Background, Border, Element, Length};
use kirie_annotate::emoji::{self, Emoji, Group, SKIN_TONES, SkinTone};
use kirie_ui::icon::Icon;
use kirie_ui::{Palette, radius, space};

const FILE: &str = "emojis.ron";
const TONE_FILE: &str = "emoji-tone.ron";
/// The emoji that shows the skin tones.
const TONE_SAMPLE: &str = "\u{270b}";
/// Emoji used last, newest first.
const RECENT: usize = 24;
const FALLBACK: &str = "\u{1f44d}";
const CELL: f32 = 34.0;
const GLYPH: f32 = 22.0;
const SEARCH: f32 = 30.0;
const TAB: f32 = 30.0;
const TAB_ICON: f32 = 16.0;
const HEADING: f32 = 16.0;
const GRID: f32 = 4.0 * CELL;
const FOUND: usize = 96;

/// The face emoji are drawn in, the same one marks use, so the picker shows what you get.
pub const FONT: iced::Font = iced::Font::with_name(kirie_annotate::text::EMOJI_FAMILY);
/// The height the picker takes.
pub const HEIGHT: f32 = SEARCH + TAB + HEADING + GRID + 3.0 * space::XS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Recent,
    Group(Group),
}

impl Tab {
    fn all() -> impl Iterator<Item = Tab> {
        std::iter::once(Tab::Recent).chain(Group::iter().map(Tab::Group))
    }

    const fn icon(self) -> Icon {
        match self {
            Tab::Recent => Icon::EmojiRecent,
            Tab::Group(group) => match group {
                Group::SmileysAndEmotion => Icon::Sticker,
                Group::PeopleAndBody => Icon::EmojiPeople,
                Group::AnimalsAndNature => Icon::EmojiNature,
                Group::FoodAndDrink => Icon::EmojiFood,
                Group::TravelAndPlaces => Icon::EmojiTravel,
                Group::Activities => Icon::EmojiActivities,
                Group::Objects => Icon::EmojiObjects,
                Group::Symbols => Icon::EmojiSymbols,
                Group::Flags => Icon::EmojiFlags,
            },
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Tab::Recent => "Recently used",
            Tab::Group(group) => emoji::group_name(group),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiPicker {
    query: String,
    tab: Tab,
    /// In the default skin tone, shown in the one chosen.
    recent: Vec<String>,
    tone: SkinTone,
    /// Whether the skin tones are offered in place of the categories.
    toning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Search(String),
    Tab(Tab),
    Choose(String),
    /// Shows or hides the skin tones.
    Toning,
    Tone(SkinTone),
}

impl Default for EmojiPicker {
    fn default() -> Self {
        Self::with_recent(Vec::new())
    }
}

impl EmojiPicker {
    fn with_recent(recent: Vec<String>) -> Self {
        let tab = if recent.is_empty() {
            Tab::Group(Group::SmileysAndEmotion)
        } else {
            Tab::Recent
        };
        Self {
            query: String::new(),
            tab,
            recent,
            tone: SkinTone::Default,
            toning: false,
        }
    }

    /// The picker with the emoji used last time.
    pub fn load() -> Self {
        let recent: Vec<String> = crate::storage::store::state_path(FILE)
            .and_then(|path| crate::storage::store::read(&path))
            .unwrap_or_default();
        let tone = crate::storage::store::state_path(TONE_FILE)
            .and_then(|path| crate::storage::store::read(&path))
            .filter(|tone| SKIN_TONES.contains(tone))
            .unwrap_or(SkinTone::Default);
        let mut recent: Vec<String> = recent
            .iter()
            .filter_map(|kept| emoji::untoned(kept))
            .map(|untoned| untoned.as_str().to_owned())
            .collect();
        dedup_in_order(&mut recent);
        recent.truncate(RECENT);
        Self {
            tone,
            ..Self::with_recent(recent)
        }
    }

    /// Reads the recent emoji and skin tone again, keeping the tab and search shown.
    pub fn reload(&mut self) {
        let Self { recent, tone, .. } = Self::load();
        self.recent = recent;
        self.tone = tone;
    }

    /// The emoji new stickers show: the one used last.
    pub fn chosen(&self) -> &str {
        self.recent
            .first()
            .and_then(|kept| emoji::untoned(kept))
            .map_or(FALLBACK, |untoned| self.toned(untoned).as_str())
    }

    pub fn tone(&self) -> SkinTone {
        self.tone
    }

    /// `emoji` in the skin tone chosen.
    pub fn toned(&self, emoji: &'static Emoji) -> &'static Emoji {
        emoji::toned(emoji, self.tone)
    }

    /// Applies `event`; the emoji chosen, if one was.
    pub fn update(&mut self, event: Event) -> Option<String> {
        match event {
            Event::Search(query) => {
                self.query = query;
                None
            }
            Event::Tab(tab) => {
                self.query.clear();
                self.tab = tab;
                self.toning = false;
                None
            }
            Event::Choose(emoji) => {
                self.remember(&emoji);
                Some(emoji)
            }
            Event::Toning => {
                self.toning = !self.toning;
                None
            }
            Event::Tone(tone) => {
                self.tone = tone;
                self.toning = false;
                keep(TONE_FILE, &tone, "skin tone");
                None
            }
        }
    }

    /// Puts `emoji` first among the ones used last, and keeps them.
    pub fn remember(&mut self, emoji: &str) {
        self.push(emoji);
        keep(FILE, &self.recent, "recent emoji");
    }

    fn push(&mut self, emoji: &str) {
        let Some(untoned) = emoji::untoned(emoji) else {
            return;
        };
        let untoned = untoned.as_str();
        self.recent.retain(|kept| kept != untoned);
        self.recent.insert(0, untoned.to_owned());
        self.recent.truncate(RECENT);
    }

    fn shown(&self) -> Vec<&'static str> {
        let untoned: Vec<&'static Emoji> = if self.query.trim().is_empty() {
            match self.tab {
                Tab::Recent => self
                    .recent
                    .iter()
                    .filter_map(|kept| emoji::untoned(kept))
                    .collect(),
                Tab::Group(group) => emoji::group(group).collect(),
            }
        } else {
            emoji::search(&self.query, FOUND)
        };
        untoned
            .into_iter()
            .map(|emoji| self.toned(emoji).as_str())
            .collect()
    }

    /// The search, the categories and the emoji in a grid `width` wide; `current` is the emoji
    /// shown as chosen.
    pub fn view<'a>(
        &'a self,
        current: &str,
        palette: Palette,
        width: impl Into<Length>,
    ) -> Element<'a, Event> {
        let search = text_input("Search emoji", &self.query)
            .on_input(Event::Search)
            .size(13)
            .padding([space::XS, space::S])
            .width(Length::Fill)
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
        let searching = !self.query.trim().is_empty();
        let tone = kirie_ui::widget::hint(
            cell(
                tone_sample(self.tone),
                self.toning,
                SEARCH,
                palette,
                Event::Toning,
            ),
            "Skin tone",
            palette,
        );
        let tabs = if self.toning {
            Row::with_children(SKIN_TONES.into_iter().map(|tone| {
                cell(
                    tone_sample(tone),
                    tone == self.tone,
                    TAB,
                    palette,
                    Event::Tone(tone),
                )
            }))
        } else {
            Row::with_children(Tab::all().map(|tab| {
                let active = !searching && self.tab == tab;
                kirie_ui::widget::hint(tab_button(tab, active, palette), tab.name(), palette)
            }))
        }
        .spacing(2);
        let heading = if self.toning {
            "Skin tone".to_owned()
        } else if searching {
            format!("Results for \u{201c}{}\u{201d}", self.query.trim())
        } else {
            self.tab.name().to_owned()
        };
        let heading = container(text(heading).size(12).color(palette.muted)).height(HEADING);
        let shown = self.shown();
        let grid: Element<'a, Event> = if shown.is_empty() {
            container(
                text(if searching {
                    "No emoji found"
                } else {
                    "Emoji you use show up here"
                })
                .size(13)
                .color(palette.muted),
            )
            .center_x(Length::Fill)
            .padding(space::M)
            .into()
        } else {
            Row::with_children(shown.into_iter().map(|face| {
                cell(
                    face,
                    face == current,
                    CELL,
                    palette,
                    Event::Choose(face.to_owned()),
                )
            }))
            .spacing(2)
            .wrap()
            .into()
        };
        let bar = scrollable::Scrollbar::new()
            .width(4)
            .scroller_width(4)
            .spacing(space::XS);
        column![
            row![search, tone].spacing(space::XS),
            tabs,
            heading,
            scrollable(grid)
                .direction(scrollable::Direction::Vertical(bar))
                .height(GRID)
                .width(Length::Fill),
        ]
        .spacing(space::XS)
        .width(width)
        .into()
    }
}

/// The emoji that shows `tone`.
fn tone_sample(tone: SkinTone) -> &'static str {
    emoji::untoned(TONE_SAMPLE).map_or(TONE_SAMPLE, |hand| emoji::toned(hand, tone).as_str())
}

/// Keeps `value` in the state file `name` for the next session.
fn keep<T: serde::Serialize>(name: &str, value: &T, what: &str) {
    let saved = crate::storage::store::state_path(name)
        .ok_or_else(|| "cannot find the state folder".to_owned())
        .and_then(|path| crate::storage::store::save(&path, value));
    if let Err(error) = saved {
        tracing::warn!(%error, "{what} not saved");
    }
}

/// Drops the repeats of `list`, keeping each first one where it is.
fn dedup_in_order(list: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    list.retain(|item| seen.insert(item.clone()));
}

/// A category's tab: its icon, underlined while `active`, so it reads apart from the emoji.
fn tab_button<'a>(tab: Tab, active: bool, palette: Palette) -> Element<'a, Event> {
    let tint = if active {
        palette.accent
    } else {
        palette.muted
    };
    let icon = svg(tab.icon().handle())
        .width(TAB_ICON)
        .height(TAB_ICON)
        .style(move |_, _| svg::Style { color: Some(tint) });
    let underline = container(text(""))
        .width(Length::Fill)
        .height(2)
        .style(move |_| container::Style {
            background: active.then_some(Background::Color(palette.accent)),
            ..container::Style::default()
        });
    button(column![container(icon).center(Length::Fill), underline])
        .width(Length::Fill)
        .height(TAB)
        .padding(0)
        .on_press(Event::Tab(tab))
        .style(move |_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(palette.surface[1])),
            border: Border {
                radius: radius::CONTROL.into(),
                ..Border::default()
            },
            ..button::Style::default()
        })
        .into()
}

/// A square button `size` wide showing `face`, outlined while it is the one `chosen`.
fn cell(
    face: &str,
    chosen: bool,
    size: f32,
    palette: Palette,
    on_press: Event,
) -> Element<'_, Event> {
    button(container(text(face).font(FONT).size(GLYPH).line_height(1.0)).center(Length::Fill))
        .width(size)
        .height(size)
        .padding(0)
        .on_press(on_press)
        .style(move |_, status| {
            let background = match (chosen, status) {
                (true, _) => Some(palette.accent.scale_alpha(0.2)),
                (false, button::Status::Hovered) => Some(palette.surface[1]),
                (false, button::Status::Pressed) => Some(palette.surface[2]),
                _ => None,
            };
            button::Style {
                background: background.map(Background::Color),
                text_color: palette.text,
                border: Border {
                    color: palette.accent,
                    width: if chosen { 1.5 } else { 0.0 },
                    radius: radius::CONTROL.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reloading_keeps_the_tab_and_search_shown() {
        let mut picker = EmojiPicker::default();
        picker.update(Event::Tab(Tab::Group(Group::Activities)));
        picker.update(Event::Search("thu".into()));
        picker.reload();
        assert_eq!(picker.tab, Tab::Group(Group::Activities));
        assert_eq!(picker.query, "thu");
    }

    #[test]
    fn choosing_keeps_the_newest_first_and_once() {
        let mut picker = EmojiPicker::default();
        assert_eq!(picker.chosen(), FALLBACK);
        assert_eq!(picker.tab, Tab::Group(Group::SmileysAndEmotion));
        for emoji in ["\u{1f680}", "\u{1f389}", "\u{1f680}"] {
            picker.push(emoji);
        }
        assert_eq!(picker.chosen(), "\u{1f680}");
        assert_eq!(picker.recent.len(), 2);
        for _ in 0..RECENT {
            picker.push("\u{1f389}");
            picker.push("\u{1f680}");
        }
        assert_eq!(picker.recent.len(), 2);
    }

    #[test]
    fn searching_overrides_the_tab() {
        let mut picker = EmojiPicker::with_recent(vec!["\u{1f389}".into()]);
        assert_eq!(picker.tab, Tab::Recent);
        assert_eq!(picker.shown(), ["\u{1f389}"]);
        picker.update(Event::Search("rocket".into()));
        assert_eq!(picker.shown().first(), Some(&"\u{1f680}"));
        picker.update(Event::Tab(Tab::Group(Group::Flags)));
        assert!(picker.query.is_empty());
        assert!(picker.shown().len() > 100);
    }

    #[test]
    fn the_tone_chosen_colors_what_is_shown_and_chosen() {
        let thumbs = "\u{1f44d}";
        let thumbs_medium = "\u{1f44d}\u{1f3fd}";
        let mut picker = EmojiPicker::default();
        picker.update(Event::Toning);
        assert!(picker.toning);
        picker.update(Event::Tone(SkinTone::Medium));
        assert!(!picker.toning);
        picker.update(Event::Search("thumbsup".into()));
        assert_eq!(picker.shown().first(), Some(&thumbs_medium));
        assert_eq!(tone_sample(SkinTone::Medium), "\u{270b}\u{1f3fd}");
        picker.update(Event::Choose(thumbs_medium.into()));
        picker.push(thumbs);
        assert_eq!(picker.recent, [thumbs], "kept once, in the default tone");
        assert_eq!(picker.chosen(), thumbs_medium);
        picker.update(Event::Tone(SkinTone::Default));
        assert_eq!(picker.chosen(), thumbs);
        picker.push("\u{1f680}");
        picker.update(Event::Tone(SkinTone::Dark));
        assert_eq!(
            picker.chosen(),
            "\u{1f680}",
            "emoji without tones stay as they are"
        );
    }
}
