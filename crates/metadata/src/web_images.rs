//! Web image search for the art picker: port of `local-library/web-images.ts`.
//!
//! Heronite drives DuckDuckGo through a hidden Chromium and reads the
//! `i.js` response over CDP. Here the same call is made directly: fetch the
//! results page for the `vqd` token, then `i.js`. When DuckDuckGo refuses
//! (403 / no token / no results) the search falls back to Bing's image HTML
//! (`a.iusc` metadata) and Google's image HTML (Playnite's regex).

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use heronite_i18n::tr;
use parking_lot::Mutex;
use serde_json::Value;

use crate::http::{Http, FIREFOX_UA};
use crate::types::{CoverFamily, WebImage};
use crate::util::re;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);

/// `asHttpUrl`: undoes the `=` / `&` escapes, keeps http(s) URLs.
fn as_http_url(value: &str) -> Option<String> {
    let decoded = value.replace("\\u003d", "=").replace("\\u0026", "&");
    let url = url::Url::parse(decoded.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn number(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::Number(n)) => n.as_f64().map(|f| f.max(0.0) as u32).unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<f64>().map(|f| f.max(0.0) as u32).unwrap_or(0),
        _ => 0,
    }
}

/// `mapImages`: `{image, thumbnail, width, height}` rows to results,
/// de-duplicated by image URL.
fn map_images(rows: impl IntoIterator<Item = (String, String, u32, u32)>) -> Vec<WebImage> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (image, thumb, width, height) in rows {
        let Some(image_url) = as_http_url(&image) else { continue };
        let thumb_url = as_http_url(&thumb).unwrap_or_else(|| image_url.clone());
        if !seen.insert(image_url.clone()) {
            continue;
        }
        out.push(WebImage { image_url, thumb_url, width, height });
    }
    out
}

/// Parses the body of DuckDuckGo's `i.js`.
pub fn parse_ddg_images(body: &Value) -> Vec<WebImage> {
    let rows = body.get("results").and_then(Value::as_array).into_iter().flatten().map(|r| {
        (
            r.get("image").and_then(Value::as_str).unwrap_or("").to_owned(),
            r.get("thumbnail").and_then(Value::as_str).unwrap_or("").to_owned(),
            number(r.get("width")),
            number(r.get("height")),
        )
    });
    map_images(rows.collect::<Vec<_>>())
}

/// `parseGoogleHtml` (the regex Playnite uses on the image results page).
pub fn parse_google_html(html: &str) -> Vec<WebImage> {
    let flat = html.replace("\r\n", "").replace(['\r', '\n'], "");
    let rx = re!(r#"\["(https://encrypted-[^,]+?)",\d+,\d+\],\["(http.+?)",(\d+),(\d+)\]"#);
    let rows: Vec<_> =
        rx.captures_iter(&flat).map(|c| (c[2].to_owned(), c[1].to_owned(), c[4].parse().unwrap_or(0), c[3].parse().unwrap_or(0))).collect();
    map_images(rows)
}

fn html_unescape(s: &str) -> String {
    let s = re!(r"&#x([0-9a-fA-F]+);").replace_all(s, |c: &regex::Captures| {
        u32::from_str_radix(&c[1], 16).ok().and_then(char::from_u32).map(String::from).unwrap_or_default()
    });
    let s = re!(r"&#([0-9]+);")
        .replace_all(&s, |c: &regex::Captures| c[1].parse::<u32>().ok().and_then(char::from_u32).map(String::from).unwrap_or_default());
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&amp;", "&")
}

/// Bing: every `<a class="iusc" m="{...}">` carries `murl` (image), `turl`
/// (thumbnail) and sometimes `ow`/`oh`.
pub fn parse_bing_html(html: &str) -> Vec<WebImage> {
    let tag = re!(r#"<a\s[^>]*class="iusc"[^>]*>"#);
    let attr = re!(r#"\sm="([^"]*)""#);
    let rows: Vec<_> = tag
        .find_iter(html)
        .filter_map(|t| {
            let raw = attr.captures(t.as_str())?.get(1)?.as_str().to_owned();
            let meta: Value = serde_json::from_str(&html_unescape(&raw)).ok()?;
            let w = number(meta.get("ow")).max(number(meta.get("width")));
            let h = number(meta.get("oh")).max(number(meta.get("height")));
            Some((
                meta.get("murl").and_then(Value::as_str).unwrap_or("").to_owned(),
                meta.get("turl").and_then(Value::as_str).unwrap_or("").to_owned(),
                w,
                h,
            ))
        })
        .collect();
    map_images(rows)
}

/// The `vqd` token in a DuckDuckGo page.
pub fn extract_vqd(html: &str) -> Option<String> {
    let patterns = [r#"vqd="([\d-]+)""#, r#"vqd=([\d-]+)&"#, r#"vqd='([\d-]+)'"#, r#"vqd=([\d-]+)"#];
    for p in patterns {
        let rx = regex::Regex::new(p).ok()?;
        if let Some(c) = rx.captures(html) {
            return Some(c[1].to_owned());
        }
    }
    None
}

fn deadline_secs(deadline: Instant) -> u64 {
    deadline.saturating_duration_since(Instant::now()).as_secs().clamp(1, 12)
}

/// A shape filter of the image searches, for a cover family (the tall one
/// has none: its search stays as it always was).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Square,
    Wide,
}

impl Shape {
    pub fn of(family: Option<CoverFamily>) -> Option<Self> {
        match family? {
            CoverFamily::Tall => None,
            CoverFamily::Square => Some(Self::Square),
            CoverFamily::Wide => Some(Self::Wide),
        }
    }

    /// DuckDuckGo's `layout:` filter.
    fn ddg(self) -> &'static str {
        match self {
            Self::Square => "layout:Square",
            Self::Wide => "layout:Wide",
        }
    }

    /// Bing's `qft` aspect filter.
    fn bing(self) -> &'static str {
        match self {
            Self::Square => "+filterui:aspect-square",
            Self::Wide => "+filterui:aspect-wide",
        }
    }

    /// Google's `tbs` image aspect ratio.
    fn google(self) -> &'static str {
        match self {
            Self::Square => "iar:s",
            Self::Wide => "iar:w",
        }
    }
}

