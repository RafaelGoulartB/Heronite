//! VNDB, the Visual Novel Database, through its public API v2 ("Kana",
//! `POST {endpoint}/vn`, no key). One request searches and returns every
//! field the candidate needs; a game pinned to an entry is read by its id.
//!
//! VNDB asks clients to stay under 200 requests per 5 minutes (and a second
//! of server time per minute) and answers 429 past that: [`Vndb`] keeps the
//! requests of the last 5 minutes under the limit, and backs off and tries
//! again on a 429. Requests say who they are ([`APP_UA`]), nothing more.
//!
//! Images carry the flagging votes of VNDB's users (`sexual`, `violence`,
//! 0..=2 each). VNDB calls an image safe (and tame) while both averages are
//! at most [`SAFE_MAX`] and someone voted; anything else is suggestive,
//! violent or worse, and is flagged adult ([`MetadataCandidate::adult_pictures`]).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use heronite_data::models::CollectionGameMetadata;
use heronite_data::num::JsNum;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::http::{Http, APP_UA};
use crate::types::{Lookup, MatchedBy, SourceHit};
use crate::util::{blank_metadata, clip, re, score_title, title_queries, unique_names12, MIN_MATCH_SCORE};

pub const SOURCE: &str = "vndb";

/// The fields of a candidate (`POST /vn`'s `fields`).
const DETAIL_FIELDS: &str = "title, alttitle, titles{lang,title,latin,official,main}, aliases, olang, released, description, \
                             rating, votecount, platforms, image{url,dims,sexual,violence,votecount,thumbnail}, \
                             screenshots{url,dims,sexual,violence,votecount}, developers{name,original}, \
                             tags{name,rating,spoiler,lie,category}";
/// The fields of a search hit.
const HIT_FIELDS: &str =
    "title, alttitle, titles{lang,title,latin,official,main}, released, platforms, image{thumbnail,sexual,violence,votecount}";
/// Entries a title lookup reads (with every field: about 40 KB).
const LOOKUP_RESULTS: u32 = 5;
const SEARCH_RESULTS: u32 = 20;

/// The highest average flagging vote VNDB still calls safe (its
/// `set_verdict`: above 0.4 is suggestive / violent, above 1.3 explicit /
/// brutal).
pub const SAFE_MAX: f64 = 0.4;
/// The least rating (0..=3) a tag needs to be a theme: VNDB's users agree
/// it is a clear part of the story.
const MIN_TAG_RATING: f64 = 2.0;

/// Requests allowed per window, a little under VNDB's 200 per 5 minutes.
const WINDOW_REQUESTS: usize = 190;
const WINDOW: Duration = Duration::from_secs(300);
/// Waits before trying again after a 429 without `Retry-After`.
const BACKOFF: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(20)];
/// The longest `Retry-After` honoured.
const MAX_WAIT: Duration = Duration::from_secs(60);

/// What the user set for VNDB (its descriptor's options).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VndbOptions {
    /// Show the original-script title (Japanese) rather than the romanized one.
    pub original_title: bool,
    /// Take a screenshot as the background (VNDB has no background art).
    pub screenshot_background: bool,
}

