//! Screenshots of a game Steam has no media of, from the other sources the
//! metadata uses (no trailers: theirs are YouTube or Wistia pages, which
//! ffmpeg does not play). Each source answers for the game its record
//! already matched, so nothing is searched here:
//!
//! - IGDB: the record's IGDB id (`games` → `screenshots.image_id`), with the
//!   user's keys.
//! - The store: a GOG game's product (`api.gog.com/products/<id>?expand=screenshots`),
//!   or the screenshots an Amazon game's library entry lists (no request).
//! - LaunchBox: the "Screenshot - Gameplay" pictures of the record's
//!   LaunchBox game, in the downloaded database (no request but the pictures).
//! - libretro: the snapshot and title screen next to the record's box art
//!   (`Named_Snaps`, `Named_Titles`), the ones that exist.
//!
//! [`crate::MetadataService::store_media`] asks them in the order of the
//! "Store media" field and keeps the first that has any. The lists are kept like
//! Steam's ([`super::kept_or_fetched`]).

use serde_json::Value;

use super::{StoreMedia, StoreShot};
use crate::http::{Http, APP_UA};
use crate::igdb::{igdb_image_url, Igdb, IgdbCredentials};
use crate::launchbox::index::ImageKind;
use crate::launchbox::{map as lb_map, LaunchBox};
use crate::types::Lookup;
use heronite_data::paths::DataPaths;

/// Screenshots kept of one game at most.
pub const MAX_SHOTS: usize = 12;

/// What the sources other than Steam need to find a game's screenshots:
/// the ids its record matched, and what its own store keeps.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShotsGame {
    /// The IGDB id the record matched.
    pub igdb_id: Option<i64>,
    /// The game's GOG product id, when one of its copies is a GOG game.
    pub gog_id: Option<String>,
    /// The screenshots an Amazon copy's library entry lists.
    pub store_shots: Vec<String>,
    /// The LaunchBox DatabaseID the record matched.
    pub launchbox_id: Option<i64>,
    /// The record's box art from libretro-thumbnails (`.../Named_Boxarts/<name>.png`).
    pub libretro_boxart: Option<String>,
}

fn media(source: &str, page: String, screenshots: Vec<StoreShot>) -> StoreMedia {
    let mut screenshots = screenshots;
    screenshots.retain(|s| !s.thumb.is_empty() && !s.full.is_empty());
    screenshots.dedup_by(|a, b| a.full == b.full);
    screenshots.truncate(MAX_SHOTS);
    StoreMedia { source: source.into(), page, screenshots, ..Default::default() }
}

/// The screenshots of IGDB game `id`. [`Lookup::Skipped`] without keys.
pub fn igdb(igdb: &Igdb, http: &Http, creds: &IgdbCredentials, id: i64) -> Lookup<StoreMedia> {
    if !creds.configured() {
        return Lookup::Skipped;
    }
    let body = format!("fields url,screenshots.image_id; where id = {id}; limit 1;");
    match igdb.query_games(http, creds, &body) {
        Ok(rows) => match rows.into_iter().next() {
            Some(game) => {
                let shots = game
                    .screenshots
                    .iter()
                    .filter_map(|s| {
                        let id = s.image_id.as_deref();
                        Some(StoreShot { thumb: igdb_image_url(id, "screenshot_med")?, full: igdb_image_url(id, "1080p")? })
                    })
                    .collect();
                Lookup::Found(media("igdb", game.url.unwrap_or_default(), shots))
            }
            None => Lookup::NotFound,
        },
        Err(e) => {
            log::warn!("IGDB screenshots of {id}: {e:#}");
            Lookup::from_error(&e)
        }
    }
}

/// The screenshots of GOG product `id`.
pub fn gog(http: &Http, id: &str) -> Lookup<StoreMedia> {
    let id = id.trim();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Lookup::Skipped;
    }
    let url = format!("{}/products/{id}", http.endpoints.gog_api.trim_end_matches('/'));
    let body = http
        .get(url)
        .query(&[("expand", "screenshots")])
        .header("User-Agent", APP_UA)
        .timeout_secs(20)
        .send_ok()
        .and_then(|r| r.json::<Value>());
    match body {
        Ok(body) => Lookup::Found(from_gog(&body)),
        Err(e) => {
            log::warn!("GOG screenshots of {id}: {e:#}");
            Lookup::from_error(&e)
        }
    }
}

/// The `screenshots` of a GOG product answer: the medium picture (about
/// 540 px wide) and the large one (1280 px).
pub fn from_gog(body: &Value) -> StoreMedia {
    let formatted = |shot: &Value, name: &str| {
        shot.get("formatted_images")?
            .as_array()?
            .iter()
            .find(|f| f.get("formatter_name").and_then(Value::as_str) == Some(name))?
            .get("image_url")?
            .as_str()
            .map(str::to_owned)
    };
    let shots = body
        .get("screenshots")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|s| {
                    let full = formatted(s, "ggvgl_2x").or_else(|| formatted(s, "ggvgl"))?;
                    let thumb = formatted(s, "ggvgm_2x").or_else(|| formatted(s, "ggvgm")).unwrap_or_else(|| full.clone());
                    Some(StoreShot { thumb, full })
                })
                .collect()
        })
        .unwrap_or_default();
    let page = body.pointer("/links/product_card").and_then(Value::as_str).unwrap_or("").to_owned();
    media("gog", page, shots)
}