/// DuckDuckGo's `f` filters (`size,color,type,layout,license` and more,
/// read by their name).
fn ddg_filters(transparent: bool, shape: Option<Shape>) -> String {
    match (transparent, shape) {
        (false, None) => ",,,,,".into(),
        (true, None) => ",type:transparent,,,,".into(),
        (t, Some(s)) => format!(",{},,{},,", if t { "type:transparent" } else { "" }, s.ddg()),
    }
}

fn ddg_images(http: &Http, query: &str, transparent: bool, shape: Option<Shape>, deadline: Instant) -> Result<Vec<WebImage>> {
    let base = &http.endpoints.duckduckgo;
    let mut params = vec![("ia", "images"), ("iax", "images"), ("q", query)];
    if transparent {
        params.push(("iaf", "type:transparent"));
    } else if let Some(s) = shape {
        params.push(("iaf", s.ddg()));
    }
    let page = http
        .get(format!("{base}/"))
        .query(&params)
        .header("User-Agent", FIREFOX_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        .timeout_secs(deadline_secs(deadline))
        .send_ok()?
        .text();
    let vqd = extract_vqd(&page).ok_or_else(|| anyhow!("DuckDuckGo did not return a vqd token"))?;
    let filters = ddg_filters(transparent, shape);
    let resp = http
        .get(format!("{base}/i.js"))
        .query(&[("l", "us-en"), ("o", "json"), ("q", query), ("vqd", &vqd), ("f", &filters), ("p", "1")])
        .header("User-Agent", FIREFOX_UA)
        .header("Referer", "https://duckduckgo.com/")
        .header("Accept", "application/json, text/javascript, */*; q=0.01")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("X-Requested-With", "XMLHttpRequest")
        .timeout_secs(deadline_secs(deadline))
        .send_ok()?;
    Ok(parse_ddg_images(&resp.json()?))
}

fn bing_images(http: &Http, query: &str, shape: Option<Shape>, deadline: Instant) -> Result<Vec<WebImage>> {
    let mut params = vec![("q", query), ("form", "HDRSC2")];
    if let Some(s) = shape {
        params.push(("qft", s.bing()));
    }
    let html = http
        .get(format!("{}/images/search", http.endpoints.bing))
        .query(&params)
        .header("User-Agent", FIREFOX_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        .timeout_secs(deadline_secs(deadline))
        .max_bytes(8 * 1024 * 1024)
        .send_ok()?
        .text();
    Ok(parse_bing_html(&html))
}

fn google_images(http: &Http, query: &str, transparent: bool, shape: Option<Shape>, deadline: Instant) -> Result<Vec<WebImage>> {
    let mut params = vec![("tbm", "isch"), ("client", "firefox-b-d"), ("source", "lnt"), ("q", query)];
    let tbs = match (transparent, shape) {
        (true, Some(s)) => Some(format!("ic:trans,{}", s.google())),
        (true, None) => Some("ic:trans".to_owned()),
        (false, Some(s)) => Some(s.google().to_owned()),
        (false, None) => None,
    };
    if let Some(tbs) = &tbs {
        params.push(("tbs", tbs));
    }
    let html = http
        .get(format!("{}/search", http.endpoints.google))
        .query(&params)
        .header("User-Agent", FIREFOX_UA)
        .header("Accept-Language", "en-US,en;q=0.9")
        // Skips the cookie-consent redirect for EU visitors.
        .header("Cookie", "CONSENT=YES+cb; SOCS=CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg")
        .timeout_secs(deadline_secs(deadline))
        .max_bytes(8 * 1024 * 1024)
        .send_ok()?
        .text();
    Ok(parse_google_html(&html))
}

fn fetch_web_images(http: &Http, query: &str, transparent: bool, shape: Option<Shape>, deadline: Instant) -> Result<Vec<WebImage>> {
    let mut last_error: Option<anyhow::Error> = None;
    let mut attempts: Vec<(&str, Box<dyn Fn() -> Result<Vec<WebImage>> + '_>)> = vec![
        ("DuckDuckGo", Box::new(|| ddg_images(http, query, transparent, shape, deadline))),
        ("Bing", Box::new(|| bing_images(http, query, shape, deadline))),
        ("Google", Box::new(|| google_images(http, query, transparent, shape, deadline))),
    ];
    for (name, attempt) in attempts.drain(..) {
        if Instant::now() >= deadline {
            bail!(tr!("Image search timed out"));
        }
        match attempt() {
            Ok(images) if !images.is_empty() => return Ok(images),
            Ok(_) => log::info!("Web image search: {name} returned no images"),
            Err(e) => {
                log::warn!("Web image search: {name} failed: {e}");
                last_error = Some(e);
            }
        }
    }
    match last_error {
        Some(e) => Err(e),
        None => Ok(Vec::new()),
    }
}

/// Serializes searches, like the TS `searchChain`.
static SEARCH_LOCK: Mutex<()> = Mutex::new(());

/// `searchCollectionWebImages`: images for `query`. With `transparent`, a
/// failed search is retried once without the filter. Errors read
/// "Could not search images. Try another term.", like Heronite's.
pub fn search_web_images(http: &Http, query: &str, transparent: bool) -> Result<Vec<WebImage>> {
    search_web_images_shaped(http, query, transparent, None)
}

/// [`search_web_images`] favouring pictures of `family`'s shape: each
/// engine's aspect filter for the square and wide covers (none for the tall
/// one, whose search is as it always was). A shaped search that finds
/// nothing is tried once without the shape.
pub fn search_web_images_shaped(http: &Http, query: &str, transparent: bool, family: Option<CoverFamily>) -> Result<Vec<WebImage>> {
    let query = re!(r"\s+").replace_all(query, " ").trim().to_owned();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let shape = Shape::of(family);
    let _serial = SEARCH_LOCK.lock();
    log::info!("Web image search starting for \"{query}\"");
    let mut result = fetch_web_images(http, &query, transparent, shape, Instant::now() + SEARCH_TIMEOUT);
    if shape.is_some() && result.as_ref().is_ok_and(Vec::is_empty) {
        result = fetch_web_images(http, &query, transparent, None, Instant::now() + SEARCH_TIMEOUT);
    }
    if result.is_err() && transparent {
        log::warn!("Transparent web image search failed, retrying without filter");
        result = fetch_web_images(http, &query, false, shape, Instant::now() + SEARCH_TIMEOUT);
    }
    result.map_err(|e| {
        log::warn!("Web image search failed: {e}");
        anyhow!(tr!("Could not search images. Try another term."))
    })
}

/// The two queries Playnite's image picker offers for a game.
pub fn cover_query(title: &str) -> String {
    format!("\"{title}\" cover")
}

pub fn wallpaper_query(title: &str) -> String {
    format!("\"{title}\" wallpaper")
}

/// The game's title in a query of the picker: the quoted part of
/// `"Portal 2" cover`, else the query without a last "cover" or "wallpaper".
pub fn title_of_query(query: &str) -> String {
    let q = query.trim();
    if let Some((_, rest)) = q.split_once('"') {
        if let Some((title, _)) = rest.split_once('"') {
            if !title.trim().is_empty() {
                return title.trim().to_owned();
            }
        }
    }
    for suffix in [" cover", " wallpaper"] {
        let cut = q.len().saturating_sub(suffix.len());
        if q.is_char_boundary(cut) && q[cut..].eq_ignore_ascii_case(suffix) {
            return q[..cut].trim().to_owned();
        }
    }
    q.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn title_of_query_takes_the_quoted_title_or_drops_the_suffix() {
        assert_eq!(title_of_query(&cover_query("007 First Light")), "007 First Light");
        assert_eq!(title_of_query(&wallpaper_query("Portal 2")), "Portal 2");
        assert_eq!(title_of_query("  Hades II Cover "), "Hades II");
        assert_eq!(title_of_query("Hades II"), "Hades II");
        assert_eq!(title_of_query("\"\" cover"), "\"\"");
        assert_eq!(title_of_query("Pokémon wallpaper"), "Pokémon");
    }

    #[test]
    fn parses_recorded_duckduckgo_response() {
        let body: Value = serde_json::from_str(&fixture("ddg_images_portal2_cover.json")).unwrap();
        let images = parse_ddg_images(&body);
        assert_eq!(images.len(), 6);
        assert_eq!(images[0].image_url, "https://combineoverwiki.net/images/8/89/Portal_2_cover.jpg");
        assert!(images[0].thumb_url.starts_with("https://tse3.mm.bing.net/th/id/OIP."));
        assert_eq!((images[0].width, images[0].height), (899, 1129));
        assert!(images.iter().all(|i| i.width > 0 && i.height > 0));
    }

    #[test]
    fn dedupes_and_drops_non_http() {
        let body = serde_json::json!({"results": [
            {"image": "https://a/x.jpg", "thumbnail": "https://a/t.jpg", "width": 10, "height": "20"},
            {"image": "https://a/x.jpg", "thumbnail": "https://a/t2.jpg"},
            {"image": "ftp://a/y.jpg"}, {"image": "data:image/png;base64,xx"}, {"image": ""},
            {"image": "https://a/z.png", "thumbnail": "javascript:1"}
        ]});
        let images = parse_ddg_images(&body);
        assert_eq!(images.len(), 2);
        assert_eq!((images[0].width, images[0].height), (10, 20));
        assert_eq!(images[1].thumb_url, images[1].image_url);
    }

    #[test]
    fn parses_recorded_bing_markup() {
        let images = parse_bing_html(&fixture("bing_images_portal2.html"));
        assert_eq!(images.len(), 5);
        assert!(images[0].image_url.starts_with("https://i.gadgets360cdn.com/products/large/"));
        assert!(images[0].thumb_url.starts_with("https://ts2.mm.bing.net/th?id=OIP."));
        // The entities of the `m` attribute were decoded (the `&` of the thumbnail query).
        assert!(images[0].thumb_url.contains("&pid=15.1"));
    }

    #[test]
    fn parses_google_markup_like_playnite() {
        let html = r#"junk ["https://encrypted-tbn0.gstatic.com/images?q=tbn:AAA=",225,225],["https://example.com/a.jpg?x=1&y=2",1500,1000]
,["x"] ["https://encrypted-tbn1.gstatic.com/images?q=tbn:BBB",100,100],["https://example.com/b.png",800,600]"#;
        let images = parse_google_html(html);
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].image_url, "https://example.com/a.jpg?x=1&y=2");
        assert_eq!(images[0].thumb_url, "https://encrypted-tbn0.gstatic.com/images?q=tbn:AAA=");
        // regex groups: (thumb w,h) then (image h?,w?): Playnite reads group 3 as height, 4 as width.
        assert_eq!((images[0].width, images[0].height), (1000, 1500));
    }

    #[test]
    fn a_shape_adds_each_engines_aspect_filter() {
        assert_eq!(Shape::of(Some(CoverFamily::Tall)), None);
        assert_eq!(Shape::of(None), None);
        assert_eq!(ddg_filters(false, None), ",,,,,");
        assert_eq!(ddg_filters(true, None), ",type:transparent,,,,");
        assert_eq!(ddg_filters(false, Shape::of(Some(CoverFamily::Square))), ",,,layout:Square,,");
        assert_eq!(ddg_filters(true, Some(Shape::Wide)), ",type:transparent,,layout:Wide,,");
        assert_eq!((Shape::Square.bing(), Shape::Wide.google()), ("+filterui:aspect-square", "iar:w"));
    }

    #[test]
    fn vqd_token() {
        let html = fixture_html();
        assert_eq!(extract_vqd(&html).as_deref(), Some("4-1234"));
        assert_eq!(extract_vqd("<a href=\"/?vqd=4-9&x=1\">").as_deref(), Some("4-9"));
        assert_eq!(extract_vqd("nothing"), None);
        assert_eq!(cover_query("Portal 2"), "\"Portal 2\" cover");
        assert_eq!(wallpaper_query("Portal 2"), "\"Portal 2\" wallpaper");
    }

    fn fixture_html() -> String {
        r#"<script>nrje('/d.js?q=x&vqd="4-1234"&p=1')</script>"#.to_owned()
    }
}
