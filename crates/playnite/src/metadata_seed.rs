//! `seedPlayniteMetadata` and the pieces it needs (`metadata/blank.ts`,
//! `metadata/merge.ts` with a single Playnite candidate, `metadata/images.ts`
//! for local files): fills `local_library/metadata.json` from the Playnite
//! game and copies its cover/background into `metadata-art/`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use heronite_data::models::{CollectionGameMetadata, MetadataFile};
use heronite_data::settings::METADATA_FIELDS;
use heronite_data::HeroicPaths;
use indexmap::IndexMap;
use regex::Regex;

use crate::reader::PlayniteGame;

const ALLOWED_EXT: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "avif"];
const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;

/// `blankMetadata`.
pub fn blank_metadata(app_name: &str, runner: &str) -> CollectionGameMetadata {
    CollectionGameMetadata { app_name: app_name.to_owned(), runner: runner.to_owned(), ..Default::default() }
}

/// `uniqueNames`.
pub fn unique_names(values: &[String], max: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let name = value.trim();
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            continue;
        }
        out.push(name.to_owned());
        if out.len() >= max {
            break;
        }
    }
    out
}

/// `clipText(value, 2000)`: whitespace collapsed, clipped to `max` UTF-16
/// units with an ellipsis.
pub fn clip_text(value: Option<&str>, max: usize) -> Option<String> {
    let text = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    if text.encode_utf16().count() <= max {
        return Some(text);
    }
    let mut clipped = String::new();
    let mut units = 0;
    for c in text.chars() {
        units += c.len_utf16();
        if units > max - 1 {
            break;
        }
        clipped.push(c);
    }
    Some(format!("{}…", clipped.trim()))
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

/// `stripHtml`.
pub fn strip_html(value: Option<&str>) -> String {
    let Some(value) = value.filter(|v| !v.is_empty()) else { return String::new() };
    let s = re(r"(?i)<br\s*/?>").replace_all(value, "\n").into_owned();
    let s = re(r"(?i)</p>").replace_all(&s, "\n").into_owned();
    let s = re(r"<[^>]+>").replace_all(&s, " ").into_owned();
    let s = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let s = re(r"\s+\n").replace_all(&s, "\n").into_owned();
    let s = re(r"\n{3,}").replace_all(&s, "\n\n").into_owned();
    let s = re(r"[ \t]+").replace_all(&s, " ").into_owned();
    s.trim().to_owned()
}

/// `playniteFilePath`.
fn playnite_file_path(library_path: &Path, relative: Option<&str>) -> Option<PathBuf> {
    let relative = relative.filter(|r| !r.is_empty())?;
    let slashed = relative.replace('\\', "/");
    let cleaned = re(r"(?i)^\{PlayniteDir\}/?").replace(&slashed, "").into_owned();
    let candidates = [
        library_path.join("files").join(&cleaned),
        library_path.join(&cleaned),
        if cleaned.starts_with("files/") {
            library_path.join(&cleaned)
        } else {
            library_path.join("files").join(&cleaned)
        },
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// `usablePlayniteFile`: the file, if it exists and is at most 2 MiB.
pub fn usable_playnite_file(library_path: &Path, relative: Option<&str>) -> Option<PathBuf> {
    let path = playnite_file_path(library_path, relative)?;
    let meta = fs::metadata(&path).ok()?;
    if meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    Some(path)
}

/// `metadataFromPlayniteGame`.
pub fn metadata_from_playnite_game(
    app_name: &str,
    runner: &str,
    game: &PlayniteGame,
    library_path: &Path,
) -> CollectionGameMetadata {
    let cover = usable_playnite_file(library_path, game.cover_image.as_deref());
    let hero = usable_playnite_file(library_path, game.background_image.as_deref());
    let mut m = blank_metadata(app_name, runner);
    m.title = Some(game.name.clone());
    m.description = clip_text(Some(&strip_html(game.description.as_deref())), 2000);
    m.release_date = game.release_date.clone();
    m.developers = unique_names(&game.developers, 12);
    m.publishers = unique_names(&game.publishers, 12);
    m.genres = unique_names(&game.genres, 12);
    m.themes = unique_names(&game.tags, 12);
    m.platforms = unique_names(&game.platforms, 12);
    m.series = game.series.clone();
    m.features = unique_names(&game.features, 12);
    m.cover_url = cover.map(|p| p.to_string_lossy().into_owned());
    m.hero_url = hero.map(|p| p.to_string_lossy().into_owned());
    m
}

// ---------------------------------------------------------------------------
// merge (one candidate: "playnite")
// ---------------------------------------------------------------------------

fn has_field_value(m: &CollectionGameMetadata, field: &str) -> bool {
    match field {
        "description" => m.description.as_deref().is_some_and(|s| !s.is_empty()),
        "releaseDate" => m.release_date.as_deref().is_some_and(|s| !s.is_empty()),
        "series" => m.series.as_deref().is_some_and(|s| !s.is_empty()),
        "criticScore" => m.critic_score.is_some(),
        "cover" => m.cover_url.as_deref().is_some_and(|s| !s.is_empty()),
        "hero" => m.hero_url.as_deref().is_some_and(|s| !s.is_empty()),
        "developers" => !m.developers.is_empty(),
        "publishers" => !m.publishers.is_empty(),
        "genres" => !m.genres.is_empty(),
        "themes" => !m.themes.is_empty(),
        "gameModes" => !m.game_modes.is_empty(),
        "platforms" => !m.platforms.is_empty(),
        "features" => !m.features.is_empty(),
        _ => false,
    }
}

/// `writeField` from the candidate (only called when it has a value).
fn copy_field(target: &mut CollectionGameMetadata, from: &CollectionGameMetadata, field: &str) -> bool {
    match field {
        "description" => target.description = from.description.clone(),
        "releaseDate" => target.release_date = from.release_date.clone(),
        "series" => target.series = from.series.clone(),
        "criticScore" => target.critic_score = from.critic_score,
        "cover" => target.cover_url = from.cover_url.clone(),
        "hero" => target.hero_url = from.hero_url.clone(),
        "developers" => target.developers = from.developers.clone(),
        "publishers" => target.publishers = from.publishers.clone(),
        "genres" => target.genres = from.genres.clone(),
        "themes" => target.themes = from.themes.clone(),
        "gameModes" => target.game_modes = from.game_modes.clone(),
        "platforms" => target.platforms = from.platforms.clone(),
        "features" => target.features = from.features.clone(),
        _ => return false,
    }
    true
}

/// `mergeMetadata` with `candidates = [{source: 'playnite', ...}]`,
/// `overwriteExisting = false`, `preserveManual = true`.
pub fn merge_playnite_candidate(
    app_name: &str,
    runner: &str,
    current: Option<&CollectionGameMetadata>,
    candidate: &CollectionGameMetadata,
    priority: &IndexMap<String, Vec<String>>,
) -> CollectionGameMetadata {
    let mut next = current.cloned().unwrap_or_else(|| blank_metadata(app_name, runner));
    for field in METADATA_FIELDS {
        let locked_manual = current.and_then(|c| c.field_sources.get(field)).is_some_and(|s| s == "manual");
        if locked_manual {
            continue;
        }
        if current.is_some_and(|c| has_field_value(c, field)) {
            continue;
        }
        let order = priority.get(field).map(Vec::as_slice).unwrap_or(&[]);
        for source in order {
            if source == "manual" || source != "playnite" {
                continue;
            }
            if !has_field_value(candidate, field) {
                continue;
            }
            if copy_field(&mut next, candidate, field) {
                next.field_sources.insert(field.to_owned(), "playnite".into());
            }
            break;
        }
    }
    if candidate.title.as_deref().is_some_and(|t| !t.is_empty()) && next.title.as_deref().is_none_or(str::is_empty) {
        next.title = candidate.title.clone();
    }
    if !candidate.websites.is_empty() && next.websites.is_empty() {
        next.websites = candidate.websites.clone();
    }
    next.app_name = app_name.to_owned();
    next.runner = runner.to_owned();
    next.fetched_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
    next.fetch_error = None;
    next
}

// ---------------------------------------------------------------------------
// images
// ---------------------------------------------------------------------------

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `localArtUrl('metadata-art', folder, file)`.
pub fn local_art_url(kind: &str, segments: &[&str]) -> String {
    let encoded: Vec<String> = segments.iter().map(|s| encode_uri_component(s)).collect();
    format!("localart://{kind}/{}", encoded.join("/"))
}

/// `folderName` of `metadata/images.ts`.
pub fn art_folder_name(runner: &str, app_name: &str) -> String {
    let raw = format!("{runner}_{app_name}");
    let mut out = String::new();
    let mut in_bad_run = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            in_bad_run = false;
        } else if !in_bad_run {
            out.push('_');
            in_bad_run = true;
        }
    }
    out.chars().take(120).collect()
}

/// `path.extname(p).replace('.', '').toLowerCase() || 'jpg'`.
fn ext_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].replacen('.', "", 1).to_lowercase(),
        _ => String::new(),
    }
}

