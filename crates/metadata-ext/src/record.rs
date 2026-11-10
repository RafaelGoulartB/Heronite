//! What a script returns, checked and clamped the way the built-in sources
//! map their answers: plain text, capped lists and lengths, dates as
//! `YYYY[-MM[-DD]]`, pictures only as http(s) addresses on the manifest's
//! hosts, and only the fields the manifest provides.

use chrono::{Datelike, NaiveDate};
use heronite_data::age_rating::{AgeRating, RatingSystem};
use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use heronite_metadata::merge::clear_field;
use heronite_metadata::util::{blank_metadata, clip, clip_text, strip_html, unique_names12};
use heronite_metadata::{ArtKind, MatchedBy};
use serde_json::{Map, Value};

use crate::net::{self, HostRule};

/// Hits of one `find` kept at most.
const MAX_HITS: usize = 25;
/// Below this `confidence` a hit is a guess even when the titles agree.
const SURE_CONFIDENCE: f64 = 0.75;

fn text(v: Option<&Value>, max: usize) -> Option<String> {
    v.and_then(Value::as_str).and_then(|s| clip_text(Some(&strip_html(Some(s))), max))
}

fn names(v: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = v else { return Vec::new() };
    let clean: Vec<Option<String>> = items.iter().take(64).map(|i| text(Some(i), 100)).collect();
    unique_names12(clean.iter().map(|s| s.as_deref()))
}

/// `YYYY[-MM[-DD]]` from the usual ways of writing a date: ISO
/// (`2011-04-18`, `2011-04`, `2011`, `2011/04/18`), and English month
/// names (`April 18, 2011`, `18 Apr 2011`, `April 2011`). `None` when it
/// is not a real date between 1950 and 2100.
pub fn parse_date(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['.', ',']).replace(',', " ");
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let year_ok = |y: i32| (1950..=2100).contains(&y);
    let iso = t.replace('/', "-");
    let parts: Vec<&str> = iso.split('-').collect();
    let num = |s: &str| s.parse::<u32>().ok();
    if parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())) && parts[0].len() == 4 {
        let y = num(parts[0])? as i32;
        if !year_ok(y) {
            return None;
        }
        return match parts.as_slice() {
            [_] => Some(format!("{y:04}")),
            [_, m] => (1..=12).contains(&num(m)?).then(|| format!("{y:04}-{:02}", num(m).unwrap_or(1))),
            [_, m, d] => NaiveDate::from_ymd_opt(y, num(m)?, num(d)?).map(|d| d.format("%Y-%m-%d").to_string()),
            _ => None,
        };
    }
    // A four-digit year last: chrono would read "April 2011" as the 20th of
    // April of the year 11.
    if !t.ends_with(|c: char| c.is_ascii_digit()) || !t.rsplit(' ').next().is_some_and(|y| y.len() == 4) {
        return None;
    }
    let real = |d: NaiveDate| year_ok(d.year());
    let days = ["%B %d %Y", "%b %d %Y", "%d %B %Y", "%d %b %Y"];
    if t.split(' ').count() == 3 {
        let day = days.iter().find_map(|f| NaiveDate::parse_from_str(&t, f).ok().filter(|d| real(*d)));
        return day.map(|d| d.format("%Y-%m-%d").to_string());
    }
    let month = ["%d %B %Y", "%d %b %Y"].iter().find_map(|f| NaiveDate::parse_from_str(&format!("01 {t}"), f).ok().filter(|d| real(*d)));
    month.map(|d| d.format("%Y-%m").to_string())
}

/// An http(s) picture address the manifest allows, else `None`. It is
/// marked as an extension's (`heronite_images::public::marked`): whatever
/// downloads it later, now or after a restart, reaches public addresses
/// only.
fn picture(v: Option<&Value>, rules: &[HostRule]) -> Option<String> {
    let s = v?.as_str()?.trim();
    (s.len() <= 2000).then_some(())?;
    net::check(s, rules).ok().map(|u| heronite_images::public::marked(u.as_str()))
}

/// A game a script found, as the host judged it.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub year: Option<i32>,
    pub platform: Option<String>,
    pub thumb: Option<String>,
    /// `adult: true`: its thumbnail is flagged adult.
    pub thumb_adult: bool,
    /// Its name in another script or language (`altTitle`).
    pub alt_title: Option<String>,
    pub matched_by: MatchedBy,
}

