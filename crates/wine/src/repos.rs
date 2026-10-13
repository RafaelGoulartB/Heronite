//! The Proton sources the Wine Manager offers and the parsing of their GitHub
//! releases (GE-Proton, Proton-CachyOS, Valve's Proton and Wine-Lutris are
//! ports of Heroic's `wine/manager/downloader/{constants,utilities}.ts`).
//!
//! Every source is a row of [`SPECS`]: where its releases live and which
//! release file is the build for this machine. Adding a source that publishes
//! a plain `.tar.*` archive per release only needs a new row.

use anyhow::{Context, Result};
use heronite_i18n::tr;
use serde::Deserialize;

use crate::types::WineVersionInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Repository {
    ProtonCachyos,
    ProtonGe,
    DwProton,
    UmuProton,
    ProtonEm,
    ProtonSarek,
    /// Valve's Proton (not offered in the Wine Manager UI on Linux).
    Proton,
    /// Lutris Wine builds (not offered in the UI on Linux).
    WineLutris,
}

/// Which file of a release is the build for this machine.
struct Spec {
    repo: Repository,
    /// `type` stored in `wine-downloader-info.json` (Heroic's `Type` for the
    /// sources Heroic knows), also the name shown in the UI.
    kind: &'static str,
    /// `owner/name` on GitHub.
    github: &'static str,
    /// Accepted archive endings.
    archives: &'static [&'static str],
    /// Part of the x86-64 archive name; `None`: any archive that is not the
    /// ARM build.
    x86_64: Option<&'static str>,
    /// Part of the ARM archive name; `None`: the source has no ARM builds.
    arm64: Option<&'static str>,
    /// Other flavours published in the same release, never picked.
    skip: &'static [&'static str],
}

const TAR_GZ: &[&str] = &[".tar.gz"];
const TAR_XZ: &[&str] = &[".tar.xz"];
const ANY_TAR: &[&str] = &[".tar.gz", ".tar.xz"];

const SPECS: [Spec; 8] = [
    Spec {
        repo: Repository::ProtonCachyos,
        kind: "Proton-CachyOS",
        github: "CachyOS/proton-cachyos",
        archives: TAR_XZ,
        // `-x86_64.tar` leaves out the `x86_64_v3` builds.
        x86_64: Some("-x86_64.tar"),
        arm64: Some("-arm64.tar"),
        skip: &[],
    },
    Spec {
        repo: Repository::ProtonGe,
        kind: "GE-Proton",
        github: "GloriousEggroll/proton-ge-custom",
        archives: TAR_GZ,
        // Older releases have a single archive without the architecture.
        x86_64: None,
        arm64: Some("-aarch64"),
        skip: &[],
    },
    Spec {
        repo: Repository::DwProton,
        kind: "DW-Proton",
        github: "dawn-winery/dwproton-mirror",
        archives: TAR_XZ,
        x86_64: Some("-x86_64.tar"),
        arm64: None,
        skip: &[],
    },
    Spec {
        repo: Repository::UmuProton,
        kind: "UMU-Proton",
        github: "Open-Wine-Components/umu-proton",
        archives: TAR_GZ,
        x86_64: None,
        arm64: None,
        skip: &[],
    },
    Spec {
        repo: Repository::ProtonEm,
        kind: "Proton-EM",
        github: "BananaWorks07/Proton",
        archives: TAR_XZ,
        x86_64: None,
        arm64: None,
        skip: &[],
    },
    Spec {
        repo: Repository::ProtonSarek,
        kind: "Proton-Sarek",
        github: "pythonlover02/Proton-Sarek",
        archives: TAR_GZ,
        x86_64: None,
        arm64: None,
        // The same build with asynchronous shader compilation.
        skip: &["-async"],
    },
    Spec {
        repo: Repository::Proton,
        kind: "Proton",
        github: "ValveSoftware/Proton",
        archives: ANY_TAR,
        x86_64: None,
        arm64: None,
        skip: &[],
    },
    Spec {
        repo: Repository::WineLutris,
        kind: "Wine-Lutris",
        github: "lutris/wine",
        archives: ANY_TAR,
        x86_64: None,
        arm64: None,
        skip: &[],
    },
];

/// The sources of the Wine Manager, in the order it shows them.
const OFFERED: [Repository; 6] = [
    Repository::ProtonCachyos,
    Repository::ProtonGe,
    Repository::DwProton,
    Repository::UmuProton,
    Repository::ProtonEm,
    Repository::ProtonSarek,
];

const ARM: bool = cfg!(target_arch = "aarch64");

