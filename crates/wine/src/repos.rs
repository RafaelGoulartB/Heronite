//! Repositories Heroic offers on Linux and GitHub release parsing (port of
//! `wine/manager/downloader/{constants,utilities}.ts` and the Linux branch of
//! `getAvailableVersions`).

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use heronite_i18n::tr;
use serde::Deserialize;

use crate::types::WineVersionInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Repository {
    ProtonGe,
    ProtonCachyos,
    /// Valve's Proton (not offered in the Wine Manager UI on Linux).
    Proton,
    /// Lutris Wine builds (not offered in the UI on Linux).
    WineLutris,
}

impl Repository {
    /// Tabs of Heroic's Wine Manager on Linux, in its order (also the ones
    /// refreshed by default).
    pub const LINUX: [Repository; 2] = [Repository::ProtonCachyos, Repository::ProtonGe];

    /// GitHub releases endpoint (`constants.ts`).
    pub fn api_url(self) -> &'static str {
        match self {
            Repository::ProtonGe => "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases",
            Repository::ProtonCachyos => "https://api.github.com/repos/CachyOS/proton-cachyos/releases",
            Repository::Proton => "https://api.github.com/repos/ValveSoftware/Proton/releases",
            Repository::WineLutris => "https://api.github.com/repos/lutris/wine/releases",
        }
    }

    /// `type` stored in `wine-downloader-info.json` (Heroic's `Type`).
    pub fn kind(self) -> &'static str {
        match self {
            Repository::ProtonGe => "GE-Proton",
            Repository::ProtonCachyos => "Proton-CachyOS",
            Repository::Proton => "Proton",
            Repository::WineLutris => "Wine-Lutris",
        }
    }

    pub fn from_kind(kind: &str) -> Option<Self> {
        [Repository::ProtonGe, Repository::ProtonCachyos, Repository::Proton, Repository::WineLutris].into_iter().find(|r| r.kind() == kind)
    }

    /// Blurb under the tabs (Heroic `wineExplanation.*`).
    pub fn explanation(self) -> &'static str {
        match self {
            Repository::ProtonGe => tr!("GE-Proton is a Proton variant created by Glorious Eggroll. It is meant to be used along with the umu launcher (default in Heroic)."),
            Repository::ProtonCachyos => tr!("Proton-CachyOS is a Proton variant maintained by the CachyOS team. It includes extra tools like DXVK-Sarek and D7VK."),
            Repository::Proton => tr!("Valve's Proton."),
            Repository::WineLutris => tr!("Wine builds from Lutris."),
        }
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    html_url: String,
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

fn version_name(kind: &str, tag: &str) -> String {
    if kind.contains("Wine") {
        format!("Wine-{tag}")
    } else {
        tag.to_owned()
    }
}

/// `/\d+-\d+$/` on the version (picks the newest "plain" build over special
/// builds like LoL or Diablo wine for `-latest`).
fn ends_with_digits_dash_digits(s: &str) -> bool {
    let digits = s.trim_end_matches(|c: char| c.is_ascii_digit());
    if digits.len() == s.len() {
        return false;
    }
    match digits.strip_suffix('-') {
        Some(before) => before.ends_with(|c: char| c.is_ascii_digit()),
        None => false,
    }
}

/// Parses the GitHub releases response. Returns the releases (newest first,
/// as GitHub lists them) with the synthetic `<type>-latest` entry first.
pub fn parse_releases(repo: Repository, json: &str) -> Result<Vec<WineVersionInfo>> {
    let list: Vec<GhRelease> = serde_json::from_str(json).context("unexpected GitHub releases response")?;
    let kind = repo.kind();
    let mut releases = Vec::with_capacity(list.len() + 1);
    for r in list {
        let mut info = WineVersionInfo {
            version: version_name(kind, &r.tag_name),
            kind: kind.to_owned(),
            date: r.published_at.split('T').next().unwrap_or_default().to_owned(),
            release_notes_link: r.html_url,
            ..Default::default()
        };
        match repo {
            Repository::ProtonGe => {
                // GE-Proton ships aarch64 and x86_64 builds: pick ours.
                let want_arm = cfg!(target_arch = "aarch64");
                for a in &r.assets {
                    let is_arm = a.name.contains("-aarch64");
                    if is_arm != want_arm {
                        continue;
                    }
                    if a.name.ends_with(".sha512sum") {
                        info.checksum = Some(a.browser_download_url.clone());
                    } else if a.name.ends_with(".tar.gz") {
                        info.download = Some(a.browser_download_url.clone());
                        info.downsize = Some(a.size);
                    }
                }
            }
            Repository::ProtonCachyos => {
                let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
                if let Some(a) = r.assets.iter().find(|a| a.browser_download_url.ends_with(&format!("{arch}.sha512sum"))) {
                    info.checksum = Some(a.browser_download_url.clone());
                }
                if let Some(a) = r.assets.iter().find(|a| a.browser_download_url.ends_with(&format!("{arch}.tar.xz"))) {
                    info.download = Some(a.browser_download_url.clone());
                    info.downsize = Some(a.size);
                }
            }
            _ => {
                for a in &r.assets {
                    if a.name.ends_with("sha512sum") {
                        info.checksum = Some(a.browser_download_url.clone());
                    } else if a.name.ends_with("tar.gz") || a.name.ends_with("tar.xz") {
                        info.download = Some(a.browser_download_url.clone());
                        info.downsize = Some(a.size);
                    }
                }
            }
        }
        releases.push(info);
    }

    if !releases.is_empty() {
        let latest = releases.iter().find(|r| ends_with_digits_dash_digits(&r.version)).unwrap_or(&releases[0]);
        let entry = WineVersionInfo {
            version: format!("{}-latest", latest.kind),
            kind: latest.kind.clone(),
            date: latest.date.clone(),
            download: latest.download.clone(),
            downsize: latest.downsize,
            disksize: latest.disksize,
            checksum: latest.checksum.clone(),
            release_notes_link: latest.release_notes_link.clone(),
            ..Default::default()
        };
        releases.insert(0, entry);
    }
    Ok(releases)
}

