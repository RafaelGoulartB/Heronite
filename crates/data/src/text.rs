//! Text helpers that reproduce the JavaScript behaviour Heronite relies on:
//! `String.prototype.localeCompare`, `facetKey`, `toPlainText`,
//! `formatTimestamp`, `removeSpecialcharacters`.

use std::cmp::Ordering;

use chrono::{DateTime, Datelike, Local, NaiveDate};
use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{Collator, CollatorBorrowed};
use unicode_normalization::UnicodeNormalization;

thread_local! {
    // `CollatorBorrowed` is neither `Send` nor `Sync`, so one per thread.
    static COLLATOR: CollatorBorrowed<'static> =
        Collator::try_new(Default::default(), CollatorOptions::default())
            .expect("compiled collation data");
    static COLLATOR_BASE: CollatorBorrowed<'static> = {
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Primary);
        Collator::try_new(Default::default(), options).expect("compiled collation data")
    };
}

/// `a.localeCompare(b)` (ICU root collation, like V8 with an `en` locale).
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    COLLATOR.with(|c| c.compare(a, b))
}

/// `a.localeCompare(b, undefined, { sensitivity: 'base' })`.
pub fn locale_cmp_base(a: &str, b: &str) -> Ordering {
    COLLATOR_BASE.with(|c| c.compare(a, b))
}

/// `facetKey` of `collectionFacets.ts`: NFD, no combining marks, collapsed
/// whitespace, trimmed, lower-cased.
pub fn facet_key(value: &str) -> String {
    let stripped: String = value.nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect();
    let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.to_lowercase()
}

/// `splitNames`: comma separated list, trimmed, empty items dropped.
pub fn split_names(value: Option<&str>) -> Vec<String> {
    value.map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()).unwrap_or_default()
}

/// `uniqueNames`: trims, drops empties, de-duplicates by [`facet_key`].
pub fn unique_names<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw in items {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        if seen.insert(facet_key(value)) {
            out.push(value.to_owned());
        }
    }
    out
}

/// `toPlainText` of `CollectionFocus.tsx`: tags become spaces, `&nbsp;` too,
/// whitespace collapsed.
pub fn to_plain_text(value: &str) -> String {
    let mut no_tags = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        no_tags.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('>') {
            // `<[^>]+>` needs at least one character before the `>`.
            Some(end) if end > 0 => {
                no_tags.push(' ');
                rest = &after[end + 1..];
            }
            _ => {
                no_tags.push('<');
                rest = after;
            }
        }
    }
    no_tags.push_str(rest);
    no_tags.replace("&nbsp;", " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `removeSpecialcharacters` of `backend/utils.ts` (key of the wiki cache).
pub fn remove_special_characters(text: &str) -> String {
    const REMOVED: &str = ":|/*?<>\\&{}%$@`!™+'\"®";
    text.chars().filter(|c| !REMOVED.contains(*c)).collect()
}

/// Which `toLocaleDateString(undefined, numeric)` layout to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateLocale {
    /// `M/D/YYYY`
    #[default]
    En,
    /// `DD/MM/YYYY`
    Pt,
    /// `D/M/YYYY`
    Es,
    /// `DD/MM/YYYY`
    Fr,
    /// `D.M.YYYY`
    De,
}

impl DateLocale {
    /// From Heroic's `language` setting (`en`, `pt`, `pt-BR`...).
    pub fn from_language(language: &str) -> Self {
        let short = language.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        match short.as_str() {
            "pt" => Self::Pt,
            "fr" => Self::Fr,
            "es" | "it" => Self::Es,
            "de" | "cs" | "fi" | "hu" => Self::De,
            _ => Self::En,
        }
    }

    pub fn format(self, y: i32, m: u32, d: u32) -> String {
        match self {
            Self::En => format!("{m}/{d}/{y}"),
            Self::Pt | Self::Fr => format!("{d:02}/{m:02}/{y}"),
            Self::Es => format!("{d}/{m}/{y}"),
            Self::De => format!("{d}.{m}.{y}"),
        }
    }
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn month_from_name(name: &str) -> Option<u32> {
    let lower = name.trim_end_matches('.').to_ascii_lowercase();
    if lower.len() < 3 {
        return None;
    }
    MONTHS.iter().position(|m| lower.starts_with(m)).map(|i| i as u32 + 1)
}

/// Parses the date formats found in metadata (`2020-05-12`, `2007-9-14`,
/// `19 Sep, 2022`, `Apr 22, 2013`) as a calendar date, without any timezone
/// shift.
fn parse_calendar_date(value: &str) -> Option<(i32, u32, u32)> {
    let value = value.trim();
    let date_part = value.split(['T', ' ']).next().unwrap_or(value);
    let nums: Vec<&str> = date_part.split('-').collect();
    if nums.len() == 3 && nums.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) {
        let (y, m, d) = (nums[0].parse().ok()?, nums[1].parse().ok()?, nums[2].parse().ok()?);
        NaiveDate::from_ymd_opt(y, m, d)?;
        return Some((y, m, d));
    }
    // "19 Sep, 2022" / "Apr 22, 2013"
    let words: Vec<&str> = value.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()).collect();
    if words.len() == 3 {
        let (day, month, year) = match (month_from_name(words[1]), month_from_name(words[0])) {
            (Some(m), _) => (words[0].parse::<u32>().ok()?, m, words[2].parse::<i32>().ok()?),
            (None, Some(m)) => (words[1].parse::<u32>().ok()?, m, words[2].parse::<i32>().ok()?),
            _ => return None,
        };
        NaiveDate::from_ymd_opt(year, month, day)?;
        return Some((year, month, day));
    }
    None
}

