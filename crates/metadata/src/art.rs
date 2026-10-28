//! Custom cover/hero overrides (`art.json` + `local_library/art/`): port of
//! `src/local-library/art.ts`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use heronite_data::json_store;
use heronite_data::models::{ArtFile, CollectionGameArt};
use heronite_data::paths::{percent_decode, DataPaths};
use heronite_i18n::tr;

use crate::http::{redact_url, Http};
use crate::types::ArtKind;
use crate::util::{folder_name, local_art_url, normalize_ext, now_ms, sniff_image_ext, ALLOWED_EXT};

const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const DOWNLOAD_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

/// `collectionArtKey`.
pub fn art_key(runner: &str, app_name: &str) -> String {
    format!("{runner}:{app_name}")
}

/// `fileFromUrl`: the existing file behind a `localart:` or `file:` URL.
fn file_from_url(paths: &DataPaths, url: Option<&str>) -> Option<PathBuf> {
    let url = url.filter(|u| !u.is_empty())?;
    if url.starts_with("localart:") {
        return paths.resolve_localart(url, None).filter(|p| p.exists());
    }
    if let Some(rest) = url.strip_prefix("file://") {
        return Some(PathBuf::from(percent_decode(rest)));
    }
    url.strip_prefix("file:").map(|rest| PathBuf::from(percent_decode(rest)))
}

/// `displayUrl`: `localart://art/...` for a file inside the art folder, else
/// the bare file name under it.
fn display_url(paths: &DataPaths, file: &Path) -> String {
    let root = paths.art_dir();
    match file.strip_prefix(&root) {
        Ok(rel) if !rel.as_os_str().is_empty() => {
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            local_art_url("art", &refs)
        }
        _ => local_art_url("art", &[&file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()]),
    }
}

fn is_managed(paths: &DataPaths, file: &Path) -> bool {
    file.starts_with(paths.art_dir())
}

fn remove_managed(paths: &DataPaths, url: Option<&str>) {
    let Some(file) = file_from_url(paths, url) else { return };
    if !is_managed(paths, &file) || !file.exists() {
        return;
    }
    if let Err(e) = fs::remove_file(&file) {
        log::warn!("Could not remove Collection art {}: {e}", file.display());
    }
}

/// `hydrate`: keeps only art whose file exists, with normalized URLs.
fn hydrate(paths: &DataPaths, art: Option<&CollectionGameArt>) -> CollectionGameArt {
    let mut next = CollectionGameArt::default();
    let Some(art) = art else { return next };
    if let Some(p) = file_from_url(paths, art.cover_url.as_deref()).filter(|p| p.exists()) {
        next.cover_url = Some(display_url(paths, &p));
    }
    if let Some(p) = file_from_url(paths, art.hero_url.as_deref()).filter(|p| p.exists()) {
        next.hero_url = Some(display_url(paths, &p));
    }
    next
}

/// `getAllCollectionArt`.
pub fn get_all_collection_art(paths: &DataPaths) -> Result<indexmap::IndexMap<String, CollectionGameArt>> {
    let file: ArtFile = json_store::read(&paths.art())?;
    Ok(file.games.iter().map(|(k, a)| (k.clone(), hydrate(paths, Some(a)))).collect())
}

/// `getCollectionGameArt`.
pub fn get_collection_game_art(paths: &DataPaths, runner: &str, app_name: &str) -> Result<CollectionGameArt> {
    let file: ArtFile = json_store::read(&paths.art())?;
    Ok(hydrate(paths, file.games.get(&art_key(runner, app_name))))
}

/// `writeArt`: stores `art` (hydrated) or drops the entry when it is empty.
/// Fields of the entry this crate does not model are kept.
fn write_art(paths: &DataPaths, runner: &str, app_name: &str, art: &CollectionGameArt) -> Result<CollectionGameArt> {
    let key = art_key(runner, app_name);
    let cleaned = hydrate(paths, Some(art));
    json_store::update::<ArtFile, _>(&paths.art(), |file| {
        if cleaned.cover_url.is_none() && cleaned.hero_url.is_none() {
            file.games.shift_remove(&key);
        } else {
            let mut entry = cleaned.clone();
            if let Some(old) = file.games.get(&key) {
                entry.extra = old.extra.clone();
            }
            file.games.insert(key.clone(), entry);
        }
        Ok(())
    })?;
    Ok(cleaned)
}

fn dest_file(paths: &DataPaths, runner: &str, app_name: &str, kind: ArtKind, ext: &str) -> Result<PathBuf> {
    let dir = paths.art_dir().join(folder_name(runner, app_name));
    fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{}-{}.{ext}", kind.as_str(), now_ms())))
}

