//! A game's trailers and screenshots from its store page (the focus panel's
//! "Store media" section, off until the user turns it on).
//!
//! Steam is the source today (`appdetails` with `filters=movies,screenshots`,
//! for any game that has a Steam app id). The list is small JSON kept for a
//! few days under `<cache>/store-media/lists/`; the trailers themselves are
//! streamed when played, and only their first seconds may be kept
//! ([`cache`]), within the size the user allows. Turning the section off
//! deletes the whole folder ([`clear`]).

pub mod cache;
pub mod hls;

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::Http;
use crate::types::Lookup;
use crate::util::now_ms;

/// How long a list is used before the store is asked again. Steam's trailer
/// addresses carry a timestamp that changes now and then; a list whose
/// trailer no longer loads is fetched again at once ([`forget`]).
const LIST_MS: i64 = 3 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreMedia {
    /// `"steam"`.
    pub source: String,
    /// The store page the media come from.
    pub page: String,
    #[serde(default)]
    pub videos: Vec<StoreVideo>,
    #[serde(default)]
    pub screenshots: Vec<StoreShot>,
    /// Milliseconds since the epoch.
    #[serde(default)]
    pub fetched_at: i64,
}

impl StoreMedia {
    pub fn is_empty(&self) -> bool {
        self.videos.is_empty() && self.screenshots.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreVideo {
    /// The store's id of the video (names its cached files).
    pub id: String,
    pub name: String,
    /// A small still (Steam's is 293x165).
    pub thumb: String,
    /// The HLS master playlist (H.264).
    pub hls: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreShot {
    /// About 600 px wide.
    pub thumb: String,
    /// Full size (1920 px on Steam).
    pub full: String,
}

/// `<dir>/lists/<source>-<id>.json`.
fn list_path(dir: &Path, source: &str, id: &str) -> PathBuf {
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    dir.join("lists").join(format!("{source}-{id}.json"))
}

/// The `movies` and `screenshots` of an `appdetails` answer's `data`.
pub fn from_steam(app_id: &str, data: &Value) -> StoreMedia {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_owned();
    let list = |k: &str| data.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut videos: Vec<(bool, StoreVideo)> = list("movies")
        .iter()
        .filter_map(|m| {
            let id = match m.get("id") {
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::String(t)) => t.clone(),
                _ => return None,
            };
            let hls = s(m, "hls_h264");
            (!hls.is_empty()).then(|| {
                let highlight = m.get("highlight").and_then(Value::as_bool).unwrap_or(false);
                (highlight, StoreVideo { id, name: s(m, "name"), thumb: s(m, "thumbnail"), hls })
            })
        })
        .collect();
    // The store's own order, its highlighted trailers first.
    videos.sort_by_key(|(highlight, _)| !*highlight);
    let screenshots = list("screenshots")
        .iter()
        .map(|p| StoreShot { thumb: s(p, "path_thumbnail"), full: s(p, "path_full") })
        .filter(|p| !p.thumb.is_empty() && !p.full.is_empty())
        .collect();
    StoreMedia {
        source: "steam".into(),
        page: format!("https://store.steampowered.com/app/{app_id}"),
        videos: videos.into_iter().map(|(_, v)| v).collect(),
        screenshots,
        fetched_at: 0,
    }
}

fn fetch_steam(http: &Http, app_id: &str) -> Lookup<StoreMedia> {
    let url = format!("{}/api/appdetails", http.endpoints.steam_store);
    let body = http
        .get(url)
        .query(&[("appids", app_id), ("filters", "movies,screenshots")])
        .timeout_secs(20)
        .send_ok()
        .and_then(|r| r.json::<Value>());
    match body {
        Ok(body) => {
            let payload = body.get(app_id);
            let ok = payload.and_then(|p| p.get("success")).and_then(Value::as_bool) == Some(true);
            match payload.and_then(|p| p.get("data")).filter(|_| ok) {
                Some(data) => Lookup::Found(from_steam(app_id, data)),
                None => Lookup::NotFound,
            }
        }
        Err(e) => {
            log::warn!("Steam media of {app_id}: {e}");
            Lookup::from_error(&e)
        }
    }
}

/// The trailers and screenshots of the Steam app `app_id`: the copy under
/// `dir` while it is recent, else asked again (an old copy still answers
/// when Steam cannot be reached). Blocking.
pub fn steam(http: &Http, dir: &Path, app_id: &str) -> Lookup<StoreMedia> {
    let app_id = app_id.trim();
    if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
        return Lookup::Skipped;
    }
    let path = list_path(dir, "steam", app_id);
    let kept: Option<StoreMedia> = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if let Some(k) = kept.as_ref().filter(|k| now_ms() - k.fetched_at < LIST_MS) {
        return Lookup::Found(k.clone());
    }
    match fetch_steam(http, app_id) {
        Lookup::Found(mut media) => {
            media.fetched_at = now_ms();
            if let Err(e) = write_atomic(&path, &serde_json::to_vec(&media).unwrap_or_default()) {
                log::warn!("could not keep {}: {e}", path.display());
            }
            Lookup::Found(media)
        }
        Lookup::Unreachable => kept.map_or(Lookup::Unreachable, Lookup::Found),
        miss => miss,
    }
}

/// Drops the kept list of `app_id`, so the next [`steam`] asks again.
pub fn forget(dir: &Path, app_id: &str) {
    let _ = fs::remove_file(list_path(dir, "steam", app_id));
}

/// Deletes everything kept for the section: the lists and the videos.
pub fn clear(dir: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The addresses of every picture the kept lists name (their copies in the
/// shared image cache go with the section).
pub fn kept_pictures(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir.join("lists")) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| serde_json::from_slice::<StoreMedia>(&fs::read(e.path()).ok()?).ok())
        .flat_map(|m| {
            let videos = m.videos.into_iter().map(|v| v.thumb);
            videos.chain(m.screenshots.into_iter().flat_map(|s| [s.thumb, s.full])).collect::<Vec<_>>()
        })
        .filter(|u| !u.is_empty())
        .collect()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data() -> Value {
        json!({
            "movies": [
                {"id": 1, "name": "Gameplay", "thumbnail": "https://cdn/1.jpg", "hls_h264": "https://v/1/hls.m3u8", "highlight": false},
                {"id": 2, "name": "Launch Trailer", "thumbnail": "https://cdn/2.jpg", "hls_h264": "https://v/2/hls.m3u8", "highlight": true},
                {"id": 3, "name": "Old, no HLS", "thumbnail": "https://cdn/3.jpg", "mp4": {"480": "x"}}
            ],
            "screenshots": [
                {"id": 0, "path_thumbnail": "https://cdn/s0.600x338.jpg", "path_full": "https://cdn/s0.1920x1080.jpg"},
                {"id": 1, "path_thumbnail": "", "path_full": "https://cdn/s1.jpg"}
            ]
        })
    }

    #[test]
    fn steam_lists_highlights_first_and_skips_what_cannot_play() {
        let m = from_steam("620", &data());
        assert_eq!(m.page, "https://store.steampowered.com/app/620");
        let names: Vec<&str> = m.videos.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Launch Trailer", "Gameplay"]);
        assert_eq!(m.videos[0].id, "2");
        assert_eq!(m.screenshots.len(), 1);
        assert_eq!(m.screenshots[0].full, "https://cdn/s0.1920x1080.jpg");
    }

    #[test]
    fn a_kept_list_is_used_and_cleared_with_the_section() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store-media");
        let mut m = from_steam("620", &data());
        m.fetched_at = now_ms();
        write_atomic(&list_path(&root, "steam", "620"), &serde_json::to_vec(&m).unwrap()).unwrap();
        // Served from the copy: the endpoint is never asked.
        let http = Http::new(crate::Endpoints { steam_store: "http://127.0.0.1:9".into(), ..Default::default() });
        assert_eq!(steam(&http, &root, "620").into_option(), Some(m));
        assert!(kept_pictures(&root).contains(&"https://cdn/s0.1920x1080.jpg".to_string()));
        assert!(matches!(steam(&http, &root, "not-a-number"), Lookup::Skipped));
        clear(&root).unwrap();
        assert!(!root.exists());
        clear(&root).unwrap();
    }
}