/// `formatTimestamp` of `CollectionFocus.tsx`. Instants with a timezone (the
/// ISO strings Heroic stores for first/last played) are converted to local
/// time; date-only values are shown as written. Unparseable text is returned
/// as is.
///
/// Deviation from the TS: `new Date('2020-05-12')` is parsed as UTC, so
/// Heronite shows the previous day west of UTC. This does not.
pub fn format_timestamp(value: &str, locale: DateLocale) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if value.len() == 4 && value.bytes().all(|b| b.is_ascii_digit()) {
        return value.to_owned();
    }
    if let Ok(instant) = DateTime::parse_from_rfc3339(value) {
        let local = instant.with_timezone(&Local);
        return locale.format(local.year(), local.month(), local.day());
    }
    match parse_calendar_date(value) {
        Some((y, m, d)) => locale.format(y, m, d),
        None => value.to_owned(),
    }
}

/// `sanitize-filename` (npm) with an empty replacement, as Heroic uses it for
/// shortcut file names (creating and removing them must agree).
pub fn sanitize_filename(input: &str) -> String {
    let mut s: String = input
        .chars()
        .filter(|c| !matches!(c, '/' | '?' | '<' | '>' | '\\' | ':' | '*' | '|' | '"') && !matches!(*c as u32, 0x00..=0x1f | 0x80..=0x9f))
        .collect();
    if !s.is_empty() && s.chars().all(|c| c == '.') {
        s.clear();
    }
    let lower = s.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    let reserved = matches!(stem, "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.clear();
    }
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    // 255 bytes, on a char boundary.
    while s.len() > 255 {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_compare_matches_icu_expectations() {
        let mut v = vec!["zeta", "Alpha", "alpha", "Ōkami", "okami", "10 Dead", "2 Dead", "A b", "Ab", "Ao"];
        v.sort_by(|a, b| locale_cmp(a, b));
        assert_eq!(v, ["10 Dead", "2 Dead", "A b", "Ab", "alpha", "Alpha", "Ao", "okami", "Ōkami", "zeta"]);
        assert_eq!(locale_cmp_base("Alpha", "alpha"), Ordering::Equal);
        assert_eq!(locale_cmp_base("Ōkami", "okami"), Ordering::Equal);
    }

    #[test]
    fn facet_key_strips_accents_and_spacing() {
        assert_eq!(facet_key("  Café   Déjà "), "cafe deja");
        assert_eq!(facet_key("Action"), facet_key("action "));
    }

    #[test]
    fn plain_text_strips_tags() {
        assert_eq!(to_plain_text("<p>Hello&nbsp;<b>world</b></p>\n x"), "Hello world x");
        assert_eq!(to_plain_text("a < b > c <> d"), "a c <> d");
        assert_eq!(to_plain_text("1 <2 and 3> 4"), "1 4");
    }

    #[test]
    fn special_characters_are_removed_like_heroic() {
        assert_eq!(remove_special_characters("Onimusha: Way of the Sword"), "Onimusha Way of the Sword");
        assert_eq!(remove_special_characters("Tom's Game® 2™ | x"), "Toms Game 2  x");
    }

    #[test]
    fn dates_format_like_intl() {
        assert_eq!(format_timestamp("2024-09-19", DateLocale::En), "9/19/2024");
        assert_eq!(format_timestamp("2007-9-14", DateLocale::En), "9/14/2007");
        assert_eq!(format_timestamp("19 Sep, 2022", DateLocale::Pt), "19/09/2022");
        assert_eq!(format_timestamp("Apr 22, 2013", DateLocale::En), "4/22/2013");
        assert_eq!(format_timestamp("2010", DateLocale::En), "2010");
        assert_eq!(format_timestamp("Coming soon", DateLocale::En), "Coming soon");
        assert_eq!(format_timestamp("", DateLocale::En), "");
        assert_eq!(DateLocale::from_language("pt-BR"), DateLocale::Pt);
        assert_eq!(DateLocale::from_language("en"), DateLocale::En);
    }

    #[test]
    fn sanitize_matches_npm_package() {
        assert_eq!(sanitize_filename("Half-Life: Alyx"), "Half-Life Alyx");
        assert_eq!(sanitize_filename("a/b\\c?d*e|f\"g<h>i"), "abcdefghi");
        assert_eq!(sanitize_filename("Game..."), "Game");
        assert_eq!(sanitize_filename("trailing "), "trailing");
        assert_eq!(sanitize_filename(".."), "");
        assert_eq!(sanitize_filename("CON"), "");
        assert_eq!(sanitize_filename("con.txt"), "");
        assert_eq!(sanitize_filename("COM1"), "");
        assert_eq!(sanitize_filename("com10"), "com10");
        assert_eq!(sanitize_filename("COM0"), "");
        assert_eq!(sanitize_filename("del\u{7f}"), "del\u{7f}");
        assert_eq!(sanitize_filename("The Witcher 3: Wild Hunt"), "The Witcher 3 Wild Hunt");
        assert_eq!(sanitize_filename("tab\there"), "tabhere");
        assert_eq!(sanitize_filename(&"a".repeat(300)).len(), 255);
    }
}
