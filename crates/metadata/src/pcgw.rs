//! PC facts from PCGamingWiki, written to `store_cache/pcgamingwiki.json`
//! (read by `heronite_data::pcgw`).
//!
//! PCGamingWiki refuses anonymous Cargo queries, so the page is found the
//! way its own tools do (`/api/appid.php` and `/api/gog.php` answer with a
//! redirect to the page; the `Location` is read, not followed) or by its
//! exact title, and its wikitext is read through the MediaWiki API. Only
//! bare facts are kept (see `heronite_data::pcgw`).
//!
//! Requests go one at a time, at least a second apart, with an honest
//! User-Agent.

use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::paths::{percent_decode, DataPaths};
use heronite_data::pcgw::{PcgwEntry, PcgwFacts, PcgwFile, PcgwQuery, SaveLocation, Support};
use parking_lot::Mutex;
use serde_json::Value;

use crate::http::Http;
use crate::util::{now_ms, re};

pub const USER_AGENT: &str = concat!("Heronite/", env!("CARGO_PKG_VERSION"), " (+https://github.com/RafaelGoulartB/Heronite)");
/// At least this long between two requests.
pub const GAP: Duration = Duration::from_secs(1);
const FOUND_TTL_MS: i64 = 30 * 24 * 3600 * 1000;
const MISSING_TTL_MS: i64 = 7 * 24 * 3600 * 1000;

// ---- wikitext ----

/// The bodies (between `{{` and `}}`, name included) of every `{{name…}}`
/// template, nested ones too. The name matches with its first letter in
/// either case, like MediaWiki.
fn templates<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(rel) = text[i..].find("{{") {
        let start = i + rel;
        let rest = &text[start + 2..];
        let head = rest.trim_start();
        let same_name = head.len() >= name.len()
            && head.is_char_boundary(name.len())
            && head[..name.len()].chars().skip(1).eq(name.chars().skip(1))
            && head[..name.len()].chars().next().map(|c| c.to_lowercase().to_string())
                == name.chars().next().map(|c| c.to_lowercase().to_string());
        let after = head.get(name.len()..).and_then(|r| r.chars().next());
        if same_name && matches!(after, Some('|' | '}' | '\n' | '\r' | ' ')) {
            // Find the matching `}}`.
            let mut depth = 0usize;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b'{' && bytes[j + 1] == b'{' {
                    depth += 1;
                    j += 2;
                } else if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            if depth == 0 {
                out.push(&text[start + 2..j - 2]);
            }
        }
        i = start + 2;
    }
    out
}

/// The `|`-separated parts of a template body at its own level (name first).
fn parts(body: &str) -> Vec<&str> {
    let (mut out, mut depth, mut last) = (Vec::new(), 0i32, 0);
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1)) {
            (b'{', Some(b'{')) | (b'[', Some(b'[')) => {
                depth += 1;
                i += 2;
                continue;
            }
            (b'}', Some(b'}')) | (b']', Some(b']')) => {
                depth -= 1;
                i += 2;
                continue;
            }
            (b'|', _) if depth == 0 => {
                out.push(&body[last..i]);
                last = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&body[last..]);
    out
}

/// `key = value` parts of a template body (keys lowercased).
fn params(body: &str) -> Vec<(String, &str)> {
    parts(body)
        .into_iter()
        .skip(1)
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_lowercase(), v.trim()))
        })
        .collect()
}

fn param<'a>(params: &[(String, &'a str)], key: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
}

/// A value as text: references, comments, markup and templates dropped,
/// links reduced to their label.
fn clean(value: &str) -> String {
    let v = re!(r"(?s)<!--.*?-->").replace_all(value, "");
    let v = re!(r"(?s)<ref[^>]*/>").replace_all(&v, "");
    let v = re!(r"(?s)<ref[^>]*>.*?</ref>").replace_all(&v, "");
    let v = re!(r"<[^>]+>").replace_all(&v, "");
    let v = re!(r"\[\[(?:[^|\]]*\|)?([^\]]*)\]\]").replace_all(&v, "$1");
    let mut v = v.into_owned();
    // Innermost templates first.
    let innermost = re!(r"\{\{[^{}]*\}\}");
    while let Some(next) = innermost.find(&v).map(|m| m.range()) {
        v.replace_range(next, "");
    }
    v.trim().to_owned()
}