fn apply_art_file(paths: &DataPaths, runner: &str, app_name: &str, kind: ArtKind, dest: &Path) -> Result<CollectionGameArt> {
    let mut current = get_collection_game_art(paths, runner, app_name)?;
    match kind {
        ArtKind::Cover => {
            remove_managed(paths, current.cover_url.as_deref());
            current.cover_url = Some(display_url(paths, dest));
        }
        ArtKind::Hero => {
            remove_managed(paths, current.hero_url.as_deref());
            current.hero_url = Some(display_url(paths, dest));
        }
    }
    write_art(paths, runner, app_name, &current)
}

/// `setCollectionGameArt`: copies a local image (or a `localart://` one) into
/// `local_library/art/<runner_app>/<kind>-<ms>.<ext>` and points `art.json`
/// at it, deleting the previous managed file of that kind.
pub fn set_collection_game_art(
    paths: &DataPaths,
    runner: &str,
    app_name: &str,
    kind: ArtKind,
    source_path: &str,
) -> Result<CollectionGameArt> {
    let resolved = if source_path.starts_with("localart:") {
        paths.resolve_localart(source_path, None).filter(|p| p.exists())
    } else {
        Some(PathBuf::from(source_path))
    }
    .ok_or_else(|| anyhow!(tr!("Image not found: {path}", path = source_path)))?;
    let ext = normalize_ext(&resolved.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default());
    if !ALLOWED_EXT.contains(&ext.as_str()) {
        bail!(tr!("Unsupported image type: {kind}", kind = if ext.is_empty() { tr!("unknown") } else { &ext }));
    }
    if !resolved.exists() {
        bail!(tr!("Image not found: {path}", path = resolved.display()));
    }
    let dest = dest_file(paths, runner, app_name, kind, &ext)?;
    fs::copy(&resolved, &dest)?;
    apply_art_file(paths, runner, app_name, kind, &dest)
}

fn ext_from_url(url: &str, content_type: &str) -> String {
    let ty = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    let from_type = match ty.as_str() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/avif" => "avif",
        _ => "",
    };
    if !from_type.is_empty() {
        return from_type.into();
    }
    url::Url::parse(url).ok().and_then(|u| Path::new(u.path()).extension().map(|e| normalize_ext(&e.to_string_lossy()))).unwrap_or_default()
}

fn download_image_bytes(http: &Http, url: &str) -> Result<(Vec<u8>, String)> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.to_lowercase().starts_with("http")) {
        bail!(tr!("Image URL must be http(s)"));
    }
    let resp = http
        .get(url)
        .header("User-Agent", DOWNLOAD_UA)
        .header("Accept", "image/avif,image/webp,image/png,image/jpeg,image/gif,*/*;q=0.8")
        .header("Referer", "https://duckduckgo.com/")
        .timeout_secs(25)
        .max_bytes(MAX_IMAGE_BYTES)
        .send()?;
    if resp.status != 200 {
        bail!("HTTP {}", resp.status);
    }
    let ext = sniff_image_ext(&resp.body, true).map(str::to_owned).unwrap_or_else(|| ext_from_url(url, &resp.content_type));
    if ext.is_empty() || !ALLOWED_EXT.contains(&ext.as_str()) {
        bail!(tr!("Unsupported image type: {kind}", kind = if ext.is_empty() { tr!("unknown") } else { &ext }));
    }
    if resp.body.len() < 32 {
        bail!(tr!("Downloaded image is empty"));
    }
    Ok((resp.body, ext))
}

/// `setCollectionGameArtFromUrl`: downloads `image_url` (then `thumb_url` as a
/// fallback) and applies it like [`set_collection_game_art`].
pub fn set_collection_game_art_from_url(
    paths: &DataPaths,
    http: &Http,
    runner: &str,
    app_name: &str,
    kind: ArtKind,
    image_url: &str,
    thumb_url: Option<&str>,
) -> Result<CollectionGameArt> {
    let mut last: Option<anyhow::Error> = None;
    for url in [Some(image_url), thumb_url].into_iter().flatten().filter(|u| !u.is_empty()) {
        match download_image_bytes(http, url) {
            Ok((bytes, ext)) => {
                let dest = dest_file(paths, runner, app_name, kind, &ext)?;
                fs::write(&dest, bytes)?;
                return apply_art_file(paths, runner, app_name, kind, &dest);
            }
            Err(e) => {
                log::warn!("Collection art download failed for {}: {e}", redact_url(url));
                last = Some(e);
            }
        }
    }
    Err(anyhow!(tr!("Could not download image: {error}", error = last.map(|e| e.to_string()).unwrap_or_else(|| "undefined".into()))))
}

/// `clearCollectionGameArt`: reset one kind to the default art.
pub fn clear_collection_game_art(paths: &DataPaths, runner: &str, app_name: &str, kind: ArtKind) -> Result<CollectionGameArt> {
    let mut current = get_collection_game_art(paths, runner, app_name)?;
    match kind {
        ArtKind::Cover => {
            remove_managed(paths, current.cover_url.as_deref());
            current.cover_url = None;
        }
        ArtKind::Hero => {
            remove_managed(paths, current.hero_url.as_deref());
            current.hero_url = None;
        }
    }
    write_art(paths, runner, app_name, &current)
}

