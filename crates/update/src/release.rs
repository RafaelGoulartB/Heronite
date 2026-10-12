//! The latest GitHub release and the assets this build can install.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::version::Version;

/// Architecture part of the AppImage names (`appimagetool`'s `ARCH`).
pub const ARCH: &str = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };

/// The largest API answer read (a release with its notes and assets is a few KB).
const MAX_API_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// Bytes as GitHub reports them (0 = unknown).
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// `X.Y.Z` (the tag without its `v`).
    pub version: String,
    /// Release title (`Heronite v0.1.4`).
    pub name: String,
    /// Release notes, Markdown (grouped commit list made by git-cliff).
    pub notes: String,
    pub page_url: String,
    /// RFC 3339 (`2026-10-11T12:00:00Z`).
    pub published_at: String,
    /// The AppImage for this architecture; `None` when the release has none.
    pub appimage: Option<Asset>,
    /// Its minisign signature; without one the app does not install it.
    pub signature: Option<Asset>,
}

impl Release {
    pub fn parsed_version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }

    /// Newer than `current` (a release with an unreadable version never is).
    pub fn is_newer_than(&self, current: &Version) -> bool {
        self.parsed_version().is_some_and(|v| &v > current)
    }

    /// The app can download and swap it in by itself.
    pub fn installable(&self) -> bool {
        self.appimage.is_some() && self.signature.is_some()
    }

    /// `2026-10-11` from [`Release::published_at`].
    pub fn date(&self) -> &str {
        self.published_at.split('T').next().unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

impl From<&GhAsset> for Asset {
    fn from(a: &GhAsset) -> Self {
        Asset { name: a.name.clone(), url: a.browser_download_url.clone(), size: a.size }
    }
}

/// The AppImage to install: the fixed name first (`Heronite-x86_64.AppImage`),
/// else a versioned one (`Heronite-0.1.4-x86_64.AppImage`).
fn pick_appimage(assets: &[GhAsset]) -> Option<&GhAsset> {
    let fixed = format!("Heronite-{ARCH}.AppImage");
    let suffix = format!("-{ARCH}.AppImage");
    assets.iter().find(|a| a.name == fixed).or_else(|| assets.iter().find(|a| a.name.starts_with("Heronite-") && a.name.ends_with(&suffix)))
}

/// Parses `GET /repos/{repo}/releases/latest`.
pub fn parse_latest(json: &str) -> Result<Release> {
    let r: GhRelease = serde_json::from_str(json).context("unexpected GitHub release response")?;
    if r.draft || r.prerelease {
        bail!("the latest release is a draft or a pre-release");
    }
    let version = Version::parse(&r.tag_name).ok_or_else(|| anyhow!("the release tag {:?} is not a version", r.tag_name))?;
    let appimage = pick_appimage(&r.assets);
    let signature = appimage.and_then(|a| {
        let name = format!("{}.minisig", a.name);
        r.assets.iter().find(|s| s.name == name)
    });
    Ok(Release {
        version: version.to_string(),
        name: r.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Heronite {}", r.tag_name)),
        notes: r.body.unwrap_or_default().replace("\r\n", "\n"),
        page_url: r.html_url,
        published_at: r.published_at.unwrap_or_default(),
        appimage: appimage.map(Asset::from),
        signature: signature.map(Asset::from),
    })
}

#[derive(Debug)]
pub enum Fetched {
    /// Same answer as for `etag`: the stored release is still the latest.
    NotModified,
    Latest {
        release: Box<Release>,
        etag: Option<String>,
    },
}

/// Asks `api_url` for the latest release. With the ETag of the last answer
/// GitHub replies 304 when nothing changed (no body to read or parse).
pub fn fetch_latest(api_url: &str, etag: Option<&str>) -> Result<Fetched> {
    let mut req = crate::agent()
        .get(api_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    if let Some(etag) = etag.filter(|e| !e.is_empty()) {
        req = req.header("If-None-Match", etag);
    }
    let mut resp = req.call().map_err(crate::http_error).with_context(|| format!("checking {api_url}"))?;
    if resp.status() == 304 {
        return Ok(Fetched::NotModified);
    }
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let body = resp.body_mut().with_config().limit(MAX_API_BYTES).read_to_string().map_err(|e| anyhow!("{e}"))?;
    Ok(Fetched::Latest { release: Box::new(parse_latest(&body)?), etag })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<_> =
            assets.iter().map(|n| serde_json::json!({"name": n, "browser_download_url": format!("https://dl/{n}"), "size": 42})).collect();
        serde_json::json!({
            "tag_name": tag,
            "name": format!("Heronite {tag}"),
            "body": "### Features\r\n- Something new",
            "html_url": format!("https://github.com/x/y/releases/tag/{tag}"),
            "published_at": "2026-10-11T12:00:00Z",
            "draft": false,
            "prerelease": false,
            "assets": assets,
        })
        .to_string()
    }

    #[test]
    fn prefers_the_fixed_name_and_finds_its_signature() {
        let json = gh(
            "v0.1.4",
            &[
                "Heronite-0.1.4-x86_64.AppImage",
                "Heronite-0.1.4-x86_64.AppImage.minisig",
                "Heronite-x86_64.AppImage",
                "Heronite-x86_64.AppImage.minisig",
                "Heronite-x86_64.AppImage.zsync",
            ],
        );
        let r = parse_latest(&json).unwrap();
        assert_eq!(r.version, "0.1.4");
        assert_eq!(r.date(), "2026-10-11");
        assert_eq!(r.notes, "### Features\n- Something new");
        if ARCH == "x86_64" {
            assert_eq!(r.appimage.as_ref().unwrap().name, "Heronite-x86_64.AppImage");
            assert_eq!(r.signature.as_ref().unwrap().url, "https://dl/Heronite-x86_64.AppImage.minisig");
            assert!(r.installable());
        }
        assert!(r.is_newer_than(&Version::parse("0.1.3").unwrap()));
        assert!(!r.is_newer_than(&Version::parse("0.1.4").unwrap()));
    }

    #[test]
    fn versioned_asset_without_signature_is_not_installable() {
        let r = parse_latest(&gh("v0.2.0", &["Heronite-0.2.0-x86_64.AppImage", "Heronite-0.2.0-x86_64.AppImage.sha256"])).unwrap();
        if ARCH == "x86_64" {
            assert_eq!(r.appimage.as_ref().unwrap().name, "Heronite-0.2.0-x86_64.AppImage");
        }
        assert!(r.signature.is_none());
        assert!(!r.installable());
        let none = parse_latest(&gh("v0.2.0", &["notes.txt"])).unwrap();
        assert!(none.appimage.is_none());
    }

    #[test]
    fn rejects_bad_tags_and_prereleases() {
        assert!(parse_latest(&gh("nightly", &[])).is_err());
        let mut v: serde_json::Value = serde_json::from_str(&gh("v1.0.0", &[])).unwrap();
        v["prerelease"] = true.into();
        assert!(parse_latest(&v.to_string()).is_err());
        assert!(parse_latest("{").is_err());
    }

    #[test]
    fn stored_release_round_trips() {
        let r = parse_latest(&gh("v0.1.4", &["Heronite-x86_64.AppImage"])).unwrap();
        let back: Release = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
