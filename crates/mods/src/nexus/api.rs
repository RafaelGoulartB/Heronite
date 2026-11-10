//! Nexus REST v1 (`api.nexusmods.com/v1`): every call carries the user's API key.

use std::sync::atomic::{AtomicU64, Ordering};

use heronite_i18n::tr;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::{null_default, Account, NexusFile, NexusFiles, NxmLink};
use crate::http::{self, HttpError};

const BASE: &str = "https://api.nexusmods.com/v1";

const NONE: u64 = u64::MAX;
/// Last `X-RL-Hourly-Remaining` and `X-RL-Daily-Remaining` seen; `NONE` =
/// no answer carried it yet.
static HOURLY: AtomicU64 = AtomicU64::new(NONE);
static DAILY: AtomicU64 = AtomicU64::new(NONE);

/// Requests left `(this hour, today)` as of the last Nexus answer, `None`
/// before any call. A side the answers did not carry reads `u32::MAX`.
pub fn rate_limits() -> Option<(u32, u32)> {
    let (h, d) = (HOURLY.load(Ordering::Relaxed), DAILY.load(Ordering::Relaxed));
    if h == NONE && d == NONE {
        return None;
    }
    let side = |v: u64| if v == NONE { u32::MAX } else { v as u32 };
    Some((side(h), side(d)))
}

fn note_rate_limits<T>(reply: &http::Reply<T>) {
    let read = |name: &str| reply.header(name).and_then(|v| v.trim().parse::<u32>().ok());
    if let Some(h) = read("x-rl-hourly-remaining") {
        HOURLY.store(h.into(), Ordering::Relaxed);
    }
    if let Some(d) = read("x-rl-daily-remaining") {
        DAILY.store(d.into(), Ordering::Relaxed);
    }
}

/// GET `BASE + path` with the key. The error keeps its HTTP status.
pub(super) fn rest<T: DeserializeOwned>(api_key: &str, path: &str) -> Result<T, HttpError> {
    let headers = [("apikey", api_key), ("Application-Name", "Heronite"), ("Application-Version", env!("CARGO_PKG_VERSION"))];
    let reply = http::get_json_reply::<T>(&format!("{BASE}{path}"), &headers)?;
    note_rate_limits(&reply);
    Ok(reply.value)
}

/// `users/validate.json`; the `?` spellings are the older field names.
#[derive(Debug, Deserialize)]
pub(super) struct RawUser {
    #[serde(default)]
    user_id: u64,
    #[serde(default, deserialize_with = "null_default")]
    name: String,
    #[serde(default, alias = "is_premium?")]
    is_premium: bool,
    #[serde(default, alias = "is_supporter?")]
    is_supporter: bool,
    #[serde(default)]
    profile_url: Option<String>,
}

impl From<RawUser> for Account {
    fn from(r: RawUser) -> Self {
        Account {
            name: r.name,
            user_id: r.user_id,
            is_premium: r.is_premium,
            is_supporter: r.is_supporter,
            avatar: r.profile_url.filter(|u| !u.trim().is_empty()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawLink {
    #[serde(rename = "URI")]
    uri: String,
}

/// The CDN URL of a file. A link from the site's "Mod manager download" button
/// carries `key` and `expires`, which let free accounts through.
pub fn download_link(api_key: &str, link: &NxmLink) -> Result<String, String> {
    let mut path = format!("/games/{}/mods/{}/files/{}/download_link.json", http::encode(&link.domain), link.mod_id, link.file_id);
    if let (Some(key), Some(expires)) = (&link.key, link.expires) {
        path.push_str(&format!("?key={}&expires={expires}", http::encode(key)));
    }
    let links: Vec<RawLink> = rest(api_key, &path).map_err(|e| match (e.status(), link.key.is_some()) {
        (Some(403), true) => {
            e.with_message(tr!("The download link from Nexus Mods expired or is not valid: use the \"Mod manager download\" button on the mod's page again").to_string())
        }
        (Some(403), false) => {
            e.with_message(tr!("Free Nexus Mods accounts download with the \"Mod manager download\" button on the mod's page").to_string())
        }
        _ => e,
    })?;
    links.into_iter().map(|l| l.uri).find(|u| !u.is_empty()).ok_or_else(|| tr!("Nexus Mods gave no download link").to_string())
}

/// A mod page (`mods/{id}.json`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NexusMod {
    pub mod_id: u64,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    pub summary: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub picture_url: Option<String>,
    pub contains_adult_content: bool,
    /// False once the author hid or removed it.
    #[serde(default = "yes")]
    pub available: bool,
    /// `published`, `hidden`, `under_moderation`, …
    pub status: Option<String>,
}

fn yes() -> bool {
    true
}

pub fn mod_info(api_key: &str, domain: &str, mod_id: u64) -> Result<NexusMod, String> {
    Ok(rest(api_key, &format!("/games/{}/mods/{mod_id}.json", http::encode(domain)))?)
}

pub fn files(api_key: &str, domain: &str, mod_id: u64) -> Result<NexusFiles, String> {
    Ok(rest(api_key, &format!("/games/{}/mods/{mod_id}/files.json", http::encode(domain)))?)
}

pub fn file_info(api_key: &str, domain: &str, mod_id: u64, file_id: u64) -> Result<NexusFile, String> {
    Ok(rest(api_key, &format!("/games/{}/mods/{mod_id}/files/{file_id}.json", http::encode(domain)))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_links_answer_as_a_list() {
        let v: Vec<RawLink> = serde_json::from_str(
            r#"[{"name":"Nexus CDN","short_name":"Nexus CDN","URI":"https://cf-files.nexusmods.com/cdn/1704/12604/SkyUI-12604-5-2SE.7z?md5=x&expires=1"}]"#,
        )
        .unwrap();
        assert!(v[0].uri.starts_with("https://cf-files.nexusmods.com/"));
    }

    #[test]
    fn mod_pages_tolerate_nulls_and_extra_fields() {
        let m: NexusMod = serde_json::from_str(
            r#"{"name":"SkyUI","summary":"UI","description":"<b>x</b>","picture_url":"https://staticdelivery.nexusmods.com/p.png",
                "mod_id":12604,"game_id":1704,"domain_name":"skyrimspecialedition","version":"5.2SE","author":"SkyUI Team",
                "contains_adult_content":false,"status":"published","available":true,"user":{"member_id":1,"name":"x"},"endorsement":null}"#,
        )
        .unwrap();
        assert_eq!((m.mod_id, m.name.as_str(), m.version.as_deref(), m.available), (12604, "SkyUI", Some("5.2SE"), true));
        let hidden: NexusMod = serde_json::from_str(r#"{"name":null,"summary":null,"available":false,"status":"hidden"}"#).unwrap();
        assert_eq!((hidden.name.as_str(), hidden.summary, hidden.available), ("", None, false));
        let bare: NexusMod = serde_json::from_str("{}").unwrap();
        assert!(bare.available);
    }
}
