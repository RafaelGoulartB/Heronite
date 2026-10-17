//! What the Collection groups and sorts by beyond status, title, last played
//! and playtime: the fields of [`GroupBy`] and the extra [`SortMode`]s, the
//! values they read that live outside [`CollectionGame`] ([`GameExtras`],
//! ratings, categories) and the "sort name" titles are ordered by.

use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Datelike, Local};
use heronite_i18n::tr;

use crate::categories::Categories;
use crate::collection::{group_by_status, FacetKind, Game, SortMode};
use crate::library::CollectionGame;
use crate::models::CompletionStatus;
use crate::text::{facet_key, locale_cmp, parse_calendar_date};

/// The field the Collection's list is split into sections by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GroupBy {
    None,
    #[default]
    Status,
    Category,
    Store,
    /// Where the game runs: the emulated system, Linux, Mac or PC.
    Platform,
    Genre,
    Developer,
    Series,
    /// Release year.
    Year,
    /// Ranges of time played.
    Playtime,
    /// Ranges of size on disk.
    InstallSize,
}

impl GroupBy {
    pub const ALL: [GroupBy; 11] = [
        GroupBy::None,
        GroupBy::Status,
        GroupBy::Category,
        GroupBy::Store,
        GroupBy::Platform,
        GroupBy::Genre,
        GroupBy::Developer,
        GroupBy::Series,
        GroupBy::Year,
        GroupBy::Playtime,
        GroupBy::InstallSize,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Status => "status",
            Self::Category => "category",
            Self::Store => "store",
            Self::Platform => "platform",
            Self::Genre => "genre",
            Self::Developer => "developer",
            Self::Series => "series",
            Self::Year => "year",
            Self::Playtime => "playtime",
            Self::InstallSize => "installSize",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.as_str() == value)
    }

    /// A game may sit in several of its sections (one per genre...).
    pub fn many_per_game(self) -> bool {
        matches!(self, Self::Category | Self::Store | Self::Genre | Self::Developer)
    }
}

/// Values of the games kept by the app outside the library, by game key.
#[derive(Debug, Clone, Default)]
pub struct GameExtras {
    /// Day the game entered the library ([`crate::added`]).
    pub added: HashMap<String, String>,
    /// Size on disk of installed games, in bytes.
    pub sizes: HashMap<String, u64>,
    /// Name of the system an emulated game runs on.
    pub systems: HashMap<String, String>,
}

/// Everything sorting and grouping read besides the games themselves.
#[derive(Clone, Copy, Default)]
pub struct Organize<'a> {
    pub extras: Option<&'a GameExtras>,
    /// Star ratings, `1..=5`.
    pub ratings: Option<&'a HashMap<String, u8>>,
    pub categories: Option<&'a Categories>,
    pub statuses: &'a [CompletionStatus],
}

impl Organize<'_> {
    pub fn added<'g>(&'g self, g: &CollectionGame) -> Option<&'g str> {
        self.extras?.added.get(&g.key).map(String::as_str)
    }

    pub fn size(&self, g: &CollectionGame) -> Option<u64> {
        self.extras?.sizes.get(&g.key).copied().filter(|b| *b > 0)
    }

    pub fn rating(&self, g: &CollectionGame) -> Option<u8> {
        self.ratings?.get(&g.key).copied().filter(|r| *r > 0)
    }

    /// The emulated system, else Linux / Mac for a native game, else PC.
    pub fn platform(&self, g: &CollectionGame) -> String {
        if let Some(system) = self.extras.and_then(|e| e.systems.get(&g.key)).filter(|s| !s.is_empty()) {
            return system.clone();
        }
        let native = g.game.install.platform.as_deref().map(str::to_ascii_lowercase);
        match native.as_deref() {
            Some("linux") => "Linux".into(),
            Some("osx" | "mac") => "Mac".into(),
            _ => "PC".into(),
        }
    }
}

/// Times the game was played (sessions recorded).
pub fn times_played(g: &CollectionGame) -> usize {
    g.sessions.as_ref().map_or(0, |s| s.len())
}

/// `(year, month, day)` of a release date as stores write it (ISO date or
/// instant, "19 Sep, 2022", "Apr 22, 2013", a bare year); month and day are
/// `0` when only the year is known.
pub fn release_ymd(raw: &str) -> Option<(i32, u32, u32)> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(instant) = DateTime::parse_from_rfc3339(value) {
        let local = instant.with_timezone(&Local);
        return Some((local.year(), local.month(), local.day()));
    }
    if let Some(date) = parse_calendar_date(value) {
        return Some(date);
    }
    // "2022", "Q3 2024", "Sep 2022": a plausible year in the text.
    value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|w| w.len() == 4)
        .filter_map(|w| w.parse::<i32>().ok())
        .find(|y| (1950..=2100).contains(y))
        .map(|y| (y, 0, 0))
}