impl Repository {
    fn spec(self) -> &'static Spec {
        SPECS.iter().find(|s| s.repo == self).expect("every repository has a spec")
    }

    /// The sources the Wine Manager offers on this machine, in order (only
    /// the ones with builds for its architecture).
    pub fn offered() -> &'static [Repository] {
        static LIST: std::sync::OnceLock<Vec<Repository>> = std::sync::OnceLock::new();
        LIST.get_or_init(|| OFFERED.into_iter().filter(|r| !ARM || r.spec().arm64.is_some()).collect())
    }

    /// `owner/name` on GitHub.
    pub fn github(self) -> &'static str {
        self.spec().github
    }

    /// GitHub releases endpoint.
    pub fn api_url(self) -> String {
        format!("https://api.github.com/repos/{}/releases", self.github())
    }

    /// The public release feed (not part of GitHub's API).
    pub fn feed_url(self) -> String {
        format!("https://github.com/{}/releases.atom", self.github())
    }

    /// `type` stored in `wine-downloader-info.json`, also the name shown.
    pub fn kind(self) -> &'static str {
        self.spec().kind
    }

    pub fn from_kind(kind: &str) -> Option<Self> {
        SPECS.iter().find(|s| s.kind == kind).map(|s| s.repo)
    }

    /// What the source is, under its name.
    pub fn explanation(self) -> &'static str {
        match self {
            Repository::ProtonGe => tr!("GE-Proton is a Proton variant created by Glorious Eggroll. It is meant to be used along with the umu launcher (default in Heroic)."),
            Repository::ProtonCachyos => tr!("Proton-CachyOS is a Proton variant maintained by the CachyOS team. It includes extra tools like DXVK-Sarek and D7VK."),
            Repository::DwProton => tr!("Dawn Winery's Proton: Proton Experimental with extra game fixes."),
            Repository::UmuProton => tr!("Valve's Proton, packaged by the umu project to run games outside Steam."),
            Repository::ProtonEm => tr!("Proton with patches that are still being tested, such as FSR 4 and HDR. Some releases are pre-releases."),
            Repository::ProtonSarek => tr!("Proton with DXVK-Sarek, for older graphics cards without Vulkan 1.3 support."),
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

/// The archive of a release built for this machine.
fn pick_archive<'a>(spec: &Spec, assets: &'a [GhAsset], arm: bool) -> Option<&'a GhAsset> {
    assets.iter().find(|a| {
        let name = a.name.as_str();
        let is_arm = spec.arm64.is_some_and(|t| name.contains(t));
        let wanted = match (arm, spec.x86_64) {
            (true, _) => is_arm,
            (false, Some(token)) => name.contains(token),
            (false, None) => !is_arm,
        };
        wanted && spec.archives.iter().any(|e| name.ends_with(e)) && !spec.skip.iter().any(|s| name.contains(s))
    })
}

/// The checksum published next to `archive`: `<archive>.sha512sum` or the
/// archive name without its extension, SHA-512 or SHA-256.
fn pick_checksum<'a>(spec: &Spec, assets: &'a [GhAsset], archive: &str) -> Option<&'a GhAsset> {
    let stem = spec.archives.iter().find_map(|e| archive.strip_suffix(e)).unwrap_or(archive);
    let names: Vec<String> =
        [archive, stem].iter().flat_map(|base| ["sha512sum", "sha256sum"].map(|ext| format!("{base}.{ext}"))).collect();
    names.iter().find_map(|n| assets.iter().find(|a| &a.name == n))
}

/// Parses the GitHub releases response. Returns the releases (newest first,
/// as GitHub lists them) with the synthetic `<type>-latest` entry first: the
/// newest plain build that is not a pre-release.
pub fn parse_releases(repo: Repository, json: &str) -> Result<Vec<WineVersionInfo>> {
    parse_releases_for(repo, json, ARM)
}

