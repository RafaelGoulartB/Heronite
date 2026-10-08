//! `store/wine-downloader-info.json`: the release cache shared with Heroic
//! (`wineDownloaderInfoStore`, key `wine-releases`). Port of
//! `updateWineVersionInfos` and the store updates of install/remove.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use heronite_data::{json_store, HeroicPaths};
use serde_json::Value;

use crate::repos::{self, Repository};
use crate::types::WineVersionInfo;

const KEY: &str = "wine-releases";

pub fn store_path(heroic: &HeroicPaths) -> PathBuf {
    heroic.root.join("store/wine-downloader-info.json")
}

/// Cached releases (empty when the file or key is missing). Entries that do
/// not parse are skipped.
pub fn read_releases(heroic: &HeroicPaths) -> Vec<WineVersionInfo> {
    read_from(&store_path(heroic))
}

fn read_from(path: &Path) -> Vec<WineVersionInfo> {
    let doc = json_store::read_value(path).unwrap_or(Value::Null);
    doc.get(KEY)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect())
        .unwrap_or_default()
}

fn has_key(heroic: &HeroicPaths) -> bool {
    json_store::read_value(&store_path(heroic)).map_or(false, |v| v.get(KEY).is_some())
}

fn write_releases(heroic: &HeroicPaths, list: &[WineVersionInfo]) -> Result<()> {
    let value = serde_json::to_value(list)?;
    json_store::update_value(&store_path(heroic), |doc| {
        doc[KEY] = value;
        Ok(())
    })
}

fn dir_exists(path: &str) -> bool {
    !path.is_empty() && Path::new(path).exists()
}

/// The merge of `updateWineVersionInfos`: keeps what is installed, marks
/// `hasUpdate` when the `-latest` checksum changed, adds new releases and
/// replaces the `-latest` entries; sorted by date, newest first.
/// `exists` tells whether an `installDir` is still on disk.
pub fn merge_releases(
    old_releases: Vec<WineVersionInfo>,
    mut fetched: Vec<WineVersionInfo>,
    exists: impl Fn(&str) -> bool,
) -> Vec<WineVersionInfo> {
    let mut to_store: Vec<WineVersionInfo> = Vec::new();
    for old in old_releases {
        to_store.push(old.clone());
        let index = fetched.iter().position(|release| {
            if release.kind == "GE-Proton" {
                // The "Proton" prefix got dropped from the version string.
                if format!("Proton-{}", release.version) == old.version {
                    return true;
                }
                // "Proton-GE-latest" was renamed to "GE-Proton-latest".
                if release.version == "GE-Proton-latest" && old.version == "Proton-GE-latest" {
                    return true;
                }
            }
            release.version == old.version
        });
        if let Some(dir) = old.install_dir.as_deref() {
            if exists(dir) {
                match index {
                    Some(i) => {
                        let r = &mut fetched[i];
                        r.install_dir = old.install_dir.clone();
                        r.is_installed = old.is_installed;
                        r.disksize = old.disksize;
                        if r.checksum != old.checksum || old.has_update == Some(true) {
                            r.has_update = Some(true);
                        }
                    }
                    None => fetched.push(old.clone()),
                }
            }
        }
    }
    for release in fetched {
        let found = to_store
            .iter()
            .position(|o| o.version == release.version || o.version == format!("Proton-{}", release.version));
        match found {
            None => to_store.push(release),
            Some(i) if release.version.ends_with("-latest") => to_store[i] = release,
            Some(_) => {}
        }
    }
    // Stable sort, newest first (dates are `YYYY-MM-DD`).
    to_store.sort_by(|a, b| b.date.cmp(&a.date));
    to_store
}

pub struct RefreshOutcome {
    pub releases: Vec<WineVersionInfo>,
    /// One message per repository that could not be fetched.
    pub errors: Vec<String>,
}

/// Fetches `repos` from GitHub and merges them into the cache. The cache is
/// left untouched when every repository failed. Blocking (network).
pub fn refresh(heroic: &HeroicPaths, repos_to_fetch: &[Repository], count: usize) -> Result<RefreshOutcome> {
    let mut fetched = Vec::new();
    let mut errors = Vec::new();
    for repo in repos_to_fetch {
        match repos::fetch_releases(*repo, count) {
            Ok(mut r) => fetched.append(&mut r),
            Err(e) => {
                log::warn!("wine manager: {e}");
                errors.push(format!("{}: {e}", repo.kind()));
            }
        }
    }
    if fetched.is_empty() && !errors.is_empty() {
        return Err(anyhow!(errors.join("\n")));
    }
    Ok(RefreshOutcome { releases: apply_fetched(heroic, fetched)?, errors })
}

/// Merges already fetched releases into the cache file.
pub fn apply_fetched(heroic: &HeroicPaths, fetched: Vec<WineVersionInfo>) -> Result<Vec<WineVersionInfo>> {
    let merged = if has_key(heroic) {
        merge_releases(read_releases(heroic), fetched, dir_exists)
    } else {
        fetched
    };
    write_releases(heroic, &merged)?;
    Ok(merged)
}

/// Replaces the cached entry of `info.version` (after an install/update).
pub fn mark_installed(heroic: &HeroicPaths, info: &WineVersionInfo) -> Result<()> {
    let mut list = read_releases(heroic);
    match list.iter().position(|r| r.version == info.version) {
        Some(i) => list[i] = info.clone(),
        None => list.push(info.clone()),
    }
    write_releases(heroic, &list)
}