/// `{{p|appdata}}` and the other path tokens of the save tables.
fn path_token(token: &str) -> String {
    let t = token.trim().to_lowercase().replace('/', "\\");
    match t.as_str() {
        "game" => "<game folder>".into(),
        "steam" => "<Steam folder>".into(),
        "uid" => "<user id>".into(),
        "username" => "<user name>".into(),
        "appdata" => "%APPDATA%".into(),
        "localappdata" => "%LOCALAPPDATA%".into(),
        "userprofile" => "%USERPROFILE%".into(),
        "userprofile\\documents" => "%USERPROFILE%\\Documents".into(),
        "userprofile\\appdata\\locallow" => "%USERPROFILE%\\AppData\\LocalLow".into(),
        "public" => "%PUBLIC%".into(),
        "programdata" => "%PROGRAMDATA%".into(),
        "windir" => "%WINDIR%".into(),
        "hkcu" => "HKEY_CURRENT_USER".into(),
        "hklm" => "HKEY_LOCAL_MACHINE".into(),
        "wow64" => "WOW6432Node".into(),
        "osxhome" | "linuxhome" => "~".into(),
        "xdgdatahome" => "$XDG_DATA_HOME".into(),
        "xdgconfighome" => "$XDG_CONFIG_HOME".into(),
        other => format!("<{other}>"),
    }
}

fn save_path(raw: &str) -> String {
    // Tokens become markers first: `clean` would take `<game folder>` for a tag.
    let mut tokens = Vec::new();
    let marked = re!(r"(?i)\{\{\s*p\s*\|([^{}|]*)\}\}").replace_all(raw, |c: &regex::Captures| {
        tokens.push(path_token(&c[1]));
        format!("\u{1}{}\u{2}", tokens.len() - 1)
    });
    let cleaned = clean(&marked);
    re!("\u{1}(\\d+)\u{2}")
        .replace_all(&cleaned, |c: &regex::Captures| c[1].parse::<usize>().ok().and_then(|i| tokens.get(i)).cloned().unwrap_or_default())
        .into_owned()
}

fn ids(value: Option<&str>) -> Vec<String> {
    clean(value.unwrap_or(""))
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_owned)
        .collect()
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !value.is_empty() && !list.iter().any(|v| v.eq_ignore_ascii_case(&value)) {
        list.push(value);
    }
}

/// The facts of a page's wikitext.
pub fn parse_wikitext(page: &str, revid: Option<u64>, text: &str) -> PcgwFacts {
    let mut f = PcgwFacts { page: page.to_owned(), revid, ..Default::default() };
    if let Some(info) = templates(text, "Infobox game").first() {
        let p = params(info);
        f.steam_ids = ids(param(&p, "steam appid"));
        f.steam_ids.extend(ids(param(&p, "steam appid side")));
        f.gog_ids = ids(param(&p, "gogcom id"));
        f.gog_ids.extend(ids(param(&p, "gogcom id side")));
        f.hltb_id = ids(param(&p, "hltb")).into_iter().next();
    }
    for engine in templates(text, "Infobox game/row/engine") {
        if let Some(name) = parts(engine).get(1) {
            push_unique(&mut f.engines, clean(name));
        }
    }
    if let Some(video) = templates(text, "Video").first() {
        let p = params(video);
        let s = |k: &str| param(&p, k).map(clean).and_then(|v| Support::parse(&v));
        (f.ultrawide, f.hdr, f.fps_120, f.uhd_4k) = (s("ultrawidescreen"), s("hdr"), s("120 fps"), s("4k ultra hd"));
    }
    if let Some(input) = templates(text, "Input").first() {
        let p = params(input);
        let s = |k: &str| param(&p, k).map(clean).and_then(|v| Support::parse(&v));
        (f.controller, f.full_controller) = (s("controller support"), s("full controller"));
    }
    if let Some(mw) = templates(text, "Middleware").first() {
        for name in clean(param(&params(mw), "anticheat").unwrap_or("")).split(',') {
            push_unique(&mut f.anticheat, name.trim().to_owned());
        }
    }
    for saves in templates(text, "Game data/saves") {
        let ps = parts(saves);
        let Some(platform) = ps.get(1).map(|p| clean(p)).filter(|p| !p.is_empty()) else { continue };
        let paths: Vec<String> = ps.iter().skip(2).map(|p| save_path(p)).filter(|p| !p.is_empty()).collect();
        if !paths.is_empty() {
            f.saves.push(SaveLocation { platform, paths });
        }
    }
    f.fixes = templates(text, "Fixbox").len() as u32;
    f
}