/// The screenshots an Amazon game's library entry lists.
pub fn amazon(urls: &[String]) -> Lookup<StoreMedia> {
    let shots: Vec<StoreShot> =
        urls.iter().filter(|u| u.starts_with("https://")).map(|u| StoreShot { thumb: u.clone(), full: u.clone() }).collect();
    if shots.is_empty() {
        Lookup::NotFound
    } else {
        Lookup::Found(media("amazon", String::new(), shots))
    }
}

/// The addresses of an Amazon library entry's `screenshots`: each a string,
/// or an object with a `url`.
pub fn amazon_urls(list: Option<&Value>) -> Vec<String> {
    let Some(list) = list.and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.trim().to_owned()),
            Value::Object(o) => o.get("url").and_then(Value::as_str).map(|s| s.trim().to_owned()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// The gameplay screenshots of LaunchBox game `id`, from the downloaded
/// database ([`Lookup::Skipped`] without one).
pub fn launchbox(lb: &LaunchBox, paths: &DataPaths, http: &Http, id: i64) -> Lookup<StoreMedia> {
    lb.by_id(paths, &id.to_string()).map(|game| {
        let base = &http.endpoints.launchbox_images;
        let shots = game
            .images
            .iter()
            .filter(|i| i.kind == ImageKind::Screenshot)
            .map(|i| {
                let url = lb_map::image_url(base, &i.file);
                StoreShot { thumb: url.clone(), full: url }
            })
            .collect();
        media("launchbox", format!("https://gamesdb.launchbox-app.com/games/details/{id}"), shots)
    })
}

/// The snapshot and title screen libretro-thumbnails has next to `boxart`
/// (one of its `Named_Boxarts` addresses): those that exist.
pub fn libretro(http: &Http, boxart: &str) -> Lookup<StoreMedia> {
    let base = http.endpoints.libretro_thumbnails.trim_end_matches('/');
    if !boxart.starts_with(&format!("{base}/")) || !boxart.contains("/Named_Boxarts/") {
        return Lookup::Skipped;
    }
    let mut shots = Vec::new();
    let mut unreachable = false;
    for folder in ["/Named_Snaps/", "/Named_Titles/"] {
        let url = boxart.replacen("/Named_Boxarts/", folder, 1);
        let sent = http.get(&url).header("Range", "bytes=0-31").header("User-Agent", APP_UA).timeout_secs(15).max_bytes(1 << 20).send();
        match sent {
            Ok(r) if r.ok() => shots.push(StoreShot { thumb: url.clone(), full: url }),
            Ok(_) => {}
            Err(_) => unreachable = true,
        }
    }
    match (shots.is_empty(), unreachable) {
        (false, _) => Lookup::Found(media("libretro", String::new(), shots)),
        (true, true) => Lookup::Unreachable,
        (true, false) => Lookup::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gog_gives_its_large_and_medium_pictures() {
        let shot = |id: &str| {
            json!({"image_id": id, "formatted_images": [
                {"formatter_name": "ggvgm", "image_url": format!("https://g/{id}_ggvgm.jpg")},
                {"formatter_name": "ggvgm_2x", "image_url": format!("https://g/{id}_ggvgm_2x.jpg")},
                {"formatter_name": "ggvgl_2x", "image_url": format!("https://g/{id}_ggvgl_2x.jpg")}
            ]})
        };
        let body =
            json!({"links": {"product_card": "https://www.gog.com/game/x"}, "screenshots": [shot("a"), {"image_id": "b"}, shot("c")]});
        let m = from_gog(&body);
        assert_eq!((m.source.as_str(), m.page.as_str()), ("gog", "https://www.gog.com/game/x"));
        assert_eq!(m.screenshots.len(), 2, "a picture without its large size is left out");
        assert_eq!(m.screenshots[0], StoreShot { thumb: "https://g/a_ggvgm_2x.jpg".into(), full: "https://g/a_ggvgl_2x.jpg".into() });
        assert!(m.videos.is_empty());
    }

    #[test]
    fn amazon_reads_strings_and_objects() {
        let list = json!(["https://m/1.jpg", {"url": "https://m/2.jpg"}, "", 3, "http://m/insecure.jpg"]);
        let urls = amazon_urls(Some(&list));
        assert_eq!(urls, ["https://m/1.jpg", "https://m/2.jpg", "http://m/insecure.jpg"]);
        let Lookup::Found(m) = amazon(&urls) else { panic!() };
        assert_eq!(m.screenshots.len(), 2, "only https pictures");
        assert!(matches!(amazon(&[]), Lookup::NotFound));
    }

    #[test]
    fn a_list_keeps_at_most_its_share_without_repeats() {
        let shot = |n: usize| StoreShot { thumb: format!("t{n}"), full: format!("f{n}") };
        let mut list: Vec<StoreShot> = (0..20).map(shot).collect();
        list.insert(1, shot(0));
        let m = media("x", String::new(), list);
        assert_eq!(m.screenshots.len(), MAX_SHOTS);
        assert_eq!(m.screenshots[1], shot(1));
    }

    #[test]
    fn libretro_is_asked_only_for_its_own_box_art() {
        let http = Http::new(crate::Endpoints { libretro_thumbnails: "http://127.0.0.1:9".into(), ..Default::default() });
        assert!(matches!(libretro(&http, "https://elsewhere/Named_Boxarts/x.png"), Lookup::Skipped));
        assert!(matches!(libretro(&http, "http://127.0.0.1:9/Nintendo - SNES/Named_Logos/x.png"), Lookup::Skipped));
        assert!(matches!(libretro(&http, "http://127.0.0.1:9/Nintendo - SNES/Named_Boxarts/x.png"), Lookup::Unreachable));
    }
}