pub fn release_year(g: &CollectionGame) -> Option<i32> {
    g.details.release_date.as_deref().and_then(release_ymd).map(|(y, _, _)| y)
}

/// A key the sort compares; games without one go last.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SortValue {
    Num(i64),
    Text(String),
}

/// The value `sort` orders `g` by (bigger and newer first); `None` when the
/// game has none (never played, no rating...). Not used for the title.
pub fn sort_value(g: &CollectionGame, sort: SortMode, o: &Organize) -> Option<SortValue> {
    use SortValue::{Num, Text};
    match sort {
        SortMode::Title => None,
        SortMode::Playtime => Some(Num(g.playtime_minutes as i64)),
        SortMode::LastPlayed => g.last_played.clone().filter(|s| !s.is_empty()).map(Text),
        SortMode::DateAdded => o.added(g).map(|d| Text(d.to_owned())),
        SortMode::TimesPlayed => Some(times_played(g)).filter(|n| *n > 0).map(|n| Num(n as i64)),
        SortMode::InstallSize => o.size(g).filter(|_| g.installed).map(|b| Num(b as i64)),
        SortMode::ReleaseDate => g
            .details
            .release_date
            .as_deref()
            .and_then(release_ymd)
            .map(|(y, m, d)| Num(i64::from(y) * 10_000 + i64::from(m) * 100 + i64::from(d))),
        SortMode::Rating => o.rating(g).map(|r| Num(i64::from(r))),
        SortMode::CriticScore => g.details.critic_score.filter(|s| *s > 0.).map(|s| Num((s * 100.).round() as i64)),
    }
}

/// Roman numeral made of I, V and X, two letters or more ("II", "VII", "XIV").
fn roman(word: &str) -> Option<u32> {
    if word.len() < 2 || !word.bytes().all(|b| matches!(b, b'I' | b'V' | b'X')) {
        return None;
    }
    let digit = |b: u8| match b {
        b'I' => 1,
        b'V' => 5,
        _ => 10,
    };
    let bytes = word.as_bytes();
    let mut total = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        let v = digit(b);
        if bytes.get(i + 1).is_some_and(|&n| digit(n) > v) {
            total -= v;
        } else {
            total += v;
        }
    }
    // Only well-formed numerals: writing the value back gives the word.
    let value = u32::try_from(total).ok().filter(|v| (2..40).contains(v))?;
    (to_roman(value) == word).then_some(value)
}