impl Default for VndbOptions {
    fn default() -> Self {
        Self { original_title: false, screenshot_background: true }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VnTitle {
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub latin: Option<String>,
    #[serde(default)]
    pub official: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VnImage {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Width and height.
    #[serde(default)]
    pub dims: Option<[u32; 2]>,
    #[serde(default)]
    pub sexual: f64,
    #[serde(default)]
    pub violence: f64,
    #[serde(default)]
    pub votecount: u32,
}

impl VnImage {
    /// VNDB's "safe and tame": voted on, and both averages low.
    pub fn is_safe(&self) -> bool {
        self.votecount > 0 && self.sexual <= SAFE_MAX && self.violence <= SAFE_MAX
    }

    fn landscape(&self) -> bool {
        self.dims.is_some_and(|[w, h]| w > h)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VnName {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VnTag {
    #[serde(default)]
    pub name: Option<String>,
    /// 0 (exclusive) ..= 3.
    #[serde(default)]
    pub rating: f64,
    /// 0 none, 1 minor, 2 major.
    #[serde(default)]
    pub spoiler: u8,
    #[serde(default)]
    pub lie: bool,
    /// `cont` (content), `ero` (sexual), `tech` (technical).
    #[serde(default)]
    pub category: Option<String>,
}

/// A `/vn` result with the fields asked.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Vn {
    /// `v17`.
    pub id: String,
    /// The main title, romanized.
    #[serde(default)]
    pub title: Option<String>,
    /// The main title in its original script, when it differs.
    #[serde(default)]
    pub alttitle: Option<String>,
    #[serde(default)]
    pub titles: Vec<VnTitle>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// `YYYY-MM-DD`, `YYYY-MM`, `YYYY` or `TBA`.
    #[serde(default)]
    pub released: Option<String>,
    /// With VNDB's formatting codes ([`plain_text`]).
    #[serde(default)]
    pub description: Option<String>,
    /// Bayesian rating, 10..=100.
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub image: Option<VnImage>,
    #[serde(default)]
    pub screenshots: Vec<VnImage>,
    #[serde(default)]
    pub developers: Vec<VnName>,
    #[serde(default)]
    pub tags: Vec<VnTag>,
}

impl Vn {
    /// Every name it is known by: titles (original and romanized), aliases.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        let titles = self.titles.iter().flat_map(|t| [Some(t.title.as_str()), t.latin.as_deref()]).flatten();
        [self.title.as_deref(), self.alttitle.as_deref()].into_iter().flatten().chain(titles).chain(self.aliases.iter().map(String::as_str))
    }

    /// The best [`score_title`] of `title` against its names.
    fn score(&self, title: &str) -> i32 {
        self.names().map(|n| score_title(title, n)).max().unwrap_or(0)
    }

    /// The title shown and the other one, per the user's option.
    pub fn shown_titles(&self, original_first: bool) -> (Option<String>, Option<String>) {
        let latin = self.title.clone().filter(|t| !t.is_empty());
        let original = self.alttitle.clone().filter(|t| !t.is_empty() && Some(t) != latin.as_ref());
        if original_first && original.is_some() {
            (original, latin)
        } else {
            (latin.or_else(|| original.clone()), original)
        }
    }

    fn year(&self) -> Option<i32> {
        crate::types::release_year(release_date(self.released.as_deref()).as_deref())
    }
}

/// A release date the record keeps: VNDB's full or partial date as it is;
/// `None` for `TBA`, `unknown` and the like.
pub fn release_date(released: Option<&str>) -> Option<String> {
    released.map(str::trim).filter(|d| re!(r"^\d{4}(-\d{2}(-\d{2})?)?$").is_match(d)).map(str::to_owned)
}

/// VNDB's formatting codes as plain text: spoilers left out, links reduced
/// to their text, `[b]`, `[i]`, `[u]`, `[s]`, `[quote]`, `[raw]` and `[code]`
/// dropped. The source note ("[From Steam]") stays.
pub fn plain_text(text: &str) -> String {
    let s = re!(r"(?is)\[spoiler\].*?\[/spoiler\]").replace_all(text, "");
    let s = re!(r"(?is)\[url=[^\]]*\](.*?)\[/url\]").replace_all(&s, "$1");
    let s = re!(r"(?i)\[/?(?:b|i|u|s|quote|raw|code)\]").replace_all(&s, "");
    let s = re!(r"[ \t]+\n").replace_all(&s, "\n");
    let s = re!(r"\n{3,}").replace_all(&s, "\n\n");
    s.trim().to_owned()
}

/// The content tags that are not spoilers or lies, rated at least
/// [`MIN_TAG_RATING`], highest first, at most 12.
pub fn themes(tags: &[VnTag]) -> Vec<String> {
    let mut kept: Vec<&VnTag> =
        tags.iter().filter(|t| t.spoiler == 0 && !t.lie && t.category.as_deref() == Some("cont") && t.rating >= MIN_TAG_RATING).collect();
    kept.sort_by(|a, b| b.rating.total_cmp(&a.rating));
    unique_names12(kept.into_iter().map(|t| t.name.as_deref()))
}

/// The background: the landscape screenshot with the most flagging votes
/// among the safe ones (first on a tie); else the first landscape one,
/// flagged adult (`true`).
pub fn background(screenshots: &[VnImage]) -> Option<(String, bool)> {
    let landscape = || screenshots.iter().filter(|s| s.landscape() && s.url.as_deref().is_some_and(|u| u.starts_with("http")));
    let safe = landscape().filter(|s| s.is_safe()).fold(None::<&VnImage>, |best, s| match best {
        Some(b) if b.votecount >= s.votecount => Some(b),
        _ => Some(s),
    });
    match safe {
        Some(s) => s.url.clone().map(|u| (u, false)),
        None => landscape().next().and_then(|s| s.url.clone()).map(|u| (u, true)),
    }
}

/// A VNDB entry as a candidate's record and the pictures it flags adult.
pub fn map_vn(app_name: &str, runner: &str, vn: &Vn, options: VndbOptions) -> (CollectionGameMetadata, Vec<String>) {
    let mut md = blank_metadata(app_name, runner);
    let mut adult = Vec::new();
    md.title = vn.shown_titles(options.original_title).0;
    md.description = vn.description.as_deref().and_then(|d| clip(Some(&plain_text(d))));
    md.release_date = release_date(vn.released.as_deref());
    md.developers = unique_names12(vn.developers.iter().map(|d| d.name.as_deref()));
    // Every VNDB entry is one: the Collection's genre, as IGDB names it.
    md.genres = vec!["Visual Novel".to_owned()];
    md.themes = themes(&vn.tags);
    md.critic_score = vn.rating.filter(|r| (1.0..=100.0).contains(r)).map(|r| JsNum(r.round()));
    if let Some(image) = vn.image.as_ref().filter(|i| i.url.as_deref().is_some_and(|u| u.starts_with("http"))) {
        md.cover_url = image.url.clone();
        if !image.is_safe() {
            adult.push("cover".to_owned());
        }
    }
    if options.screenshot_background {
        if let Some((url, flagged)) = background(&vn.screenshots) {
            md.hero_url = Some(url);
            if flagged {
                adult.push("hero".to_owned());
            }
        }
    }
    (md, adult)
}

/// The VNDB id a typed text names: `v17`, `V17` or a vndb.org address
/// (`https://vndb.org/v17/chars`). A bare number is a title.
pub fn parse_id(text: &str) -> Option<String> {
    let text = text.trim();
    let path = match text.split_once("vndb.org/") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or(""),
        None => text,
    };
    let caps = re!(r"^[vV](\d+)$").captures(path)?;
    let n: u64 = caps[1].parse().ok().filter(|n| *n > 0)?;
    Some(format!("v{n}"))
}

/// Platform codes as people know them, for a hit's line.
fn platform_name(code: &str) -> String {
    match code {
        "win" => "Windows",
        "lin" => "Linux",
        "mac" => "Mac",
        "ios" => "iOS",
        "and" => "Android",
        "web" => "Web",
        "dvd" => "DVD player",
        "ps1" => "PlayStation",
        "ps2" => "PlayStation 2",
        "ps3" => "PlayStation 3",
        "ps4" => "PlayStation 4",
        "ps5" => "PlayStation 5",
        "psp" => "PSP",
        "psv" => "PS Vita",
        "swi" => "Switch",
        "sw2" => "Switch 2",
        "nds" => "Nintendo DS",
        "n3d" => "Nintendo 3DS",
        "xb3" => "Xbox 360",
        "xbo" => "Xbox One",
        "xxs" => "Xbox Series",
        other => return other.to_uppercase(),
    }
    .to_owned()
}

/// A search hit of `vn` (searched with [`HIT_FIELDS`] or more).
pub fn hit(vn: &Vn, options: VndbOptions) -> Option<SourceHit> {
    let (title, alt_title) = vn.shown_titles(options.original_title);
    let platforms: Vec<String> = vn.platforms.iter().take(3).map(|p| platform_name(p)).collect();
    let image = vn.image.as_ref();
    Some(SourceHit {
        source: SOURCE.into(),
        id: Some(vn.id.clone()),
        title: Some(title?),
        year: vn.year(),
        platform: (!platforms.is_empty()).then(|| platforms.join(", ")),
        thumb_url: image.and_then(|i| i.thumbnail.clone().or_else(|| i.url.clone())).filter(|u| u.starts_with("http")),
        matched_by: None,
        alt_title,
        thumb_adult: image.is_some_and(|i| !i.is_safe()),
    })
}

/// The entry of `results` that is `title`: the best [`score_title`] of any
/// of its names, at least [`MIN_MATCH_SCORE`] (VNDB's order breaks ties),
/// and whether that name is the title itself.
pub fn best_match<'a>(title: &str, results: &'a [Vn]) -> Option<(&'a Vn, MatchedBy)> {
    let mut best: Option<(&Vn, i32)> = None;
    for vn in results {
        let score = vn.score(title);
        if score >= MIN_MATCH_SCORE && best.is_none_or(|(_, s)| score > s) {
            best = Some((vn, score));
        }
    }
    best.map(|(vn, score)| (vn, if score == 100 { MatchedBy::ExactTitle } else { MatchedBy::FuzzyTitle }))
}

/// A request that VNDB kept refusing with 429.
#[derive(Debug)]
struct Throttled;

impl std::fmt::Display for Throttled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VNDB keeps answering 429 (too many requests)")
    }
}

