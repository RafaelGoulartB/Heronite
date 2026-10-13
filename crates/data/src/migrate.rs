//! One-time copy of an existing Heroic setup into this app's data folder.
//!
//! On the first start (the data folder does not exist yet) the parts of
//! Heroic's config folder this app uses are copied in: library, Collection,
//! playtime, game settings, Epic login (`legendaryConfig`), Wine tools and
//! caches, plus the game logs and the legendary binary of the Heronite
//! install. Absolute paths into Heroic's folder (Wine builds in `tools/`) are
//! rewritten to the new one. Heroic's files are only read, so Heroic keeps
//! working on its own and the two apps never touch each other's data again.
//!
//! The copy goes to a sibling staging folder that is renamed into place at
//! the end: an interrupted copy leaves no half data folder behind.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use crate::paths::{AppPaths, DataPaths};

/// Written into the data folder by a migration.
pub const MARKER: &str = "migrated-from-heroic.json";

/// What is taken from Heroic's config folder. The rest there is Chromium's
/// (caches, cookies, local storage) or belongs to Heroic only.
const ENTRIES: &[&str] = &[
    "config.json",
    "store",
    "store_cache",
    "GamesConfig",
    "sideload_apps",
    "gog_store",
    "nile_store",
    "zoom_store",
    "legendaryConfig",
    "gogdlConfig",
    "nile_config",
    "local_library",
    "tools",
    "images-cache",
    "icons",
    "known_fixes.json",
    "areweanticheatyet.json",
];

/// Folders of large binary files never scanned for paths to rewrite.
const NO_REWRITE: &[&str] = &["tools", "images-cache", "icons"];

/// Where the old data comes from.
#[derive(Debug, Clone)]
pub struct Source {
    /// Heroic's config folder (`~/.config/heroic`).
    pub heroic: PathBuf,
    /// Heroic's logs folder (`~/.local/state/Heroic/logs`): `games/` and
    /// `runners/` are copied.
    pub logs: Option<PathBuf>,
    /// This app's launcher logs from before the data folder existed.
    pub launcher_logs: Option<PathBuf>,
    /// legendary binaries to copy into `tools/bin`, first existing wins.
    pub legendary: Vec<PathBuf>,
}

