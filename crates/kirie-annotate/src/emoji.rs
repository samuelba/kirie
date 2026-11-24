//! The emoji Kirie offers: those the bundled emoji font draws, found by their GitHub shortcode
//! (`:smile:`) or their name, in the skin tone chosen.

use std::collections::HashSet;
use std::sync::OnceLock;

pub use emojis::{Emoji, Group, SkinTone};

/// The skin tones offered, the default yellow first.
pub const SKIN_TONES: [SkinTone; 6] = [
    SkinTone::Default,
    SkinTone::Light,
    SkinTone::MediumLight,
    SkinTone::Medium,
    SkinTone::MediumDark,
    SkinTone::Dark,
];

/// Every emoji the bundled font draws as one glyph of its own, in Unicode's order.
pub fn available() -> &'static [&'static Emoji] {
    static AVAILABLE: OnceLock<Vec<&'static Emoji>> = OnceLock::new();
    AVAILABLE.get_or_init(|| {
        let mut typesetter = crate::text::Typesetter::new();
        emojis::iter()
            .filter(|emoji| typesetter.is_one_emoji(emoji.as_str()))
            .collect()
    })
}

/// The emoji in skin tones other than the default that the bundled font draws as one glyph.
fn toned_available() -> &'static HashSet<&'static str> {
    static TONED: OnceLock<HashSet<&'static str>> = OnceLock::new();
    TONED.get_or_init(|| {
        let mut typesetter = crate::text::Typesetter::new();
        available()
            .iter()
            .filter_map(|emoji| emoji.skin_tones())
            .flatten()
            .filter(|toned| toned.skin_tone() != Some(SkinTone::Default))
            .map(Emoji::as_str)
            .filter(|toned| typesetter.is_one_emoji(toned))
            .collect()
    })
}

/// `emoji` in `tone`, if it comes in skin tones and the font draws that one; else as it is.
pub fn toned(emoji: &'static Emoji, tone: SkinTone) -> &'static Emoji {
    emoji
        .with_skin_tone(tone)
        .filter(|toned| tone == SkinTone::Default || toned_available().contains(toned.as_str()))
        .unwrap_or(emoji)
}

/// The emoji `text` is, in the default skin tone, if the font draws it.
pub fn untoned(text: &str) -> Option<&'static Emoji> {
    let emoji = emojis::get(text)?;
    let untoned = emoji.with_skin_tone(SkinTone::Default).unwrap_or(emoji);
    available().contains(&untoned).then_some(untoned)
}

/// The emoji of `group` the font draws.
pub fn group(group: Group) -> impl Iterator<Item = &'static Emoji> {
    available()
        .iter()
        .copied()
        .filter(move |emoji| emoji.group() == group)
}

/// The emoji called `:shortcode:`, if the font draws it.
pub fn by_shortcode(shortcode: &str) -> Option<&'static Emoji> {
    emojis::get_by_shortcode(shortcode).filter(|emoji| available().contains(emoji))
}

/// At most `limit` emoji matching `query`, best first: a shortcode that is the query, then
/// one that starts with it, then one with a word that starts with it, then a name holding it.
pub fn search(query: &str, limit: usize) -> Vec<&'static Emoji> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(u8, usize, &'static Emoji)> = available()
        .iter()
        .enumerate()
        .filter_map(|(order, emoji)| Some((rank(emoji, &query)?, order, *emoji)))
        .collect();
    found.sort_unstable_by_key(|(rank, order, _)| (*rank, *order));
    found
        .into_iter()
        .take(limit)
        .map(|(_, _, emoji)| emoji)
        .collect()
}

fn rank(emoji: &Emoji, query: &str) -> Option<u8> {
    let shortcode = emoji
        .shortcodes()
        .filter_map(|shortcode| shortcode_rank(shortcode, query))
        .min();
    shortcode.or_else(|| {
        emoji
            .name()
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word.starts_with(query))
            .then_some(3)
    })
}