/// Blocking GitHub request (unauthenticated: 60 requests/hour per IP, so call
/// it on demand only).
pub fn fetch_releases(repo: Repository, count: usize) -> Result<Vec<WineVersionInfo>> {
    let url = format!("{}?per_page={count}", repo.api_url());
    let body = http_get_text(&url, 2 * 1024 * 1024)
        .map_err(|e| anyhow!(tr!("Could not fetch available releases from {url}: {error}", url = repo.api_url(), error = e)))?;
    parse_releases(repo, &body)
}

fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .max_redirects(8)
            .user_agent("heronite-rs")
            .build()
            .into()
    })
}

/// Shared HTTP agent for downloads too.
pub(crate) fn http_agent() -> &'static ureq::Agent {
    agent()
}

pub(crate) fn http_get_text(url: &str, max: u64) -> Result<String> {
    let mut resp = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json, text/plain, */*")
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(403) | ureq::Error::StatusCode(429) => {
                anyhow!(tr!(
                    "GitHub API rate limit reached (HTTP {status}), try again later",
                    status = match e {
                        ureq::Error::StatusCode(c) => c,
                        _ => 0,
                    }
                ))
            }
            ureq::Error::StatusCode(c) => anyhow!("HTTP {c}"),
            e => anyhow!("{e}"),
        })?;
    resp.body_mut().with_config().limit(max).read_to_string().map_err(|e| anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CACHY: &str = include_str!("../tests/fixtures/cachy-releases.json");
    const GE: &str = include_str!("../tests/fixtures/ge-releases.json");

    #[test]
    fn latest_pattern() {
        assert!(ends_with_digits_dash_digits("Proton-8-25"));
        assert!(ends_with_digits_dash_digits("lutris-GE-Proton8-26"));
        assert!(!ends_with_digits_dash_digits("cachyos-11.0-20261005-slr"));
        assert!(ends_with_digits_dash_digits("GE-Proton11-7"));
        assert!(!ends_with_digits_dash_digits("8-"));
        assert!(!ends_with_digits_dash_digits("-5"));
    }

    #[test]
    fn parses_ge_proton() {
        let r = parse_releases(Repository::ProtonGe, GE).unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].version, "GE-Proton-latest");
        assert_eq!(r[1].version, "GE-Proton11-7");
        assert_eq!(r[0].download, r[1].download);
        assert!(r[1].download.as_deref().unwrap().ends_with("GE-Proton11-7-x86_64.tar.gz"));
        assert!(r[1].checksum.as_deref().unwrap().ends_with("GE-Proton11-7-x86_64.sha512sum"));
        assert!(r[1].downsize.unwrap() > 1_000_000);
        assert_eq!(r[1].date.len(), 10);
        assert!(r[1].release_notes_link.contains("/releases/tag/GE-Proton11-7"));
        assert_eq!(r[1].kind, "GE-Proton");
        assert!(r.iter().all(|x| !x.download.as_deref().unwrap().contains("aarch64")));
    }

    #[test]
    fn parses_cachyos_without_v3_or_arm() {
        let r = parse_releases(Repository::ProtonCachyos, CACHY).unwrap();
        assert_eq!(r[0].version, "Proton-CachyOS-latest");
        assert_eq!(r[1].version, "cachyos-11.0-20261005-slr");
        assert!(r[1].download.as_deref().unwrap().ends_with("-x86_64.tar.xz"));
        assert!(r[1].checksum.as_deref().unwrap().ends_with("-x86_64.sha512sum"));
    }

    #[test]
    fn empty_list_has_no_latest() {
        assert!(parse_releases(Repository::ProtonGe, "[]").unwrap().is_empty());
        assert!(parse_releases(Repository::ProtonGe, "{\"message\":\"rate limit\"}").is_err());
    }

    #[test]
    fn wine_types_get_prefix() {
        assert_eq!(version_name("Wine-Lutris", "lutris-8"), "Wine-lutris-8");
        assert_eq!(version_name("GE-Proton", "GE-Proton9-1"), "GE-Proton9-1");
    }
}