/// Marks `version` as not installed (after a removal).
pub fn mark_removed(heroic: &HeroicPaths, version: &str) -> Result<()> {
    let mut list = read_releases(heroic);
    let r = list
        .iter_mut()
        .find(|r| r.version == version)
        .ok_or_else(|| anyhow!("{version} is not in wine-downloader-info.json"))?;
    r.is_installed = Some(false);
    r.install_dir = Some(String::new());
    r.disksize = 0;
    r.has_update = Some(false);
    write_releases(heroic, &list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(v: &str, date: &str, checksum: &str) -> WineVersionInfo {
        WineVersionInfo {
            version: v.into(),
            kind: "GE-Proton".into(),
            date: date.into(),
            download: Some(format!("https://x/{v}.tar.gz")),
            downsize: Some(10),
            checksum: Some(checksum.into()),
            ..Default::default()
        }
    }

    fn installed(mut r: WineVersionInfo, dir: &str) -> WineVersionInfo {
        r.install_dir = Some(dir.into());
        r.is_installed = Some(true);
        r.disksize = 99;
        r
    }

    #[test]
    fn merge_keeps_install_and_flags_update() {
        let old = vec![
            installed(rel("GE-Proton-latest", "2026-08-01", "sumA"), "/t/GE-Proton-latest"),
            rel("GE-Proton11-5", "2026-08-01", "x"),
        ];
        let fetched = vec![
            rel("GE-Proton-latest", "2026-09-01", "sumB"),
            rel("GE-Proton11-6", "2026-09-01", "sumB"),
            rel("GE-Proton11-5", "2026-08-01", "x"),
        ];
        let merged = merge_releases(old, fetched, |p| p == "/t/GE-Proton-latest");
        let names: Vec<_> = merged.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(names, ["GE-Proton-latest", "GE-Proton11-6", "GE-Proton11-5"]);
        let latest = &merged[0];
        assert_eq!(latest.date, "2026-09-01");
        assert_eq!(latest.install_dir.as_deref(), Some("/t/GE-Proton-latest"));
        assert_eq!(latest.is_installed, Some(true));
        assert_eq!(latest.disksize, 99);
        assert_eq!(latest.has_update, Some(true));
        assert!(!merged[1].installed());
    }

    #[test]
    fn merge_same_checksum_no_update_and_missing_dir_forgotten() {
        let old = vec![
            installed(rel("GE-Proton-latest", "2026-09-01", "sumA"), "/t/gone"),
            installed(rel("GE-Proton11-4", "2026-07-01", "z"), "/t/ok"),
        ];
        let fetched = vec![rel("GE-Proton-latest", "2026-09-01", "sumA")];
        let merged = merge_releases(old, fetched, |p| p == "/t/ok");
        // `-latest` replaced by the fetched one, whose dir no longer exists.
        let latest = merged.iter().find(|r| r.version == "GE-Proton-latest").unwrap();
        assert_eq!(latest.install_dir, None);
        assert_eq!(latest.has_update, None);
        // Installed entry that GitHub no longer lists survives.
        assert!(merged.iter().any(|r| r.version == "GE-Proton11-4" && r.installed()));
    }

    #[test]
    fn merge_renamed_ge_proton() {
        let mut old = installed(rel("Proton-GE-latest", "2026-01-01", "a"), "/t/old");
        old.version = "Proton-GE-latest".into();
        let merged = merge_releases(vec![old], vec![rel("GE-Proton-latest", "2026-02-01", "a")], |_| true);
        let r = merged.iter().find(|r| r.version == "GE-Proton-latest").unwrap();
        assert_eq!(r.install_dir.as_deref(), Some("/t/old"));
    }

    #[test]
    fn cache_roundtrip_format_and_unknown_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(tmp.path());
        let path = store_path(&heroic);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "{\n\t\"wine-releases\": [\n\t\t{\n\t\t\t\"version\": \"GE-Proton11-6\",\n\t\t\t\"type\": \"GE-Proton\",\n\t\t\t\"date\": \"2026-08-28\",\n\t\t\t\"disksize\": 0,\n\t\t\t\"future\": 1\n\t\t}\n\t],\n\t\"other\": true\n}",
        )
        .unwrap();
        let before = read_releases(&heroic);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].extra.get("future"), Some(&Value::from(1)));
        assert!(!before[0].installed());

        let mut inst = before[0].clone();
        inst.is_installed = Some(true);
        inst.install_dir = Some("/tools/proton/GE-Proton11-6".into());
        inst.disksize = 42;
        mark_installed(&heroic, &inst).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\n\t\t\t\"isInstalled\": true"), "{text}");
        assert!(text.contains("\"other\": true") && text.contains("\"future\": 1"));
        assert!(!text.ends_with('\n'));
        assert!(read_releases(&heroic)[0].installed());

        mark_removed(&heroic, "GE-Proton11-6").unwrap();
        let after = &read_releases(&heroic)[0];
        assert!(!after.installed());
        assert_eq!(after.install_dir.as_deref(), Some(""));
        assert_eq!(after.has_update, Some(false));
        assert!(mark_removed(&heroic, "nope").is_err());
    }

    #[test]
    fn reads_real_cache_if_present() {
        let real = HeroicPaths::detect();
        let list = read_releases(&real);
        if list.is_empty() {
            return;
        }
        assert!(list.iter().any(|r| r.kind == "GE-Proton" && r.downloadable()));
    }
}