/// `materializeMetadataImage` without downloading (`download = false`).
fn materialize_image(
    paths: &HeroicPaths,
    runner: &str,
    app_name: &str,
    kind: &str,
    source: Option<&str>,
) -> Result<Option<String>> {
    let Some(source) = source.map(str::trim).filter(|s| !s.is_empty()) else { return Ok(None) };
    if source.starts_with("localart:") {
        return Ok(Some(source.to_owned()));
    }
    let is_local = !source.starts_with("http");
    if !is_local {
        return Ok(Some(source.to_owned()));
    }
    if !Path::new(source).exists() {
        return Ok(None);
    }
    let mut ext = ext_of(source);
    if ext.is_empty() {
        ext = "jpg".into();
    }
    if !ALLOWED_EXT.contains(&ext.as_str()) {
        return Ok(None);
    }
    let ext = if ext == "jpeg" { "jpg".to_owned() } else { ext };
    let folder = art_folder_name(runner, app_name);
    let dir = paths.metadata_art_dir().join(&folder);
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let file_name = format!("{kind}.{ext}");
    fs::copy(source, dir.join(&file_name)).with_context(|| format!("copying {source}"))?;
    Ok(Some(local_art_url("metadata-art", &[&folder, &file_name])))
}

/// `seedPlayniteMetadata` against a loaded `metadata.json`. Untouched when the
/// game already has field sources recorded.
pub fn seed_playnite_metadata(
    paths: &HeroicPaths,
    file: &mut MetadataFile,
    priority: &IndexMap<String, Vec<String>>,
    app_name: &str,
    runner: &str,
    game: &PlayniteGame,
    library_path: &Path,
) -> Result<()> {
    let runner = if runner == "zoom" { "sideload" } else { runner };
    let key = heronite_data::models::game_key(runner, app_name);
    let existing = file.games.get(&key);
    if existing.is_some_and(|e| !e.field_sources.is_empty()) {
        return Ok(());
    }
    let candidate = metadata_from_playnite_game(app_name, runner, game, library_path);
    let mut merged = merge_playnite_candidate(app_name, runner, existing, &candidate, priority);
    merged.cover_url = materialize_image(paths, runner, app_name, "cover", merged.cover_url.as_deref())?;
    merged.hero_url = materialize_image(paths, runner, app_name, "hero", merged.hero_url.as_deref())?;
    file.games.insert(key, merged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_helpers_follow_the_ts() {
        assert_eq!(strip_html(Some("<p>Hello&nbsp;<b>World</b></p><br/>x &amp; y")), "Hello World\n\nx & y".replace("\n\n", "\n"));
        assert_eq!(clip_text(Some("  a \n b  "), 2000).as_deref(), Some("a b"));
        assert_eq!(clip_text(Some("   "), 2000), None);
        let long = "word ".repeat(1000);
        let clipped = clip_text(Some(&long), 2000).unwrap();
        assert!(clipped.ends_with('…'));
        assert!(clipped.encode_utf16().count() <= 2000);
        assert_eq!(unique_names(&["A".into(), "a ".into(), " ".into(), "B".into()], 12), ["A", "B"]);
        assert_eq!(unique_names(&["a".into(), "b".into(), "c".into()], 2), ["a", "b"]);
    }

    #[test]
    fn uri_and_folder_names() {
        assert_eq!(
            local_art_url("metadata-art", &["sideload_playnite_a b", "cover.jpg"]),
            "localart://metadata-art/sideload_playnite_a%20b/cover.jpg"
        );
        assert_eq!(art_folder_name("sideload", "a b!!c/é"), "sideload_a_b_c_");
        assert_eq!(ext_of("/x/.hidden"), "");
        assert_eq!(ext_of("/x/a.PNG"), "png");
    }
}
