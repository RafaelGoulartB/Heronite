//! What each file of the data folder and of the app state is, for a backup:
//! one of the [`Part`]s a backup holds, or something it leaves out because it
//! is downloaded or rebuilt again ([`Left`]).
//!
//! The rules name what to leave out; everything else is in. A file a future
//! version adds is then backed up until a rule says otherwise, which errs on
//! the safe side.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// What a backup holds, chosen in Settings → Backups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    /// The library, statuses, play sessions, journal, game settings, store
    /// libraries the app cannot fetch again. Always in.
    Collection,
    /// The app state of `~/.config/heronite`: presets, appearance, overlay.
    Settings,
    /// Covers and backgrounds picked by hand (`local_library/art`).
    Art,
    /// Epic, GOG and Amazon sign-ins.
    Logins,
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Collection, Part::Settings, Part::Art, Part::Logins];

    /// The key in `settings.json` `backup.exclude` and in manifests.
    pub fn key(self) -> &'static str {
        match self {
            Part::Collection => "collection",
            Part::Settings => "settings",
            Part::Art => "art",
            Part::Logins => "logins",
        }
    }

    pub fn parse(key: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The parts a backup with `exclude` (from the settings) holds.
    pub fn included(exclude: &[String]) -> Vec<Part> {
        Part::ALL.into_iter().filter(|p| *p == Part::Collection || !exclude.iter().any(|e| e == p.key())).collect()
    }
}

/// Why a file is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Left {
    /// Wine / Proton builds, DXVK, runtimes, store tools: downloaded again.
    Tools,
    /// Downloaded art, resized copies, screenshot previews, Steam heroes.
    Images,
    /// Store and wiki info, legendary's metadata, download queue state.
    Caches,
    Logs,
    /// Temp files, old `.bak` copies, runtime markers.
    Temp,
    /// The home folders games in the sandbox write to (`sandbox/`): left out
    /// like the games' prefixes, which they belong with.
    Sandbox,
}

impl Left {
    pub const ALL: [Left; 6] = [Left::Tools, Left::Images, Left::Caches, Left::Logs, Left::Temp, Left::Sandbox];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    In(Part),
    Out(Left),
}

/// The two folders a backup copies from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The data folder (`DataPaths::root`).
    Data,
    /// The app state folder (`AppPaths::config`).
    Config,
}

impl Side {
    pub fn dir_name(self) -> &'static str {
        match self {
            Side::Data => "data",
            Side::Config => "config",
        }
    }
}

/// Store files under `store_cache/` this app cannot fetch again on its own
/// (`heronite_data::maintenance` keeps them too), or that hold work not yet
/// sent (GOG play time).
const KEPT_STORE_CACHE: [&str; 5] =
    ["gog_library.json", "nile_library.json", "steam_library.json", "gog_playtime_sync_queue.json", "gog_playtime_import.json"];

const LOGINS: [&str; 5] = [
    "legendaryConfig/legendary/user.json",
    "gog_store/auth.json",
    "nile_config/nile/user.json",
    "nile_config/nile/current_user.json",
    "local_library/achievement-accounts.json",
];

/// Files the app writes while it runs, or that are stale copies.
fn is_temp(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.ends_with(".tmp") || name.ends_with(".partial") || name.contains(".bak-") || name.ends_with(".bak") || name.ends_with('~')
}

fn under(rel: &str, dir: &str) -> bool {
    rel == dir || rel.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// Class of `rel` (relative to the data folder, `/`-separated).
pub fn classify_data(rel: &str) -> Class {
    if under(rel, "tools") {
        return Class::Out(Left::Tools);
    }
    if under(rel, "logs") {
        return Class::Out(Left::Logs);
    }
    if under(rel, "sandbox") {
        return Class::Out(Left::Sandbox);
    }
    if ["images-cache", "local_library/heroes", "local_library/screenshot-thumbnails"].iter().any(|d| under(rel, d)) {
        return Class::Out(Left::Images);
    }
    if is_temp(rel) || rel == "local_library/running.json" {
        return Class::Out(Left::Temp);
    }
    if LOGINS.contains(&rel) {
        return Class::In(Part::Logins);
    }
    if under(rel, "local_library/art") || under(rel, "local_library/metadata-art") {
        return Class::In(Part::Art);
    }
    if let Some(name) = rel.strip_prefix("store_cache/") {
        return if KEPT_STORE_CACHE.contains(&name) { Class::In(Part::Collection) } else { Class::Out(Left::Caches) };
    }
    let caches = [
        "local_library/steam-details",
        "legendaryConfig/legendary/metadata",
        "legendaryConfig/legendary/tmp",
        "save-restore-undo",
        "store/download-manager.json",
        "store/wine-downloader-info.json",
        "store/wine-release-checks.json",
        "areweanticheatyet.json",
        "known_fixes.json",
    ];
    if caches.iter().any(|d| under(rel, d)) {
        return Class::Out(Left::Caches);
    }
    Class::In(Part::Collection)
}

/// Class of `rel` (relative to the app state folder).
pub fn classify_config(rel: &str) -> Class {
    if under(rel, "logs") {
        return Class::Out(Left::Logs);
    }
    if is_temp(rel) || ["epic-move.json", crate::restore::PENDING_FILE, "wine-releases-fetched"].contains(&rel) {
        return Class::Out(Left::Temp);
    }
    Class::In(Part::Settings)
}

pub fn classify(side: Side, rel: &str) -> Class {
    match side {
        Side::Data => classify_data(rel),
        Side::Config => classify_config(rel),
    }
}

/// A regular file found by [`walk`].
#[derive(Clone, Debug)]
pub struct Found {
    /// Relative to the walked root, `/`-separated.
    pub rel: String,
    pub path: PathBuf,
    pub size: u64,
    /// Milliseconds since the epoch (0 when unknown).
    pub mtime_ms: i64,
    pub class: Class,
}

/// Every regular file under `root` with its class. Symlinks are not
/// followed nor listed (the data folder holds none the app needs back), and
/// a folder that is left out entirely is not entered unless `enter_left`.
pub fn walk(root: &Path, side: Side, enter_left: bool) -> Vec<Found> {
    let mut out = Vec::new();
    walk_into(root, "", side, enter_left, &mut out);
    out
}

fn walk_into(dir: &Path, prefix: &str, side: Side, enter_left: bool, out: &mut Vec<Found>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let Ok(ft) = entry.file_type() else { continue };
        let class = classify(side, &rel);
        if ft.is_dir() {
            if enter_left || !matches!(class, Class::Out(_)) || has_kept_children(side, &rel) {
                walk_into(&entry.path(), &rel, side, enter_left, out);
            }
        } else if ft.is_file() {
            let Ok(meta) = entry.metadata() else { continue };
            out.push(Found { rel, path: entry.path(), size: meta.len(), mtime_ms: mtime_ms(&meta), class });
        }
    }
}