fn to_roman(mut n: u32) -> String {
    let mut out = String::new();
    for (v, s) in [(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")] {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// What titles are ordered by, as Playnite's sorting name: without a leading
/// "The", "A" or "An", Roman numerals from II up read as numbers and numbers
/// padded so 2 comes before 10 ("Final Fantasy VII" before "IX", "THE
/// FINALS" under F).
pub fn sort_name(title: &str) -> String {
    let t = title.trim();
    let mut rest = t;
    for article in ["the ", "a ", "an "] {
        if t.len() > article.len() && t.get(..article.len()).is_some_and(|p| p.eq_ignore_ascii_case(article)) {
            rest = t[article.len()..].trim_start();
            break;
        }
    }
    let mut out = String::with_capacity(rest.len() + 8);
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        match roman(word) {
            Some(n) => out.push_str(&format!("{n:0>8}")),
            None if word.bytes().all(|b| b.is_ascii_digit()) => {
                let digits = word.trim_start_matches('0');
                out.push_str(&format!("{digits:0>8}"));
            }
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in rest.chars() {
        // Words are runs of letters, or runs of digits ("Hitman3" → "Hitman" "3").
        let same = word.chars().last().is_none_or(|last| last.is_ascii_digit() == c.is_ascii_digit());
        if c.is_alphanumeric() && same {
            word.push(c);
        } else if c.is_alphanumeric() {
            flush(&mut word, &mut out);
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// A section of the grouped list.
#[derive(Debug, Clone)]
pub struct GameGroup {
    /// Status id for status groups; `<field>:<value>` for the others.
    pub id: String,
    pub name: String,
    /// Status colour (`None` for the other fields and "Other").
    pub color: Option<&'static str>,
    pub games: Vec<Game>,
}

const GB: u64 = 1024 * 1024 * 1024;

/// Ranges of [`GroupBy::Playtime`]: (id, label, least minutes), most first.
fn playtime_ranges() -> [(&'static str, &'static str, u64); 6] {
    [
        ("100h", tr!("100 hours or more"), 100 * 60),
        ("50h", tr!("50 to 100 hours"), 50 * 60),
        ("10h", tr!("10 to 50 hours"), 10 * 60),
        ("1h", tr!("1 to 10 hours"), 60),
        ("0h", tr!("Under 1 hour"), 1),
        ("never", tr!("Never played"), 0),
    ]
}

/// Ranges of [`GroupBy::InstallSize`]: (id, label, least bytes), largest first.
fn size_ranges() -> [(&'static str, &'static str, u64); 4] {
    [
        ("50gb", tr!("50 GB or more"), 50 * GB),
        ("10gb", tr!("10 to 50 GB"), 10 * GB),
        ("1gb", tr!("1 to 10 GB"), GB),
        ("0gb", tr!("Under 1 GB"), 1),
    ]
}

/// The sections `g` goes in: `(key, label)`; no value gives none.
fn values_of(g: &CollectionGame, by: GroupBy, o: &Organize) -> Vec<(String, String)> {
    let facet = |kind: FacetKind| g.facets[kind.index()].iter().map(|f| (f.key.clone(), f.value.clone())).collect();
    match by {
        GroupBy::None | GroupBy::Status => Vec::new(),
        GroupBy::Store => facet(FacetKind::Store),
        GroupBy::Genre => facet(FacetKind::Genre),
        GroupBy::Developer => facet(FacetKind::Developer),
        GroupBy::Series => facet(FacetKind::Series).into_iter().take(1).collect(),
        GroupBy::Category => o
            .categories
            .map(|c| c.of_game(&g.runner, &g.app_name).into_iter().map(|n| (facet_key(n), n.to_owned())).collect())
            .unwrap_or_default(),
        GroupBy::Platform => {
            let p = o.platform(g);
            vec![(facet_key(&p), p)]
        }
        GroupBy::Year => release_year(g).map(|y| vec![(y.to_string(), y.to_string())]).unwrap_or_default(),
        GroupBy::Playtime => {
            let (id, label, _) = playtime_ranges().into_iter().find(|(_, _, least)| g.playtime_minutes >= *least).expect("0 matches");
            vec![(id.into(), label.into())]
        }
        GroupBy::InstallSize => {
            if !g.installed {
                return vec![("none".into(), tr!("Not installed").into())];
            }
            match o.size(g) {
                Some(bytes) => {
                    let (id, label, _) = size_ranges().into_iter().find(|(_, _, least)| bytes >= *least).expect("1 byte matches");
                    vec![(id.into(), label.into())]
                }
                None => vec![("unknown".into(), tr!("Size unknown").into())],
            }
        }
    }
}

/// The label of the section of the games with no value.
fn empty_label(by: GroupBy) -> &'static str {
    match by {
        GroupBy::Category => tr!("Uncategorized"),
        GroupBy::Store => tr!("Unknown store"),
        GroupBy::Genre => tr!("No genre"),
        GroupBy::Developer => tr!("Unknown developer"),
        GroupBy::Series => tr!("No series"),
        GroupBy::Year => tr!("Unknown year"),
        _ => tr!("Other"),
    }
}

/// The games of `filtered` (already sorted) in sections of `by`, in its
/// order: statuses in their order, years newest first, ranges largest first,
/// names A-Z; the section of games without a value goes last. A game with
/// several values (genres...) is in each of their sections. Empty sections
/// are left out; [`GroupBy::None`] gives one section with every game.
pub fn group_games(filtered: &[Game], by: GroupBy, o: &Organize) -> Vec<GameGroup> {
    match by {
        GroupBy::None => {
            return vec![GameGroup { id: "all".into(), name: String::new(), color: None, games: filtered.to_vec() }];
        }
        GroupBy::Status => {
            return group_by_status(filtered, o.statuses)
                .into_iter()
                .map(|s| GameGroup { id: s.id, name: s.name, color: s.color, games: s.games })
                .collect();
        }
        _ => {}
    }
    // key -> (label, games), in first-seen order
    let mut sections: Vec<(String, String, Vec<Game>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut without = Vec::new();
    for g in filtered {
        let values = values_of(g, by, o);
        if values.is_empty() {
            without.push(g.clone());
        }
        for (key, label) in values {
            let i = *index.entry(key.clone()).or_insert_with(|| {
                sections.push((key, label, Vec::new()));
                sections.len() - 1
            });
            if sections[i].2.last().is_none_or(|last| last.key != g.key) {
                sections[i].2.push(g.clone());
            }
        }
    }
    let rank = |key: &str| -> usize {
        let ids: Vec<&str> = match by {
            GroupBy::Playtime => playtime_ranges().iter().map(|r| r.0).collect(),
            GroupBy::InstallSize => size_ranges().iter().map(|r| r.0).chain(["unknown", "none"]).collect(),
            _ => Vec::new(),
        };
        ids.iter().position(|id| *id == key).unwrap_or(usize::MAX)
    };
    sections.sort_by(|a, b| match by {
        GroupBy::Year => b.0.cmp(&a.0),
        GroupBy::Playtime | GroupBy::InstallSize => rank(&a.0).cmp(&rank(&b.0)),
        _ => locale_cmp(&a.1, &b.1).then_with(|| a.0.cmp(&b.0)),
    });
    let field = by.as_str();
    let mut out: Vec<GameGroup> =
        sections.into_iter().map(|(key, name, games)| GameGroup { id: format!("{field}:{key}"), name, color: None, games }).collect();
    if !without.is_empty() {
        out.push(GameGroup { id: format!("{field}:"), name: empty_label(by).into(), color: None, games: without });
    }
    out
}

/// Compares two games for `sort`: by its value (bigger and newer first,
/// `descending` turns it around; games without one stay last), ties and the
/// title sort by [`sort_name`].
pub(crate) fn compare_keyed(a: &Keyed, b: &Keyed, sort: SortMode, descending: bool) -> Ordering {
    let turn = |o: Ordering| if descending { o.reverse() } else { o };
    let by_name = || locale_cmp(&a.name, &b.name).then_with(|| locale_cmp(&a.title, &b.title));
    if sort == SortMode::Title {
        return turn(by_name());
    }
    match (&a.value, &b.value) {
        (None, None) => by_name(),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => turn(y.cmp(x)).then_with(by_name),
    }
}

/// A game with what the sort compares, worked out once per sort.
pub(crate) struct Keyed {
    pub value: Option<SortValue>,
    pub name: String,
    pub title: String,
}

impl Keyed {
    pub fn of(g: &CollectionGame, sort: SortMode, o: &Organize) -> Self {
        Self { value: sort_value(g, sort, o), name: sort_name(&g.title), title: g.title.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests_support::game;

    #[test]
    fn sort_names_drop_articles_and_read_numbers() {
        assert_eq!(sort_name("THE FINALS"), "FINALS");
        assert_eq!(sort_name("A Plague Tale: Innocence"), "Plague Tale: Innocence");
        assert_eq!(sort_name("An Untitled Story"), "Untitled Story");
        assert_eq!(sort_name("The"), "The");
        assert_eq!(sort_name("Anno 1800"), "Anno 00001800");
        assert_eq!(sort_name("Final Fantasy VII"), "Final Fantasy 00000007");
        assert_eq!(sort_name("Final Fantasy X"), "Final Fantasy X", "one letter stays");
        assert_eq!(sort_name("Hitman3"), "Hitman00000003");
        assert_eq!(sort_name("Civilization VI"), "Civilization 00000006");
        assert_eq!(sort_name("Mix IIV"), "Mix IIV", "not a numeral");
        let mut titles = ["Final Fantasy IX", "Street Fighter 30th Anniversary", "Final Fantasy VII", "THE FINALS", "Street Fighter 6"];
        titles.sort_by(|a, b| locale_cmp(&sort_name(a), &sort_name(b)));
        assert_eq!(titles, ["Final Fantasy VII", "Final Fantasy IX", "THE FINALS", "Street Fighter 6", "Street Fighter 30th Anniversary"]);
    }

    #[test]
    fn release_dates_in_any_shape() {
        assert_eq!(release_ymd("2022-02-25"), Some((2022, 2, 25)));
        assert_eq!(release_ymd("19 Sep, 2022"), Some((2022, 9, 19)));
        assert_eq!(release_ymd("Apr 22, 2013"), Some((2013, 4, 22)));
        assert_eq!(release_ymd("2016"), Some((2016, 0, 0)));
        assert_eq!(release_ymd("Q3 2024"), Some((2024, 0, 0)));
        assert_eq!(release_ymd("Coming soon"), None);
    }

    #[test]
    fn groups_by_genre_year_and_ranges() {
        let games = vec![
            game("a", "Alpha", |s| {
                s.genres = vec!["RPG".into(), "Action".into()];
                s.minutes = 6000;
            }),
            game("b", "Beta", |s| {
                s.genres = vec!["rpg".into()];
                s.minutes = 30;
            }),
            game("c", "Gamma", |_| {}),
        ];
        let o = Organize::default();
        let genre = group_games(&games, GroupBy::Genre, &o);
        let summary: Vec<_> = genre.iter().map(|g| (g.name.as_str(), g.games.len())).collect();
        assert_eq!(summary, [("Action", 1), ("RPG", 2), ("No genre", 1)]);
        assert_eq!(genre[1].id, "genre:rpg");
        let time = group_games(&games, GroupBy::Playtime, &o);
        let summary: Vec<_> = time.iter().map(|g| (g.id.as_str(), g.games.len())).collect();
        assert_eq!(summary, [("playtime:100h", 1), ("playtime:0h", 1), ("playtime:never", 1)]);
        let size = group_games(&games, GroupBy::InstallSize, &o);
        assert_eq!(size.len(), 1);
        assert_eq!(size[0].id, "installSize:none");
        let all = group_games(&games, GroupBy::None, &o);
        assert_eq!(all[0].games.len(), 3);
    }
}