impl Hit {
    /// Whether `title` names this hit: its title or its other name.
    pub fn is_named(&self, title: Option<&str>) -> bool {
        [Some(self.title.as_str()), self.alt_title.as_deref()]
            .into_iter()
            .flatten()
            .any(|name| MatchedBy::by_title(name, title) == MatchedBy::ExactTitle)
    }
}

/// What the game carries that a script may say it found it by: the keys of
/// `game.ids` and `game.hashes`.
pub struct GameKeys<'a> {
    pub title: &'a str,
    /// The release year the game's record has, if any.
    pub year: Option<i32>,
    pub ids: &'a Map<String, Value>,
    pub hashes: &'a Map<String, Value>,
}

/// How the hit was found. By an id or a hash only when the hit names one
/// the game has; by title, sure only when the game's title is the hit's
/// title or other name once normalized and the script is not unsure.
fn judge(hit: &Map<String, Value>, names: [Option<&str>; 2], game: &GameKeys<'_>) -> MatchedBy {
    let has = |m: &Map<String, Value>, k: &str| m.get(k).is_some_and(|v| v.as_str().is_some_and(|s| !s.is_empty()));
    if let Some(Value::Object(by)) = hit.get("matchedBy") {
        if let Some(id) = by.get("id").and_then(Value::as_str) {
            if has(game.ids, id) {
                return if id == "steam" { MatchedBy::SteamId } else { MatchedBy::SourceId };
            }
        }
        if let Some(hash) = by.get("hash").and_then(Value::as_str) {
            if has(game.hashes, hash) {
                return MatchedBy::RomHash;
            }
        }
    }
    let unsure = hit.get("confidence").and_then(Value::as_f64).is_some_and(|c| c < SURE_CONFIDENCE);
    let same = names.into_iter().flatten().any(|name| MatchedBy::by_title(game.title, Some(name)) == MatchedBy::ExactTitle);
    if same && !unsure {
        MatchedBy::ExactTitle
    } else {
        MatchedBy::FuzzyTitle
    }
}

/// The hits of `find`, best first: those found by an id or a hash, then
/// the same title, then the guesses, each group in the script's order.
/// Several hits of the game's title are different games of one name
/// ("Doom" of 1993 and of 2016): only the one of the game's year, if the
/// game has one and a single hit has it, stays sure; the others are
/// guesses.
pub fn hits(value: &Value, game: &GameKeys<'_>, pictures: &[HostRule]) -> Vec<Hit> {
    let Value::Array(items) = value else { return Vec::new() };
    let mut out: Vec<Hit> = items
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|h| {
            let id = match h.get("id")? {
                Value::String(s) => s.trim().to_owned(),
                Value::Number(n) => n.to_string(),
                _ => return None,
            };
            if id.is_empty() || id.len() > 200 || id.chars().any(char::is_control) {
                return None;
            }
            let title = text(h.get("title"), 200)?;
            let alt_title = text(h.get("altTitle"), 200).filter(|alt| MatchedBy::by_title(&title, Some(alt)) != MatchedBy::ExactTitle);
            Some(Hit {
                matched_by: judge(h, [Some(&title), alt_title.as_deref()], game),
                thumb_adult: h.get("adult").and_then(Value::as_bool) == Some(true),
                alt_title,
                year: h.get("year").and_then(Value::as_i64).and_then(|y| i32::try_from(y).ok()).filter(|y| (1950..=2100).contains(y)),
                platform: text(h.get("platform"), 100),
                thumb: picture(h.get("thumb"), pictures),
                id,
                title,
            })
        })
        .take(MAX_HITS)
        .collect();
    let exact = out.iter().filter(|h| h.matched_by == MatchedBy::ExactTitle).count();
    if exact > 1 {
        let of_year = |h: &Hit| game.year.is_some() && h.year == game.year;
        let one_of_year = out.iter().filter(|h| h.matched_by == MatchedBy::ExactTitle && of_year(h)).count() == 1;
        for h in out.iter_mut().filter(|h| h.matched_by == MatchedBy::ExactTitle) {
            if !(one_of_year && of_year(h)) {
                h.matched_by = MatchedBy::FuzzyTitle;
            }
        }
    }
    let rank = |by: MatchedBy| match by {
        MatchedBy::ExactTitle => 1,
        MatchedBy::FuzzyTitle => 2,
        _ => 0,
    };
    out.sort_by_key(|h| rank(h.matched_by));
    out
}

