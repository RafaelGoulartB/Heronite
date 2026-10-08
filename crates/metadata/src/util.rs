//! Text and file helpers shared by the providers: ports of `metadata/blank.ts`,
//! `metadata/match.ts`, `playnite/mapper.ts#normalizeTitle` and the image
//! helpers of `protocol.ts`.

use std::collections::HashSet;

use heronite_data::models::CollectionGameMetadata;

macro_rules! re {
    ($pat:expr) => {{
        static RE: ::std::sync::OnceLock<::regex::Regex> = ::std::sync::OnceLock::new();
        RE.get_or_init(|| ::regex::Regex::new($pat).expect("valid regex"))
    }};
}
pub(crate) use re;

/// `blankMetadata`.
pub fn blank_metadata(app_name: &str, runner: &str) -> CollectionGameMetadata {
    CollectionGameMetadata {
        app_name: app_name.to_owned(),
        runner: runner.to_owned(),
        ..Default::default()
    }
}

/// `uniqueNames`: trims, drops empties, de-duplicates case-insensitively and
/// stops at `max` entries.
pub fn unique_names<'a, I, S>(values: I, max: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<Option<&'a str>>,
{
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let Some(value) = value.into() else { continue };
        let name = value.trim();
        if name.is_empty() {
            continue;
        }
        if !seen.insert(name.to_lowercase()) {
            continue;
        }
        out.push(name.to_owned());
        if out.len() >= max {
            break;
        }
    }
    out
}

/// `uniqueNames(values)` with the default cap of 12.
pub fn unique_names12<'a, I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<Option<&'a str>>,
{
    unique_names(values, 12)
}

/// `clipText`: collapses whitespace; `None` when empty; over-long text is cut
/// to `max - 1` characters plus an ellipsis.
pub fn clip_text(value: Option<&str>, max: usize) -> Option<String> {
    let text = re!(r"\s+").replace_all(value.unwrap_or(""), " ");
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > max {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        Some(format!("{}…", cut.trim()))
    } else {
        Some(text.to_owned())
    }
}

/// `clipText(value)` with the default cap of 2000.
pub fn clip(value: Option<&str>) -> Option<String> {
    clip_text(value, 2000)
}

/// `stripHtml`.
pub fn strip_html(value: Option<&str>) -> String {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return String::new();
    };
    let s = re!(r"(?i)<br\s*/?>").replace_all(value, "\n");
    let s = re!(r"(?i)</p>").replace_all(&s, "\n");
    let s = re!(r"<[^>]+>").replace_all(&s, " ");
    let s = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let s = re!(r"\s+\n").replace_all(&s, "\n");
    let s = re!(r"\n{3,}").replace_all(&s, "\n\n");
    let s = re!(r"[ \t]+").replace_all(&s, " ");
    s.trim().to_owned()
}

/// `normalizeTitle` of `playnite/mapper.ts`.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut gap = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// `titleQueries` of `metadata/match.ts`: the title as is, without an edition
/// suffix, with `" - "` turned into `": "`, and without a demo/beta marker.
pub fn title_queries(title: &str) -> Vec<String> {
    let cleaned = re!(r"\s+").replace_all(&title.replace('_', " "), " ").trim().to_owned();
    let no_edition = re!(r"(?i)\s*[-–:]\s*(definitive|ultimate|complete|goty|deluxe|gold|standard|windows|linux).*$")
        .replace(&cleaned, "")
        .into_owned();
    let colon = re!(r"\s+-\s+").replace_all(&cleaned, ": ").into_owned();
    let no_marker = re!(r"(?i)\s*[-–:]?\s*[(\[]?\s*\b(?:(?:multiplayer|open|closed|public|free)\s+)?(?:demo|beta|alpha|playtest|trial|preview|early\s+access)\b\s*[)\]]?\s*$")
        .replace(&no_edition, "")
        .trim()
        .to_owned();
    let mut seen = HashSet::new();
    [cleaned, no_edition, colon, no_marker]
        .into_iter()
        .filter(|q| seen.insert(q.clone()))
        .filter(|q| q.chars().count() >= 2)
        .collect()
}

/// `scoreTitle`.
pub fn score_title(query: &str, name: &str) -> i32 {
    let wanted = normalize_title(query);
    let got = normalize_title(name);
    if wanted.is_empty() || got.is_empty() {
        return 0;
    }
    if wanted == got {
        return 100;
    }
    if got.starts_with(&wanted) || wanted.starts_with(&got) {
        let diff = (got.chars().count() as i32 - wanted.chars().count() as i32).abs();
        return 60.max(90 - diff);
    }
    if got.contains(&wanted) || wanted.contains(&got) {
        return 50;
    }
    let parts: Vec<&str> = wanted.split(' ').filter(|p| !p.is_empty()).collect();
    let hits = parts.iter().filter(|p| got.contains(**p)).count();
    if !parts.is_empty() && hits as f64 / parts.len() as f64 >= 0.7 {
        return 40;
    }
    0
}