fn shortcode_rank(shortcode: &str, query: &str) -> Option<u8> {
    if shortcode == query {
        Some(0)
    } else if shortcode.starts_with(query) {
        Some(1)
    } else if shortcode
        .split(['_', '-'])
        .any(|word| word.starts_with(query))
    {
        Some(2)
    } else {
        None
    }
}

/// The shortcode of `emoji` that best matches `query`, else its first one, else its name.
pub fn shortcode_for<'a>(emoji: &'a Emoji, query: &str) -> &'a str {
    let query = query.trim().to_lowercase();
    emoji
        .shortcodes()
        .filter_map(|shortcode| Some((shortcode_rank(shortcode, &query)?, shortcode)))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, shortcode)| shortcode)
        .or_else(|| emoji.shortcode())
        .unwrap_or_else(|| emoji.name())
}

/// What a category of emoji is called, for a tooltip.
pub const fn group_name(group: Group) -> &'static str {
    match group {
        Group::SmileysAndEmotion => "Smileys & emotion",
        Group::PeopleAndBody => "People & body",
        Group::AnimalsAndNature => "Animals & nature",
        Group::FoodAndDrink => "Food & drink",
        Group::TravelAndPlaces => "Travel & places",
        Group::Activities => "Activities",
        Group::Objects => "Objects",
        Group::Symbols => "Symbols",
        Group::Flags => "Flags",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_draws_most_emoji() {
        let all = emojis::iter().count();
        let drawn = available().len();
        assert!(drawn > all * 3 / 4, "{drawn} of {all}");
        assert!(
            available()
                .iter()
                .any(|emoji| emoji.as_str() == "\u{1f44d}")
        );
        assert!(group(Group::Flags).count() > 100);
    }

    #[test]
    fn searches_rank_shortcodes_first() {
        let smile = search("smile", 5);
        assert_eq!(smile.first().map(|emoji| emoji.as_str()), Some("\u{1f604}"));
        let thumbs = search("thu", 8);
        assert!(thumbs.iter().any(|emoji| emoji.as_str() == "\u{1f44d}"));
        assert!(search("cat", 50).len() > 3);
        assert!(search("  ", 5).is_empty());
        assert!(search("zzzqqq", 5).is_empty());
        assert!(
            !search("thu", 50)
                .iter()
                .any(|emoji| emoji.name().contains("Lithuania")),
            "names match by the start of a word"
        );
        assert!(
            search("lithu", 5)
                .iter()
                .any(|emoji| emoji.name().contains("Lithuania"))
        );
        assert_eq!(search("a", 3).len(), 3);
    }

    #[test]
    fn the_shortcode_shown_is_the_one_that_matched() {
        let thumbs = emojis::get("\u{1f44d}").unwrap();
        assert_eq!(shortcode_for(thumbs, "thu"), "thumbsup");
        assert_eq!(shortcode_for(thumbs, "+"), "+1");
        assert_eq!(shortcode_for(thumbs, "zzz"), thumbs.shortcode().unwrap());
    }

    #[test]
    fn emoji_come_in_the_tones_the_font_draws() {
        let thumbs = by_shortcode("thumbsup").unwrap();
        for tone in SKIN_TONES {
            let toned = toned(thumbs, tone);
            assert_eq!(toned.skin_tone(), Some(tone));
            assert_eq!(untoned(toned.as_str()), Some(thumbs));
        }
        let rocket = by_shortcode("rocket").unwrap();
        assert_eq!(toned(rocket, SkinTone::Dark), rocket);
        let handshake = by_shortcode("handshake").unwrap();
        assert_eq!(
            toned(handshake, SkinTone::Medium).skin_tone(),
            Some(SkinTone::Medium)
        );
        assert!(toned_available().len() > 1000);
        assert_eq!(untoned("not an emoji"), None);
    }

    #[test]
    fn shortcodes_find_their_emoji() {
        assert_eq!(by_shortcode("rocket").map(Emoji::as_str), Some("\u{1f680}"));
        assert_eq!(by_shortcode("no_such_thing"), None);
    }
}