impl std::error::Error for Throttled {}

/// The client: the requests of the last [`WINDOW`], shared by every lookup
/// running at once.
#[derive(Default)]
pub struct Vndb {
    sent: Mutex<VecDeque<Instant>>,
}

impl Vndb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits for a free slot of the window, then takes it.
    fn slot(&self) {
        let mut sent = self.sent.lock();
        while sent.front().is_some_and(|at| at.elapsed() >= WINDOW) {
            sent.pop_front();
        }
        if sent.len() >= WINDOW_REQUESTS {
            if let Some(oldest) = sent.pop_front() {
                std::thread::sleep(WINDOW.saturating_sub(oldest.elapsed()));
            }
        }
        sent.push_back(Instant::now());
    }

    /// `POST /vn` with `body`; after a 429, waits (`Retry-After`, else
    /// [`BACKOFF`]) and tries again, twice at most.
    fn query(&self, http: &Http, body: &Value) -> anyhow::Result<Vec<Vn>> {
        let url = format!("{}/vn", http.endpoints.vndb);
        let body = body.to_string();
        for attempt in 0..=BACKOFF.len() {
            self.slot();
            let resp = http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("User-Agent", APP_UA)
                .body(body.clone())
                .timeout_secs(15)
                .send()?;
            if resp.status == 429 {
                let Some(wait) = BACKOFF.get(attempt) else { break };
                let wait = resp.retry_after.map(Duration::from_secs).unwrap_or(*wait).min(MAX_WAIT);
                log::info!("VNDB asked to slow down; trying again in {}s", wait.as_secs());
                std::thread::sleep(wait);
                continue;
            }
            if !resp.ok() {
                return Err(resp.error());
            }
            let results: Value = resp.json()?;
            let rows = results.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
            return Ok(rows.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect());
        }
        Err(Throttled.into())
    }

    /// The entry `id` (`v17`) with every field.
    pub fn by_id(&self, http: &Http, id: &str) -> Lookup<Vn> {
        let Some(id) = parse_id(id) else { return Lookup::NotFound };
        let body = json!({"filters": ["id", "=", id], "fields": DETAIL_FIELDS, "results": 1});
        found(self.query(http, &body).map(|rows| rows.into_iter().next()), &id)
    }

    /// The entry that is `title`: a search of each of its
    /// [`title_queries`] until one finds it (VNDB matches titles in any
    /// script and aliases), with every field in the same request.
    pub fn by_title(&self, http: &Http, title: &str) -> Lookup<(Vn, MatchedBy)> {
        let mut answered = false;
        let mut failure = None;
        for query in title_queries(title) {
            let body = json!({"filters": ["search", "=", query], "fields": DETAIL_FIELDS, "sort": "searchrank", "results": LOOKUP_RESULTS});
            match self.query(http, &body) {
                Ok(rows) => {
                    answered = true;
                    if let Some((vn, by)) = best_match(title, &rows) {
                        return Lookup::Found((vn.clone(), by));
                    }
                }
                Err(e) => failure = Some(e),
            }
        }
        match failure {
            Some(e) if !answered => {
                log::warn!("VNDB lookup failed for {title}: {e:#}");
                unreachable_or_not_found(&e)
            }
            _ => Lookup::NotFound,
        }
    }

    /// Entries whose name is like `query`, in VNDB's order; a `v17` or a
    /// vndb.org address finds that entry alone.
    pub fn search(&self, http: &Http, query: &str) -> Lookup<Vec<Vn>> {
        let query = query.trim();
        if query.is_empty() {
            return Lookup::Found(Vec::new());
        }
        let filters = match parse_id(query) {
            Some(id) => json!(["id", "=", id]),
            None => json!(["search", "=", query]),
        };
        let body = json!({"filters": filters, "fields": HIT_FIELDS, "sort": "searchrank", "results": SEARCH_RESULTS});
        match self.query(http, &body) {
            Ok(rows) => Lookup::Found(rows),
            Err(e) => {
                log::warn!("VNDB search failed for {query}: {e:#}");
                unreachable_or_not_found(&e)
            }
        }
    }
}

/// A throttled VNDB could not be asked: like no answer, so nothing is
/// saved as "not found".
fn unreachable_or_not_found<T>(e: &anyhow::Error) -> Lookup<T> {
    if e.downcast_ref::<Throttled>().is_some() {
        Lookup::Unreachable
    } else {
        Lookup::from_error(e)
    }
}

fn found(result: anyhow::Result<Option<Vn>>, what: &str) -> Lookup<Vn> {
    match result {
        Ok(Some(vn)) => Lookup::Found(vn),
        Ok(None) => Lookup::NotFound,
        Err(e) => {
            log::warn!("VNDB lookup of {what} failed: {e:#}");
            unreachable_or_not_found(&e)
        }
    }
}

#[cfg(test)]
mod tests;