/// `pickBestMatch` (first best score wins ties).
pub fn pick_best_match<'a, T>(
    title: &str,
    items: &'a [T],
    name_of: impl Fn(&T) -> String,
    min_score: i32,
) -> Option<&'a T> {
    let mut best: Option<(&T, i32)> = None;
    for item in items {
        let score = score_title(title, &name_of(item));
        if score < min_score {
            continue;
        }
        if best.map_or(true, |(_, s)| score > s) {
            best = Some((item, score));
        }
    }
    best.map(|(i, _)| i)
}

/// Default `minScore` of `pickBestMatch`.
pub const MIN_MATCH_SCORE: i32 = 45;

// ---------------------------------------------------------------------------
// Files / images
// ---------------------------------------------------------------------------

/// `folderName`: `${runner}_${appName}` with runs of unusual characters
/// replaced by `_`, at most 120 characters.
pub fn folder_name(runner: &str, app_name: &str) -> String {
    let joined = format!("{runner}_{app_name}");
    let replaced = re!(r"[^a-zA-Z0-9._-]+").replace_all(&joined, "_");
    replaced.chars().take(120).collect()
}

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `localArtUrl(kind, ...segments)`.
pub fn local_art_url(kind: &str, segments: &[&str]) -> String {
    let rest: Vec<String> = segments.iter().map(|s| encode_uri_component(s)).collect();
    format!("localart://{kind}/{}", rest.join("/"))
}

/// Image type from the first bytes. `extended` also knows gif and avif
/// (`art.ts`); the narrower form is `metadata/images.ts`.
pub fn sniff_image_ext(bytes: &[u8], extended: bool) -> Option<&'static str> {
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        return Some("jpg");
    }
    if bytes.len() >= 8 && bytes[..4] == [0x89, 0x50, 0x4e, 0x47] {
        return Some("png");
    }
    if extended && bytes.len() >= 6 && bytes[..3] == *b"GIF" {
        return Some("gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if extended && bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some("avif");
    }
    None
}

pub const ALLOWED_EXT: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "avif"];

/// Lowercase extension without the dot; `jpeg` becomes `jpg`.
pub fn normalize_ext(ext: &str) -> String {
    let v = ext.trim_start_matches('.').to_lowercase();
    if v == "jpeg" {
        "jpg".into()
    } else {
        v
    }
}

/// Current time as JS `Date.now()`.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// `new Date().toISOString()`.
pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_queries_match_the_ts() {
        assert_eq!(
            title_queries("The_Witcher 3 - Definitive Edition"),
            ["The Witcher 3 - Definitive Edition", "The Witcher 3", "The Witcher 3: Definitive Edition"]
        );
        assert_eq!(title_queries("Foo Demo"), ["Foo Demo", "Foo"]);
        assert_eq!(title_queries("Foo (Open Beta)"), ["Foo (Open Beta)", "Foo"]);
        assert_eq!(title_queries("A"), Vec::<String>::new());
    }

    #[test]
    fn scoring() {
        assert_eq!(score_title("Portal 2", "portal-2"), 100);
        assert_eq!(score_title("Portal", "Portal 2"), 90 - 2);
        assert_eq!(score_title("Portal 2", "The Portal 2 Thing"), 50);
        assert_eq!(score_title("one two three four", "four three xx two"), 40);
        assert_eq!(score_title("abc", "zzz"), 0);
        let items = ["Portal Maze 2", "Portal 2: In Motion", "Portal 2"];
        let best = pick_best_match("Portal 2", &items, |s| s.to_string(), MIN_MATCH_SCORE);
        assert_eq!(best, Some(&"Portal 2"));
    }

    #[test]
    fn unique_and_clip() {
        assert_eq!(unique_names12([Some(" A "), Some("a"), None, Some(""), Some("B")]), ["A", "B"]);
        assert_eq!(unique_names(["a", "b", "c"], 2), ["a", "b"]);
        assert_eq!(clip(Some("  a \n\n b ")).as_deref(), Some("a b"));
        assert_eq!(clip(Some("   ")), None);
        let long = "x".repeat(2500);
        let c = clip(Some(&long)).unwrap();
        assert_eq!(c.chars().count(), 2000);
        assert!(c.ends_with('…'));
    }

    #[test]
    fn strip_html_matches_ts() {
        assert_eq!(
            strip_html(Some("<p>Hello&nbsp;<b>world</b> &amp; co</p><p>Line2<br/>Line3</p>")),
            "Hello world & co\n Line2\nLine3" // the space of the opening <p> survives, as in the TS
        );
    }

    #[test]
    fn art_helpers() {
        assert_eq!(folder_name("sideload", "steam_12 3"), "sideload_steam_12_3");
        assert_eq!(local_art_url("metadata-art", &["a b", "cover.jpg"]), "localart://metadata-art/a%20b/cover.jpg");
        assert_eq!(sniff_image_ext(&[0xff, 0xd8, 0xff], false), Some("jpg"));
        assert_eq!(sniff_image_ext(b"GIF89a", false), None);
        assert_eq!(sniff_image_ext(b"GIF89a", true), Some("gif"));
    }
}