impl Source {
    /// The real locations on this machine.
    pub fn detect(app: &AppPaths) -> Option<Self> {
        let heroic = dirs::config_dir()?.join("heroic");
        let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "x64" };
        let rel = format!("resources/app.asar.unpacked/build/bin/{arch}/linux/legendary");
        let mut legendary = vec![heroic.join("tools/bin/legendary")];
        if let Some(home) = dirs::home_dir() {
            legendary.push(home.join(".local/opt/Heronite").join(&rel));
        }
        for base in ["/opt/Heronite", "/opt/Heroic", "/usr/lib/heroic"] {
            legendary.push(Path::new(base).join(&rel));
        }
        Some(Self {
            heroic,
            logs: dirs::state_dir().map(|s| s.join("Heroic/logs")),
            launcher_logs: Some(app.config.join("logs")),
            legendary,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The data folder already exists (or is not the real one).
    NotNeeded,
    /// No Heroic setup to copy: the app starts empty.
    Fresh,
    Migrated(Report),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub files: usize,
    pub bytes: u64,
    /// Text files whose paths into Heroic's folder were rewritten.
    pub rewritten: usize,
    pub legendary: Option<PathBuf>,
    pub ui_state: bool,
}

/// Earlier name of this app's folders (`~/.config/heronite-rs`...).
const OLD_APP_DIR: &str = "heronite-rs";

/// Renames the UI state and cache folders of the old name to the current
/// one when only the old ones exist.
pub fn adopt_old_app_dirs(app: &AppPaths) {
    for dir in [&app.config, &app.cache] {
        let old = dir.with_file_name(OLD_APP_DIR);
        if old.is_dir() && fs::symlink_metadata(dir).is_err() {
            let _ = fs::rename(&old, dir);
        }
    }
}

/// Runs the migration when `data` is the real data folder and does not
/// exist yet. On error nothing is left in place, so the next start retries.
pub fn run_if_needed(data: &DataPaths, app: &AppPaths) -> Result<Outcome> {
    if !data.is_default() {
        return Ok(Outcome::NotNeeded);
    }
    // First, so the old launcher logs are found under the new name.
    adopt_old_app_dirs(app);
    if data.root.exists() {
        return Ok(Outcome::NotNeeded);
    }
    match Source::detect(app).filter(|s| s.heroic.join("config.json").exists() || s.heroic.join("store").is_dir()) {
        Some(source) => migrate(&source, data, app).map(Outcome::Migrated),
        None => Ok(Outcome::Fresh),
    }
}

/// Copies `source` into `data.root`, which must not exist yet.
pub fn migrate(source: &Source, data: &DataPaths, app: &AppPaths) -> Result<Report> {
    let target = &data.root;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "data".into());
    let staging = target.with_file_name(format!(".{name}.migrating-{}", std::process::id()));
    remove_stale_staging(target, &name);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;

    let result = fill(source, &staging, target);
    let mut report = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    if let Err(e) = fs::rename(&staging, target) {
        let _ = fs::remove_dir_all(&staging);
        // Another instance finished first: its copy is as good as ours.
        if target.exists() {
            return Ok(report);
        }
        return Err(e).with_context(|| format!("moving the data into {}", target.display()));
    }
    if !app.ui_state().exists() {
        if let Some(ui) = crate::heronite_ui_import::import(&source.heroic) {
            report.ui_state = ui.save(app).is_ok();
        }
    }
    Ok(report)
}

fn fill(source: &Source, staging: &Path, target: &Path) -> Result<Report> {
    let mut report = Report::default();
    for entry in ENTRIES {
        let from = source.heroic.join(entry);
        if fs::symlink_metadata(&from).is_ok() {
            copy_any(&from, &staging.join(entry), &mut report).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    if let Some(logs) = &source.logs {
        for sub in ["games", "runners"] {
            let from = logs.join(sub);
            if from.is_dir() {
                copy_any(&from, &staging.join("logs").join(sub), &mut report)?;
            }
        }
    }
    if let Some(dir) = source.launcher_logs.as_deref().filter(|d| d.is_dir()) {
        copy_any(dir, &staging.join("logs/launcher"), &mut report)?;
    }
    let bin = DataPaths::at(staging).legendary_bin();
    if !bin.exists() {
        if let Some(found) = source.legendary.iter().find(|p| p.is_file()) {
            fs::create_dir_all(bin.parent().unwrap())?;
            fs::copy(found, &bin).with_context(|| format!("copying {}", found.display()))?;
            set_executable(&bin)?;
            report.legendary = Some(found.clone());
        }
    }
    // Files hold the folder as Heroic saw it: the given path or, when it goes
    // through a symlink, the real one.
    let mut olds = vec![source.heroic.clone()];
    olds.extend(fs::canonicalize(&source.heroic).ok().filter(|c| *c != source.heroic));
    report.rewritten = rewrite_paths(staging, &olds, target)?;
    let marker = json!({
        "from": source.heroic,
        "at": chrono::Local::now().to_rfc3339(),
        "files": report.files,
        "bytes": report.bytes,
    });
    fs::write(staging.join(MARKER), serde_json::to_string_pretty(&marker)?)?;
    Ok(report)
}

/// Copies a file, a symlink (as a link) or a folder tree.
fn copy_any(from: &Path, to: &Path, report: &mut Report) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
    } else if ft.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_any(&entry.path(), &to.join(entry.file_name()), report)?;
        }
    } else if ft.is_file() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        // `fs::copy` uses copy_file_range: a reflink on btrfs/XFS, so even
        // gigabytes of Proton builds copy in moments there.
        report.bytes += fs::copy(from, to)?;
        report.files += 1;
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_: &Path) -> io::Result<()> {
    Ok(())
}

/// Replaces each of `olds` by `new` in the JSON and INI files under `root`
/// where it is a whole path prefix (followed by `/`, a quote or the end of a
/// value).
fn rewrite_paths(root: &Path, olds: &[PathBuf], new: &Path) -> Result<usize> {
    let olds: Vec<String> = olds.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let new = new.to_string_lossy().into_owned();
    let mut count = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                let skip = dir == root && NO_REWRITE.contains(&entry.file_name().to_string_lossy().as_ref());
                if !skip {
                    stack.push(path);
                }
                continue;
            }
            let is_text = path.extension().is_some_and(|e| e == "json" || e == "ini");
            if !ft.is_file() || !is_text {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let mut current = text.clone();
            for old in &olds {
                if let Some(changed) = replace_path_prefix(&current, old, &new) {
                    current = changed;
                }
            }
            if current != text {
                fs::write(&path, current)?;
                count += 1;
            }
        }
    }
    Ok(count)
}