// ---- requests ----

/// Paces requests (one at a time, [`GAP`] apart).
pub struct PcgwClient {
    last: Mutex<Option<Instant>>,
    gap: Mutex<Duration>,
}

impl Default for PcgwClient {
    fn default() -> Self {
        Self::new(GAP)
    }
}

/// The page title in a `/wiki/<Page>` address.
fn page_from_location(location: &str) -> Option<String> {
    let (_, rest) = location.split_once("/wiki/")?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let page = percent_decode(rest).replace('_', " ").trim().to_owned();
    (!page.is_empty()).then_some(page)
}

impl PcgwClient {
    pub fn new(gap: Duration) -> Self {
        Self { last: Mutex::new(None), gap: Mutex::new(gap) }
    }

    pub fn set_gap(&self, gap: Duration) {
        *self.gap.lock() = gap;
    }

    /// Waits until the previous request is `gap` old; holds the lock while
    /// the request runs, so requests never overlap.
    fn send(&self, http: &Http, path_and_query: &str, follow: bool) -> Result<crate::http::Response> {
        let mut last = self.last.lock();
        if let Some(at) = *last {
            if let Some(wait) = self.gap.lock().checked_sub(at.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        let base = http.endpoints.pcgamingwiki.trim_end_matches('/');
        let mut req = http
            .get(format!("{base}{path_and_query}"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .timeout_secs(15);
        if !follow {
            req = req.no_redirects();
        }
        let resp = req.send();
        *last = Some(Instant::now());
        let resp = resp?;
        if resp.status >= 500 || resp.status == 429 || resp.status == 403 {
            bail!("PCGamingWiki answered {}", resp.status);
        }
        Ok(resp)
    }

    /// `/api/appid.php` or `/api/gog.php`: the page a store id redirects to.
    fn page_by_redirect(&self, http: &Http, path: &str, id: &str) -> Result<Option<String>> {
        let resp = self.send(http, &format!("{path}{id}"), false)?;
        Ok(match resp.status {
            301 | 302 | 303 | 307 | 308 => resp.location.as_deref().and_then(page_from_location),
            _ => None,
        })
    }

    /// The page of an exact title (after the wiki's redirects).
    fn page_by_title(&self, http: &Http, title: &str) -> Result<Option<String>> {
        let q = crate::util::encode_uri_component(title.trim());
        let resp = self.send(http, &format!("/w/api.php?action=query&titles={q}&redirects=1&format=json&formatversion=2"), true)?;
        let v: Value = resp.json()?;
        let page = v.pointer("/query/pages/0").filter(|p| !p.get("missing").is_some_and(|m| m.as_bool() != Some(false)));
        Ok(page.and_then(|p| p.get("title")).and_then(Value::as_str).map(str::to_owned))
    }

    /// The wikitext of a page: `(title, revid, text)`.
    fn wikitext(&self, http: &Http, page: &str) -> Result<Option<(String, Option<u64>, String)>> {
        let q = crate::util::encode_uri_component(page);
        let resp = self.send(
            http,
            &format!("/w/api.php?action=parse&page={q}&prop=wikitext%7Crevid&redirects=1&format=json&formatversion=2"),
            true,
        )?;
        let v: Value = resp.json()?;
        if v.pointer("/error/code").and_then(Value::as_str) == Some("missingtitle") {
            return Ok(None);
        }
        if let Some(code) = v.pointer("/error/code").and_then(Value::as_str) {
            bail!("PCGamingWiki: {code}");
        }
        let Some(parse) = v.get("parse") else { bail!("PCGamingWiki: no page in the answer") };
        let title = parse.get("title").and_then(Value::as_str).unwrap_or(page).to_owned();
        let text = parse.get("wikitext").and_then(|w| w.as_str().or_else(|| w.get("*").and_then(Value::as_str))).unwrap_or("");
        Ok(Some((title, parse.get("revid").and_then(Value::as_u64), text.to_owned())))
    }

    /// Finds and reads the page of `query`: by Steam id, GOG id, then exact
    /// title (refused when the page lists other store ids than the game's).
    pub fn fetch(&self, http: &Http, query: &PcgwQuery) -> Result<Option<PcgwFacts>> {
        let id = |v: &Option<String>| v.clone().filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()));
        let (steam, gog) = (id(&query.steam_id), id(&query.gog_id));
        let mut page = None;
        if let Some(s) = &steam {
            page = self.page_by_redirect(http, "/api/appid.php?appid=", s)?;
        }
        if page.is_none() {
            if let Some(g) = &gog {
                page = self.page_by_redirect(http, "/api/gog.php?page=", g)?;
            }
        }
        let by_title = page.is_none();
        if by_title && !query.title.trim().is_empty() {
            page = self.page_by_title(http, &query.title)?;
        }
        let Some(page) = page else { return Ok(None) };
        let Some((title, revid, text)) = self.wikitext(http, &page)? else { return Ok(None) };
        let facts = parse_wikitext(&title, revid, &text);
        let contradicts =
            |ours: &Option<String>, theirs: &[String]| ours.as_ref().is_some_and(|o| !theirs.is_empty() && !theirs.contains(o));
        if by_title && (contradicts(&steam, &facts.steam_ids) || contradicts(&gog, &facts.gog_ids)) {
            log::info!("PCGamingWiki page {title} is another game than {}", query.title);
            return Ok(None);
        }
        Ok(Some(facts))
    }
}

/// Outcome of [`get_or_fetch`].
#[derive(Debug, Clone, PartialEq)]
pub struct PcgwLookup {
    pub facts: Option<PcgwFacts>,
    pub from_cache: bool,
}

fn fresh(entry: &PcgwEntry, now: i64) -> bool {
    let ttl = if entry.facts.is_some() { FOUND_TTL_MS } else { MISSING_TTL_MS };
    now - entry.fetched_at < ttl
}

/// The facts of `query`: the cache while fresh (30 days, 7 for "no page"),
/// else asked (`force`: asked anyway). When PCGamingWiki can't be reached
/// nothing is written and the error is returned.
pub fn get_or_fetch(paths: &DataPaths, http: &Http, client: &PcgwClient, query: &PcgwQuery, force: bool) -> Result<PcgwLookup> {
    let keys = query.keys();
    let Some(primary) = keys.first().cloned() else { return Ok(PcgwLookup { facts: None, from_cache: false }) };
    let now = now_ms();
    if !force {
        let file = PcgwFile::load(paths);
        if let Some(entry) = keys.iter().find_map(|k| file.entries.get(k)).filter(|e| fresh(e, now)) {
            return Ok(PcgwLookup { facts: entry.facts.clone(), from_cache: true });
        }
    }
    let facts = client.fetch(http, query)?;
    let entry = PcgwEntry { facts: facts.clone(), fetched_at: now_ms() };
    json_store::update(&paths.pcgw(), |file: &mut PcgwFile| {
        file.entries.insert(primary, entry);
        Ok(())
    })?;
    Ok(PcgwLookup { facts, from_cache: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made-up wikitext in the shape of a PCGamingWiki page (no wiki text
    /// is copied).
    const PAGE: &str = r#"{{Infobox game
|cover        = Example.jpg
|engines      =
{{Infobox game/row/engine|Clockwork|name=Clockwork 2}}
{{Infobox game/row/engine|Clockwork}}
|steam appid  = 4242
|steam appid side = 4243, 4244
|gogcom id    = 1999999999
|hltb         = 777
}}
'''Example Quest''' is a made-up game.
{{Fixbox|description=First fix|fix=Do the thing.}}
{{Fixbox|description=Second fix|fix=Do the other thing.}}
===Game data===
{{Game data/saves|Windows|{{p|appdata}}\ExampleQuest\Saves\<ref>{{Refcheck|user=Someone|date=2026-01-01}}</ref>|{{P|game}}\save\}}
{{Game data/saves|Steam|{{p|steam}}\userdata\{{p|uid}}\4242\remote\}}
{{Game data/saves|Linux|}}
{{Video
|ultrawidescreen           = hackable
|ultrawidescreen notes     = Use a [[Glossary:Mod|mod]]. {{cn}}
|4k ultra hd               = true
|120 fps                   = limited
|hdr                       = unknown
}}
{{Input
|controller support        = true
|full controller           = false
}}
{{Middleware
|anticheat        = Easy Anti-Cheat<ref>made up</ref>, BattlEye
|anticheat notes  = Kernel level.
}}
"#;

    #[test]
    fn facts_from_wikitext() {
        let f = parse_wikitext("Example Quest", Some(99), PAGE);
        assert_eq!(f.page, "Example Quest");
        assert_eq!(f.revid, Some(99));
        assert_eq!(f.steam_ids, ["4242", "4243", "4244"]);
        assert_eq!(f.gog_ids, ["1999999999"]);
        assert_eq!(f.hltb_id.as_deref(), Some("777"));
        assert_eq!(f.engines, ["Clockwork"]);
        assert_eq!(f.fixes, 2);
        assert_eq!((f.ultrawide, f.uhd_4k, f.fps_120, f.hdr), (Some(Support::Hackable), Some(Support::Yes), Some(Support::Limited), None));
        assert_eq!((f.controller, f.full_controller), (Some(Support::Yes), Some(Support::No)));
        assert_eq!(f.anticheat, ["Easy Anti-Cheat", "BattlEye"]);
        assert_eq!(f.saves.len(), 2, "a platform with no path is left out");
        assert_eq!(f.saves[0].platform, "Windows");
        assert_eq!(f.saves[0].paths, [r"%APPDATA%\ExampleQuest\Saves\", r"<game folder>\save\"]);
        assert_eq!(f.saves[1].paths, [r"<Steam folder>\userdata\<user id>\4242\remote\"]);
        assert_eq!(f.save_paths().unwrap().platform, "Windows");
    }

    #[test]
    fn empty_and_odd_pages() {
        let f = parse_wikitext("Nothing", None, "Just text, {{stub}}.");
        assert_eq!(f, PcgwFacts { page: "Nothing".into(), ..Default::default() });
        // An unclosed template is ignored, not a panic.
        let f = parse_wikitext("Broken", None, "{{Video\n|hdr = true\n");
        assert_eq!(f.hdr, None);
        // `{{video …}}` with a lowercase first letter is the same template.
        let f = parse_wikitext("Lower", None, "{{video\n|hdr = true\n}}");
        assert_eq!(f.hdr, Some(Support::Yes));
        // Similar names don't match.
        assert!(templates("{{Videos|hdr=true}}", "Video").is_empty());
    }

    /// `HERONITE_PCGW_SAMPLE=<action=parse answer>`: parses a real page
    /// saved locally (never committed: the wiki's text is CC BY-NC-SA).
    #[test]
    fn real_page_parses() {
        let Some(path) = std::env::var_os("HERONITE_PCGW_SAMPLE") else { return };
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let text = v.pointer("/parse/wikitext").and_then(Value::as_str).unwrap();
        let f = parse_wikitext(v.pointer("/parse/title").and_then(Value::as_str).unwrap(), None, text);
        eprintln!("{f:#?}");
        assert!(!f.steam_ids.is_empty() || !f.gog_ids.is_empty());
    }

    #[test]
    fn locations() {
        assert_eq!(page_from_location("https://www.pcgamingwiki.com/wiki/Half-Life_2").as_deref(), Some("Half-Life 2"));
        assert_eq!(page_from_location("/wiki/M%C3%B6rk_Borg?x=1").as_deref(), Some("Mörk Borg"));
        assert_eq!(page_from_location("https://www.pcgamingwiki.com/wiki/"), None);
        assert_eq!(page_from_location("https://www.pcgamingwiki.com/"), None);
    }
}
