//! What the app downloads by itself:
//!
//! - when this PC has no Wine or Proton to run Windows games with, the newest
//!   GE-Proton release (a numbered one, not the `GE-Proton-latest` entry);
//! - with `autoUpdateLatestProton`, the new release of an installed `-latest`
//!   build (`GE-Proton-latest`, `Proton-CachyOS-latest`, ...). Only the
//!   sources that have one are checked, through [`checks`] (the release feed
//!   first, an API call only when it changed), so this costs nothing while
//!   nothing is published.

use heronite_data::DataPaths;

use crate::checks::{self, CheckMode, CheckOutcome, GitHub};
use crate::repos::Repository;
use crate::store;
use crate::types::WineVersionInfo;

/// The newest numbered GE-Proton that can be downloaded among `releases`.
pub fn newest_ge(releases: &[WineVersionInfo]) -> Option<&WineVersionInfo> {
    let kind = Repository::ProtonGe.kind();
    releases
        .iter()
        .filter(|r| r.kind == kind && !r.version.ends_with("-latest") && r.downloadable())
        .max_by(|a, b| a.date.cmp(&b.date).then_with(|| crate::default::natural_cmp(&a.version, &b.version)))
}

/// Brings the cached GE-Proton releases up to date when needed (one GitHub
/// call at most, none when the cache is recent) and gives the newest.
/// Blocking (network).
pub fn latest_ge(data_paths: &DataPaths) -> Result<WineVersionInfo, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let outcome = checks::check_source(data_paths, &GitHub, Repository::ProtonGe, CheckMode::Auto, now);
    let releases = store::read_releases(data_paths);
    match (newest_ge(&releases), outcome) {
        (Some(info), _) => Ok(info.clone()),
        (None, CheckOutcome::Failed(e)) => Err(e),
        (None, CheckOutcome::RateLimited { .. }) => Err("GitHub asks to wait before listing the GE-Proton releases".into()),
        (None, _) => Err("No GE-Proton release to download".into()),
    }
}

/// An installed build that follows its source's newest release.
fn installed_latest(r: &WineVersionInfo) -> bool {
    r.version.ends_with("-latest") && r.installed()
}

/// The sources with an installed `-latest` build: the only ones the
/// automatic update checks.
pub fn latest_sources(releases: &[WineVersionInfo]) -> Vec<Repository> {
    let mut repos = Vec::new();
    for repo in releases.iter().filter(|r| installed_latest(r)).filter_map(|r| Repository::from_kind(&r.kind)) {
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    repos
}

/// The installed `-latest` builds with a new release to download.
pub fn latest_updates(releases: &[WineVersionInfo]) -> Vec<WineVersionInfo> {
    releases.iter().filter(|r| installed_latest(r) && r.has_update() && r.downloadable()).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(version: &str, date: &str) -> WineVersionInfo {
        WineVersionInfo {
            version: version.into(),
            kind: "GE-Proton".into(),
            date: date.into(),
            download: Some(format!("https://x/{version}.tar.gz")),
            downsize: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn the_newest_numbered_ge_proton() {
        let mut other = rel("proton-cachyos-10", "2026-12-01");
        other.kind = "Proton-CachyOS".into();
        let mut no_file = rel("GE-Proton11-12", "2026-11-01");
        no_file.downsize = None;
        let list = [
            rel("GE-Proton-latest", "2026-10-19"),
            other,
            no_file,
            rel("GE-Proton11-10", "2026-10-19"),
            rel("GE-Proton11-11", "2026-10-19"),
            rel("GE-Proton11-9", "2026-10-14"),
        ];
        assert_eq!(newest_ge(&list).unwrap().version, "GE-Proton11-11");
        assert!(newest_ge(&[]).is_none());
    }

    #[test]
    fn only_installed_latest_builds_are_followed() {
        let installed = |mut r: WineVersionInfo, update: bool| {
            r.install_dir = Some(format!("/t/{}", r.version));
            r.is_installed = Some(true);
            r.has_update = Some(update);
            r
        };
        let mut cachy = installed(rel("Proton-CachyOS-latest", "2026-10-20"), false);
        cachy.kind = "Proton-CachyOS".into();
        let list = [
            installed(rel("GE-Proton-latest", "2026-10-19"), true),
            cachy,
            // A numbered build never updates; a -latest one not installed is not followed.
            installed(rel("GE-Proton11-11", "2026-10-19"), true),
            rel("DW-Proton-latest", "2026-10-01"),
        ];
        assert_eq!(latest_sources(&list), [Repository::ProtonGe, Repository::ProtonCachyos]);
        let updates: Vec<_> = latest_updates(&list).into_iter().map(|r| r.version).collect();
        assert_eq!(updates, ["GE-Proton-latest"]);
    }
}
