//! "Clear cache" of the global settings (Heroic's `ClearCache` button).
//!
//! Heroic empties the wiki/third-party game info store and the Epic, GOG and
//! Amazon library/install caches. Only what this app can rebuild on its own is
//! cleared here: the third-party info (re-fetched on demand), the Epic caches
//! (the Epic page's refresh rebuilds them) and the resized-image cache.
//! GOG/Amazon caches stay (this app cannot refresh them) and so do logins,
//! installed games, game settings and the Collection data.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::paths::{AppPaths, DataPaths};

/// What a clear touched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearReport {
    pub files: usize,
    pub bytes: u64,
}

/// Single files cleared inside `store_cache/` (without `.json`).
const CACHE_FILES: [&str; 4] = ["wikigameinfo", "legendary_library", "legendary_gameinfo", "legendary_install_info"];

/// The files and folders [`clear_caches`] removes.
pub fn cache_targets(data_paths: &DataPaths, app: &AppPaths) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = CACHE_FILES.iter().map(|f| data_paths.store_cache(f)).collect();
    out.push(app.thumbs_dir());
    out
}

fn tree_size(path: &Path) -> (usize, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else { return (0, 0) };
    if meta.is_dir() {
        let Ok(rd) = fs::read_dir(path) else { return (0, 0) };
        rd.flatten().map(|e| tree_size(&e.path())).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    } else {
        (1, meta.len())
    }
}

/// Size of what [`clear_caches`] would remove.
pub fn cache_size(data_paths: &DataPaths, app: &AppPaths) -> ClearReport {
    let (files, bytes) = cache_targets(data_paths, app).iter().map(|p| tree_size(p)).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    ClearReport { files, bytes }
}

/// Removes the caches listed by [`cache_targets`]; missing ones are skipped.
pub fn clear_caches(data_paths: &DataPaths, app: &AppPaths) -> Result<ClearReport> {
    let mut report = ClearReport::default();
    for target in cache_targets(data_paths, app) {
        let (files, bytes) = tree_size(&target);
        if files == 0 {
            continue;
        }
        let res = if target.is_dir() { fs::remove_dir_all(&target) } else { fs::remove_file(&target) };
        match res {
            Ok(()) => {
                report.files += files;
                report.bytes += bytes;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", target.display())),
        }
    }
    Ok(report)
}

/// "12.3 MB" style size.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024. && u < UNITS.len() - 1 {
        v /= 1024.;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{} {}", heronite_i18n::decimal(v, 1), UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clears_only_the_rebuildable_caches() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("heroic"));
        let app = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        let cache = data_paths.root.join("store_cache");
        fs::create_dir_all(&cache).unwrap();
        for (f, body) in [
            ("wikigameinfo.json", "{}"),
            ("legendary_library.json", "{\"library\":[]}"),
            ("legendary_gameinfo.json", "{}"),
            ("gog_library.json", "{\"games\":[1]}"),
            ("legendary_games_override.json", "{}"),
        ] {
            fs::write(cache.join(f), body).unwrap();
        }
        fs::create_dir_all(app.thumbs_dir().join("ab")).unwrap();
        fs::write(app.thumbs_dir().join("ab/x"), [0u8; 100]).unwrap();
        fs::create_dir_all(data_paths.root.join("GamesConfig")).unwrap();
        fs::write(data_paths.root.join("GamesConfig/g.json"), "{}").unwrap();

        let before = cache_size(&data_paths, &app);
        assert_eq!(before.files, 4);
        assert_eq!(before.bytes, 2 + 14 + 2 + 100);
        let report = clear_caches(&data_paths, &app).unwrap();
        assert_eq!(report, before);
        assert!(!cache.join("wikigameinfo.json").exists());
        assert!(!cache.join("legendary_library.json").exists());
        assert!(!app.thumbs_dir().exists());
        // Kept: GOG cache, the Epic override list, game settings.
        assert!(cache.join("gog_library.json").exists());
        assert!(cache.join("legendary_games_override.json").exists());
        assert!(data_paths.root.join("GamesConfig/g.json").exists());
        // Second run: nothing left.
        assert_eq!(clear_caches(&data_paths, &app).unwrap(), ClearReport::default());
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }
}