/// A record a script fetched, as the host keeps it.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub metadata: CollectionGameMetadata,
    /// Picture fields flagged adult.
    pub adult: Vec<String>,
}

/// The record `fetch` returned, for game `app_name` / `runner`. `None` when
/// it returned nothing (`null`). Fields outside `provides`, and outside
/// `asked` when set, are left out; with `adult_source` every picture
/// counts as adult.
pub fn record(
    value: &Value,
    app_name: &str,
    runner: &str,
    provides: &[String],
    asked: Option<&[String]>,
    pictures: &[HostRule],
    adult_source: bool,
) -> Option<Fetched> {
    let r = value.as_object()?;
    let mut md = blank_metadata(app_name, runner);
    md.title = text(r.get("title"), 200);
    md.description = r.get("description").and_then(Value::as_str).and_then(|d| clip(Some(&strip_html(Some(d)))));
    md.release_date = r.get("releaseDate").and_then(Value::as_str).and_then(parse_date);
    md.developers = names(r.get("developers"));
    md.publishers = names(r.get("publishers"));
    md.genres = names(r.get("genres"));
    md.themes = names(r.get("themes"));
    md.game_modes = names(r.get("gameModes"));
    md.platforms = names(r.get("platforms"));
    md.features = names(r.get("features"));
    md.series = text(r.get("series"), 200);
    md.critic_score = r.get("score").and_then(Value::as_f64).filter(|s| (0.0..=100.0).contains(s)).map(|s| JsNum(s.round()));
    md.age_ratings = age_ratings(r.get("ageRatings"));
    if let Some(Value::Array(sites)) = r.get("websites") {
        md.websites = sites
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| s.len() <= 500 && url::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https")))
            .take(12)
            .map(str::to_owned)
            .collect();
    }
    let mut adult = Vec::new();
    if let Some(Value::Array(images)) = r.get("images") {
        for image in images.iter().filter_map(Value::as_object).take(32) {
            let kind = match (image.get("kind").and_then(Value::as_str), image.get("shape").and_then(Value::as_str)) {
                (Some("cover"), None | Some("tall")) => ArtKind::Cover,
                (Some("cover"), Some("square")) => ArtKind::CoverSquare,
                (Some("cover"), Some("wide")) => ArtKind::CoverWide,
                (Some("background"), _) => ArtKind::Hero,
                _ => continue,
            };
            let slot = kind.of_record_mut(&mut md);
            if slot.is_some() {
                continue;
            }
            if let Some(url) = picture(image.get("url"), pictures) {
                *slot = Some(url);
                if adult_source || image.get("adult").and_then(Value::as_bool) == Some(true) {
                    adult.push(kind.as_str().to_owned());
                }
            }
        }
    }
    for field in heronite_data::settings::ALL_FIELDS {
        let wanted = provides.iter().any(|p| p == field) && asked.is_none_or(|a| a.iter().any(|f| f == field));
        if !wanted {
            clear_field(&mut md, field);
        }
    }
    adult.retain(|f| ArtKind::from_field(f).is_some_and(|k| k.of_record(&md).is_some()));
    Some(Fetched { metadata: md, adult })
}

fn age_ratings(v: Option<&Value>) -> Vec<AgeRating> {
    let Some(Value::Array(items)) = v else { return Vec::new() };
    let mut out: Vec<AgeRating> = Vec::new();
    for item in items.iter().filter_map(Value::as_object).take(16) {
        let Some(system) = item.get("system").and_then(Value::as_str).and_then(RatingSystem::parse) else { continue };
        let Some(rating) = item.get("rating").and_then(Value::as_str) else { continue };
        let descriptors = names(item.get("descriptors"));
        if let Some(r) = AgeRating::new(system, rating, descriptors) {
            if !out.iter().any(|o| o.system == r.system) {
                out.push(r);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