/// Left-out folders that still hold files a backup keeps.
fn has_kept_children(side: Side, rel: &str) -> bool {
    side == Side::Data && rel == "store_cache"
}

pub(crate) fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_rules() {
        use Class::*;
        for (rel, want) in [
            ("local_library/library.json", In(Part::Collection)),
            ("local_library/sessions.json", In(Part::Collection)),
            ("GamesConfig/x.json", In(Part::Collection)),
            ("config.json", In(Part::Collection)),
            ("store/config.json", In(Part::Collection)),
            ("legendaryConfig/legendary/installed.json", In(Part::Collection)),
            ("legendaryConfig/legendary/manifests/a.manifest", In(Part::Collection)),
            ("store_cache/gog_library.json", In(Part::Collection)),
            ("store_cache/gog_playtime_sync_queue.json", In(Part::Collection)),
            ("legendaryConfig/legendary/user.json", In(Part::Logins)),
            ("gog_store/auth.json", In(Part::Logins)),
            ("nile_config/nile/user.json", In(Part::Logins)),
            ("local_library/achievement-accounts.json", In(Part::Logins)),
            ("local_library/achievements/gog/1207658924.json", In(Part::Collection)),
            ("local_library/art/sideload_x/hero-1.png", In(Part::Art)),
            ("local_library/metadata-art/x/h.jpg", In(Part::Art)),
            ("tools/proton/GE/bin/wine", Out(Left::Tools)),
            ("logs/launcher/a.log", Out(Left::Logs)),
            ("sandbox/game-g1/.local/share/save.dat", Out(Left::Sandbox)),
            ("images-cache/abc", Out(Left::Images)),
            ("local_library/heroes/1.jpg", Out(Left::Images)),
            ("local_library/screenshot-thumbnails/a", Out(Left::Images)),
            ("store_cache/humble_deals.json", Out(Left::Caches)),
            ("store_cache/legendary_library.json", Out(Left::Caches)),
            ("legendaryConfig/legendary/metadata/x.json", Out(Left::Caches)),
            ("local_library/steam-details/1.english.json", Out(Left::Caches)),
            ("store/download-manager.json", Out(Left::Caches)),
            ("local_library/running.json", Out(Left::Temp)),
            ("local_library/statuses.json.bak-before-unnumber", Out(Left::Temp)),
            ("sideload_apps/library.json.bak-bloodborne-20261007011623", Out(Left::Temp)),
            ("store/.config.json.heronite-rs-12-1.tmp", Out(Left::Temp)),
        ] {
            assert_eq!(classify_data(rel), want, "{rel}");
        }
        // Only whole path components match.
        assert_eq!(classify_data("toolsy.json"), In(Part::Collection));
        assert_eq!(classify_data("local_library/art.json"), In(Part::Collection));
        assert_eq!(classify_data("GamesConfig/logs.json"), In(Part::Collection));
    }

    #[test]
    fn config_rules() {
        assert_eq!(classify_config("ui-state.json"), Class::In(Part::Settings));
        assert_eq!(classify_config("appearance.json"), Class::In(Part::Settings));
        assert_eq!(classify_config("logs/x.log"), Class::Out(Left::Logs));
        assert_eq!(classify_config("epic-move.json"), Class::Out(Left::Temp));
        assert_eq!(classify_config(crate::restore::PENDING_FILE), Class::Out(Left::Temp));
    }

    #[test]
    fn included_parts() {
        assert_eq!(Part::included(&[]), Part::ALL);
        assert_eq!(Part::included(&["art".into(), "collection".into()]), [Part::Collection, Part::Settings, Part::Logins]);
        assert_eq!(Part::parse("logins"), Some(Part::Logins));
    }

    #[test]
    fn walk_skips_left_out_folders_but_keeps_store_files() {
        let d = tempfile::tempdir().unwrap();
        for p in ["tools/a/b", "store_cache/gog_library.json", "store_cache/x.json", "local_library/library.json"] {
            let f = d.path().join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, "x").unwrap();
        }
        let mut rels: Vec<String> = walk(d.path(), Side::Data, false).into_iter().map(|f| f.rel).collect();
        rels.sort();
        assert_eq!(rels, ["local_library/library.json", "store_cache/gog_library.json", "store_cache/x.json"]);
        assert_eq!(walk(d.path(), Side::Data, true).len(), 4);
    }
}