/// `text` with every `old` that is a whole path (or a path prefix) replaced;
/// `None` when nothing matched.
pub fn replace_path_prefix(text: &str, old: &str, new: &str) -> Option<String> {
    if old.is_empty() || !text.contains(old) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(i) = rest.find(old) {
        let after = &rest[i + old.len()..];
        let whole = after.is_empty() || after.starts_with(['/', '"', '\'', '\\', '\n', '\r']);
        out.push_str(&rest[..i]);
        out.push_str(if whole { new } else { old });
        changed |= whole;
        rest = after;
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// Staging folders of migrations whose process is gone.
fn remove_stale_staging(target: &Path, name: &str) {
    let Some(parent) = target.parent() else { return };
    let prefix = format!(".{name}.migrating-");
    for entry in fs::read_dir(parent).into_iter().flatten().flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = file_name.strip_prefix(&prefix) else { continue };
        if !Path::new("/proc").join(pid).exists() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn path_prefixes_are_replaced_only_when_whole() {
        let old = "/h/.config/heroic";
        let new = "/h/.local/share/heronite";
        let text = r#"{"bin":"/h/.config/heroic/tools/proton/GE/proton","root":"/h/.config/heroic","other":"/h/.config/heroic2/x"}"#;
        assert_eq!(
            replace_path_prefix(text, old, new).unwrap(),
            r#"{"bin":"/h/.local/share/heronite/tools/proton/GE/proton","root":"/h/.local/share/heronite","other":"/h/.config/heroic2/x"}"#
        );
        assert_eq!(replace_path_prefix(r#"{"a":"/h/.config/heroic2"}"#, old, new), None);
        assert_eq!(replace_path_prefix("{}", old, new), None);
    }

    #[test]
    fn copies_what_the_app_uses_and_rewrites_paths() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = dir.path().join("config/heroic");
        let bin = r#""/x/heroic/tools/proton/GE/proton""#.replace("/x/heroic", &heroic.to_string_lossy());
        write(&heroic.join("config.json"), &format!(r#"{{"defaultSettings":{{"wineVersion":{{"bin":{bin}}}}}}}"#));
        write(&heroic.join("store/config.json"), "{}");
        write(&heroic.join("GamesConfig/G.json"), &format!(r#"{{"G":{{"wineVersion":{{"bin":{bin}}}}}}}"#));
        write(&heroic.join("local_library/library.json"), r#"{"games":{}}"#);
        write(&heroic.join("legendaryConfig/legendary/user.json"), "{}");
        write(&heroic.join("tools/proton/GE/proton"), &heroic.to_string_lossy());
        write(&heroic.join("Cache/data"), "chromium");
        write(&heroic.join("Local Storage/leveldb/LOG"), "chromium");
        let logs = dir.path().join("state/Heroic/logs");
        write(&logs.join("games/G_sideload/launch.log"), "played");
        write(&logs.join("heroic.log"), "heroic's own");
        let launcher_logs = dir.path().join("config/heronite-rs/logs");
        write(&launcher_logs.join("heronite-rs-2026-10-01.log"), "ours");
        let legendary = dir.path().join("opt/legendary");
        write(&legendary, "#!/bin/sh");

        let data = DataPaths::at(dir.path().join("share/heronite-rs"));
        let app = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        let source = Source {
            heroic: heroic.clone(),
            logs: Some(logs),
            launcher_logs: Some(launcher_logs),
            legendary: vec![dir.path().join("missing"), legendary.clone()],
        };
        let report = migrate(&source, &data, &app).unwrap();

        let root = &data.root;
        assert!(root.join(MARKER).exists());
        assert!(root.join("store/config.json").exists());
        assert!(root.join("local_library/library.json").exists());
        assert!(root.join("legendaryConfig/legendary/user.json").exists());
        assert!(!root.join("Cache").exists() && !root.join("Local Storage").exists());
        assert!(root.join("logs/games/G_sideload/launch.log").exists());
        assert!(!root.join("logs/heroic.log").exists());
        assert!(root.join("logs/launcher/heronite-rs-2026-10-01.log").exists());
        assert!(data.legendary_bin().exists());
        assert_eq!(report.legendary.as_deref(), Some(legendary.as_path()));
        // Paths into Heroic's folder now point into the data folder; binaries are untouched.
        let want = root.join("tools/proton/GE/proton").to_string_lossy().into_owned();
        assert!(fs::read_to_string(data.global_config()).unwrap().contains(&want));
        assert!(fs::read_to_string(data.game_config("G")).unwrap().contains(&want));
        assert_eq!(fs::read_to_string(root.join("tools/proton/GE/proton")).unwrap(), heroic.to_string_lossy());
        assert_eq!(report.rewritten, 2);
        // Heroic's folder is left as it was; no staging folder remains.
        assert!(fs::read_to_string(heroic.join("config.json")).unwrap().contains(&*heroic.to_string_lossy()));
        let leftovers: Vec<_> =
            fs::read_dir(root.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name() != "heronite-rs").collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn paths_of_a_symlinked_heroic_folder_are_rewritten_too() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real/heroic");
        write(&real.join("config.json"), &format!(r#"{{"bin":"{}/tools/w"}}"#, real.display()));
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let data = DataPaths::at(dir.path().join("share/heronite-rs"));
        let app = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        let source = Source { heroic: link, logs: None, launcher_logs: None, legendary: vec![] };
        assert_eq!(migrate(&source, &data, &app).unwrap().rewritten, 1);
        let want = format!(r#"{{"bin":"{}/tools/w"}}"#, data.root.display());
        assert_eq!(fs::read_to_string(data.global_config()).unwrap(), want);
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = dir.path().join("heroic");
        write(&heroic.join("store/config.json"), "{}");
        let data = DataPaths::at(dir.path().join("share/heronite-rs"));
        // The target's parent is a file: the staging folder cannot be made.
        write(&dir.path().join("share"), "not a folder");
        let app = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        let source = Source { heroic, logs: None, launcher_logs: None, legendary: vec![] };
        assert!(migrate(&source, &data, &app).is_err());
        assert!(!data.root.exists());
    }

    #[test]
    fn old_app_folders_are_renamed_once() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config/heronite"), cache: dir.path().join("cache/heronite") };
        write(&dir.path().join("config/heronite-rs/ui-state.json"), "{}");
        write(&dir.path().join("cache/heronite/thumbs/x"), "new");
        write(&dir.path().join("cache/heronite-rs/thumbs/x"), "old");
        adopt_old_app_dirs(&app);
        assert!(app.ui_state().exists() && !dir.path().join("config/heronite-rs").exists());
        // An existing new folder is never replaced.
        assert_eq!(fs::read_to_string(app.thumbs_dir().join("x")).unwrap(), "new");
        assert!(dir.path().join("cache/heronite-rs").exists());
    }

    #[test]
    fn only_the_real_missing_folder_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataPaths::at(dir.path().join("fixture"));
        let app = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        assert_eq!(run_if_needed(&data, &app).unwrap(), Outcome::NotNeeded);
        assert!(!data.root.exists());
    }

    /// Against the real Heroic folder, into a temp folder:
    /// `cargo test -p heronite-data real_heroic_folder -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_heroic_folder_migrates_with_the_same_library() {
        let app = AppPaths::detect();
        let Some(source) = Source::detect(&app).filter(|s| s.heroic.exists()) else { return };
        let dir = tempfile::tempdir().unwrap();
        let data = DataPaths::at(dir.path().join("heronite-rs"));
        let t = std::time::Instant::now();
        let app_tmp = AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") };
        let report = migrate(&source, &data, &app_tmp).unwrap();
        eprintln!("{report:?} in {:?}", t.elapsed());
        let before = crate::Library::load(&DataPaths::at(&source.heroic)).unwrap();
        let after = crate::Library::load(&data).unwrap();
        assert_eq!(before.games.len(), after.games.len());
        let old = source.heroic.to_string_lossy().into_owned();
        for f in [data.global_config(), data.store_config()] {
            assert!(!fs::read_to_string(f).unwrap_or_default().contains(&old));
        }
    }
}