/// `removeAllCollectionGameArt`.
pub fn remove_all_collection_game_art(paths: &DataPaths, runner: &str, app_name: &str) -> Result<()> {
    clear_collection_game_art(paths, runner, app_name, ArtKind::Cover)?;
    clear_collection_game_art(paths, runner, app_name, ArtKind::Hero)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png() -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend(std::iter::repeat_n(7u8, 64));
        v
    }

    #[test]
    fn set_replace_and_reset_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("cfg"));
        // Heronite already wrote an entry with a field we do not model.
        fs::create_dir_all(paths.local_library()).unwrap();
        fs::write(paths.art(), "{\n\t\"games\": {\n\t\t\"sideload:other\": {\n\t\t\t\"heroUrl\": \"x\"\n\t\t}\n\t},\n\t\"future\": 1\n}")
            .unwrap();
        let src = dir.path().join("my cover.PNG");
        fs::write(&src, png()).unwrap();

        let art = set_collection_game_art(&paths, "sideload", "steam_9", ArtKind::Cover, src.to_str().unwrap()).unwrap();
        let url = art.cover_url.clone().unwrap();
        assert!(url.starts_with("localart://art/sideload_steam_9/cover-") && url.ends_with(".png"), "{url}");
        let file = paths.resolve_localart(&url, None).unwrap();
        assert!(file.exists());
        assert_eq!(fs::read(&file).unwrap(), png());

        // art.json round-trips with Heronite's shape and keeps foreign data.
        let raw = json_store::read_value(&paths.art()).unwrap();
        assert_eq!(raw["games"]["sideload:steam_9"]["coverUrl"], url);
        assert_eq!(raw["future"], 1);
        assert_eq!(raw["games"]["sideload:other"]["heroUrl"], "x");
        let typed: ArtFile = json_store::read(&paths.art()).unwrap();
        assert_eq!(typed.games["sideload:steam_9"].cover_url.as_deref(), Some(url.as_str()));

        // Replacing deletes the previous managed file.
        std::thread::sleep(std::time::Duration::from_millis(3));
        let hero = set_collection_game_art(&paths, "sideload", "steam_9", ArtKind::Hero, src.to_str().unwrap()).unwrap();
        assert_eq!(hero.cover_url.as_deref(), Some(url.as_str()));
        assert!(hero.hero_url.is_some());
        std::thread::sleep(std::time::Duration::from_millis(3));
        let again = set_collection_game_art(&paths, "sideload", "steam_9", ArtKind::Cover, src.to_str().unwrap()).unwrap();
        assert_ne!(again.cover_url.as_deref(), Some(url.as_str()));
        assert!(!file.exists());

        // Reset one kind, then the other: the entry disappears.
        let after = clear_collection_game_art(&paths, "sideload", "steam_9", ArtKind::Hero).unwrap();
        assert!(after.hero_url.is_none() && after.cover_url.is_some());
        clear_collection_game_art(&paths, "sideload", "steam_9", ArtKind::Cover).unwrap();
        let raw = json_store::read_value(&paths.art()).unwrap();
        assert!(raw["games"].get("sideload:steam_9").is_none());
        assert_eq!(raw["games"]["sideload:other"], json!({"heroUrl": "x"}));
        let left: Vec<_> = fs::read_dir(paths.art_dir().join("sideload_steam_9")).unwrap().collect();
        assert!(left.is_empty());
    }

    #[test]
    fn rejects_bad_sources() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let txt = dir.path().join("a.txt");
        fs::write(&txt, "x").unwrap();
        let e = set_collection_game_art(&paths, "sideload", "a", ArtKind::Cover, txt.to_str().unwrap()).unwrap_err();
        assert_eq!(e.to_string(), "Unsupported image type: txt");
        let e = set_collection_game_art(&paths, "sideload", "a", ArtKind::Cover, "/no/such.png").unwrap_err();
        assert!(e.to_string().starts_with("Image not found"));
        let e = set_collection_game_art(&paths, "sideload", "a", ArtKind::Cover, "localart://art/zz/none.png").unwrap_err();
        assert!(e.to_string().starts_with("Image not found"));
    }

    #[test]
    fn hydrate_drops_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        fs::create_dir_all(paths.local_library()).unwrap();
        fs::write(paths.art(), r#"{"games":{"sideload:a":{"coverUrl":"localart://art/gone/cover-1.png"}}}"#).unwrap();
        assert_eq!(get_collection_game_art(&paths, "sideload", "a").unwrap().cover_url, None);
    }
}