fn parse_releases_for(repo: Repository, json: &str, arm: bool) -> Result<Vec<WineVersionInfo>> {
    let list: Vec<GhRelease> = serde_json::from_str(json).context("unexpected GitHub releases response")?;
    let spec = repo.spec();
    let mut releases = Vec::with_capacity(list.len() + 1);
    let mut stable = Vec::with_capacity(list.len());
    for r in list {
        let mut info = WineVersionInfo {
            version: version_name(spec.kind, &r.tag_name),
            kind: spec.kind.to_owned(),
            date: r.published_at.split('T').next().unwrap_or_default().to_owned(),
            release_notes_link: r.html_url,
            ..Default::default()
        };
        if let Some(archive) = pick_archive(spec, &r.assets, arm) {
            info.download = Some(archive.browser_download_url.clone());
            info.downsize = Some(archive.size);
            info.checksum = pick_checksum(spec, &r.assets, &archive.name).map(|a| a.browser_download_url.clone());
        }
        stable.push(!r.prerelease && info.download.is_some());
        releases.push(info);
    }

    let candidates = || releases.iter().zip(&stable).filter(|(_, ok)| **ok).map(|(r, _)| r);
    if let Some(latest) = candidates().find(|r| ends_with_digits_dash_digits(&r.version)).or_else(|| candidates().next()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    const CACHY: &str = include_str!("../tests/fixtures/cachy-releases.json");
    const GE: &str = include_str!("../tests/fixtures/ge-releases.json");
    const DW: &str = include_str!("../tests/fixtures/dw-releases.json");
    const UMU: &str = include_str!("../tests/fixtures/umu-releases.json");
    const EM: &str = include_str!("../tests/fixtures/em-releases.json");
    const SAREK: &str = include_str!("../tests/fixtures/sarek-releases.json");

    fn file(url: Option<&str>) -> &str {
        url.unwrap().rsplit('/').next().unwrap()
    }

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
        let r = parse_releases_for(Repository::ProtonGe, GE, false).unwrap();
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
        let arm = parse_releases_for(Repository::ProtonGe, GE, true).unwrap();
        assert_eq!(file(arm[1].download.as_deref()), "GE-Proton11-7-aarch64.tar.gz");
        assert_eq!(file(arm[1].checksum.as_deref()), "GE-Proton11-7-aarch64.sha512sum");
    }

    #[test]
    fn parses_cachyos_without_v3_or_arm() {
        let r = parse_releases_for(Repository::ProtonCachyos, CACHY, false).unwrap();
        assert_eq!(r[0].version, "Proton-CachyOS-latest");
        assert_eq!(r[1].version, "cachyos-11.0-20261005-slr");
        assert!(r[1].download.as_deref().unwrap().ends_with("-x86_64.tar.xz"));
        assert!(r[1].checksum.as_deref().unwrap().ends_with("-x86_64.sha512sum"));
    }

    #[test]
    fn parses_dw_proton_without_the_torrent() {
        let r = parse_releases_for(Repository::DwProton, DW, false).unwrap();
        assert_eq!(r[0].version, "DW-Proton-latest");
        assert_eq!(r[1].version, "dwproton-11.0-14");
        assert_eq!(file(r[1].download.as_deref()), "dwproton-11.0-14-x86_64.tar.xz");
        assert_eq!(file(r[1].checksum.as_deref()), "dwproton-11.0-14-x86_64.sha512sum");
        assert_eq!(r[0].download, r[1].download);
        // No ARM builds: nothing to install there.
        assert!(parse_releases_for(Repository::DwProton, DW, true).unwrap().iter().all(|r| r.download.is_none()));
    }

    #[test]
    fn parses_umu_proton() {
        let r = parse_releases_for(Repository::UmuProton, UMU, false).unwrap();
        assert_eq!(r[1].version, "UMU-Proton-10.0-4");
        assert_eq!(file(r[1].download.as_deref()), "UMU-Proton-10.0-4.tar.gz");
        assert_eq!(file(r[1].checksum.as_deref()), "UMU-Proton-10.0-4.sha512sum");
    }

    #[test]
    fn proton_em_uses_sha256_and_latest_skips_pre_releases() {
        let r = parse_releases_for(Repository::ProtonEm, EM, false).unwrap();
        assert_eq!(r[1].version, "EM-10.0-37-HDR");
        assert_eq!(file(r[1].checksum.as_deref()), "proton-EM-10.0-37-HDR.sha256sum");
        assert_eq!(r[0].download, r[1].download);
        let pre = r#"[{"tag_name":"EM-11.0-1-beta1","prerelease":true,"assets":[{"name":"proton-EM-11.0-1-beta1.tar.xz","browser_download_url":"https://x/b.tar.xz","size":5}]},
                     {"tag_name":"EM-10.0-37","assets":[{"name":"proton-EM-10.0-37.tar.xz","browser_download_url":"https://x/s.tar.xz","size":5}]}]"#;
        let r = parse_releases_for(Repository::ProtonEm, pre, false).unwrap();
        assert_eq!(r.len(), 3, "the beta is listed");
        assert_eq!(r[0].download.as_deref(), Some("https://x/s.tar.xz"), "-latest is the stable build");
    }

    #[test]
    fn proton_sarek_skips_the_async_build() {
        let r = parse_releases_for(Repository::ProtonSarek, SAREK, false).unwrap();
        assert_eq!(r[1].version, "Proton-Sarek10-29");
        assert_eq!(file(r[1].download.as_deref()), "Proton-Sarek10-29.tar.gz");
        assert!(r[1].checksum.is_none());
    }

    #[test]
    fn every_offered_source_is_known() {
        let offered = Repository::offered();
        assert!(offered.len() >= 2 && offered[0] == Repository::ProtonCachyos);
        for r in offered {
            assert_eq!(Repository::from_kind(r.kind()), Some(*r));
            assert!(r.api_url().starts_with("https://api.github.com/repos/") && r.feed_url().ends_with("/releases.atom"));
            assert!(!r.kind().contains("Wine"), "Proton builds go to tools/proton");
        }
        let kinds: std::collections::HashSet<_> = SPECS.iter().map(|s| s.kind).collect();
        assert_eq!(kinds.len(), SPECS.len());
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
