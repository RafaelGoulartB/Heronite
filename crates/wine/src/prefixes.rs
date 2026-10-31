//! The Wine prefixes on this machine, for the Wine Manager's "Prefixes" view
//! and the prefix menu of a game's Wine settings: where they are, which games
//! use them, and creating, duplicating and moving them to the trash.
//!
//! Prefixes are found in the folders new prefixes go to (the
//! `defaultWinePrefixDir` setting, `<defaultInstallPath>/Prefixes`, the
//! Heronite and Heroic defaults) and next to every prefix a game is set to.
//! A prefix a game is set to is listed even before Wine has created it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::launchers::{managed_by, managed_paths};
use heronite_data::proc::{Cancel, Run};
use heronite_data::{game_config, sideload, DataPaths};
use heronite_i18n::tr;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixKind {
    /// Made by Proton (the Wine prefix is in `pfx`).
    Proton,
    Wine,
    /// The folder is missing or empty: Wine creates it on the next launch.
    NotCreated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    pub path: PathBuf,
    /// The folder name.
    pub name: String,
    pub kind: PrefixKind,
    /// App names of the games set to it, sorted.
    pub games: Vec<String>,
    /// Games without a prefix of their own (and new games) use it.
    pub default: bool,
}

pub fn kind_of(path: &Path) -> PrefixKind {
    if path.join("pfx/drive_c").is_dir() || path.join("pfx/system.reg").is_file() {
        PrefixKind::Proton
    } else if path.join("drive_c").is_dir() || path.join("system.reg").is_file() {
        PrefixKind::Wine
    } else {
        PrefixKind::NotCreated
    }
}

fn text(settings: &serde_json::Map<String, Value>, key: &str) -> String {
    settings.get(key).and_then(Value::as_str).map(|s| s.trim().trim_end_matches('/').to_string()).unwrap_or_default()
}

/// Where new prefixes are made: `defaultWinePrefixDir`, else Heronite's default.
pub fn new_prefix_dir(data_paths: &DataPaths, home: &str) -> PathBuf {
    let global = game_config::load_global(data_paths, home);
    let dir = text(&global, "defaultWinePrefixDir");
    PathBuf::from(if dir.is_empty() { game_config::default_wine_prefix_dir(home) } else { dir })
}

/// The default and shared folders, never moved to the trash from here.
fn protected(data_paths: &DataPaths, home: &str) -> Vec<PathBuf> {
    let global = game_config::load_global(data_paths, home);
    ["defaultInstallPath", "defaultWinePrefix", "defaultWinePrefixDir", "winePrefix"]
        .iter()
        .map(|k| text(&global, k))
        .chain([game_config::shared_wine_prefix(home), game_config::default_wine_prefix_dir(home), home.to_string()])
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Every prefix found, the ones games use first, then by name. Blocking
/// (reads the game settings and folders), no sizes.
pub fn list(data_paths: &DataPaths, home: &str) -> Vec<Prefix> {
    let global = game_config::load_global(data_paths, home);
    let default = PathBuf::from(text(&global, "winePrefix"));

    // The prefix of every game that has its own settings file.
    let mut users: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(data_paths.games_config_dir()) {
        for e in rd.flatten() {
            let path = e.path();
            let Some(app) = path.file_stem().and_then(|s| s.to_str()).filter(|_| path.extension().is_some_and(|x| x == "json")) else {
                continue;
            };
            let settings = game_config::load_game(data_paths, app, home);
            let prefix = text(&settings, "winePrefix");
            if !prefix.is_empty() {
                users.entry(PathBuf::from(prefix)).or_default().push(app.to_string());
            }
        }
    }

    let mut roots: Vec<PathBuf> = vec![
        new_prefix_dir(data_paths, home),
        PathBuf::from(game_config::default_wine_prefix_dir(home)),
        PathBuf::from(home).join("Games/Heroic/Prefixes"),
    ];
    let install = text(&global, "defaultInstallPath");
    if !install.is_empty() {
        roots.push(PathBuf::from(install).join("Prefixes"));
    }
    roots.extend(users.keys().chain([&default]).filter_map(|p| p.parent().map(Path::to_path_buf)));
    roots.sort();
    roots.dedup();

    let mut found: BTreeMap<PathBuf, PrefixKind> = BTreeMap::new();
    for root in &roots {
        let Ok(rd) = std::fs::read_dir(root) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() && !e.file_name().to_string_lossy().starts_with('.') {
                let kind = kind_of(&path);
                if kind != PrefixKind::NotCreated {
                    found.insert(path, kind);
                }
            }
        }
    }
    for p in users.keys().chain(std::iter::once(&default).filter(|d| !d.as_os_str().is_empty())) {
        found.entry(p.clone()).or_insert_with(|| kind_of(p));
    }

    let mut out: Vec<Prefix> = found
        .into_iter()
        .map(|(path, kind)| {
            let mut games = users.remove(&path).unwrap_or_default();
            games.sort();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
            Prefix { default: path == default, path, name, kind, games }
        })
        .collect();
    out.sort_by(|a, b| {
        let used = |p: &Prefix| !p.default && p.games.is_empty();
        used(a).cmp(&used(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// The disk space of a prefix in bytes. Blocking (walks the whole folder).
pub fn size_of(path: &Path) -> u64 {
    crate::install::folder_size(path)
}

/// Why `name` cannot be a new folder in `parent` (empty, a path, taken).
fn check_new_name(parent: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') || name.contains('/') || name.contains('\0') {
        bail!(tr!("Choose another name: “{name}” cannot be a folder name", name));
    }
    let path = parent.join(name);
    if path.exists() {
        bail!(tr!("There is already a folder named “{name}” in {parent}", name, parent = parent.display()));
    }
    Ok(path)
}

/// Makes an empty prefix folder named `name` where new prefixes go. Wine fills
/// it the first time a game runs in it.
pub fn create(data_paths: &DataPaths, home: &str, name: &str) -> Result<PathBuf> {
    let parent = new_prefix_dir(data_paths, home);
    let path = check_new_name(&parent, name)?;
    std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    Ok(path)
}

/// Copies `source` to a new folder `name` next to it. Blocking; uses
/// copy-on-write when the filesystem has it (btrfs, XFS), so a big prefix
/// can take no extra space and no time. A cancelled or failed copy leaves
/// nothing behind.
pub fn duplicate(source: &Path, name: &str, cancel: &Cancel) -> Result<PathBuf> {
    if kind_of(source) == PrefixKind::NotCreated {
        bail!(tr!("{path} is not a Wine prefix yet", path = source.display()));
    }
    let parent = source.parent().ok_or_else(|| anyhow!("{} has no parent folder", source.display()))?;
    let target = check_new_name(parent, name)?;
    let mut cmd = std::process::Command::new("cp");
    cmd.args(["-a", "--reflink=auto", "--"]).arg(source).arg(&target);
    let out = Run::new().cancel(cancel).run(&mut cmd);
    let ok = matches!(&out, Ok(o) if o.success());
    if !ok {
        let _ = std::fs::remove_dir_all(&target);
        return Err(match out {
            Ok(o) if o.aborted => anyhow!(tr!("Copy cancelled")),
            Ok(o) => anyhow!(tr!("Could not copy the prefix: {error}", error = String::from_utf8_lossy(&o.stderr).trim())),
            Err(e) => anyhow!(tr!("Could not copy the prefix: {error}", error = e)),
        });
    }
    Ok(target)
}

/// Moves a prefix to the trash (it can be restored from there). Refuses the
/// default and shared folders, the home folder, the prefixes of the games
/// Lutris, Bottles and Faugus start, the prefix of a store client, and anything
/// that does not look like a prefix.
pub fn trash(data_paths: &DataPaths, home: &str, path: &Path) -> Result<()> {
    if let Some(m) = managed_by(&managed_paths(data_paths, home), path) {
        bail!(tr!(
            "Won't delete {path}: {launcher} keeps {title} there. Uninstall it from {launcher}",
            path = path.display(),
            launcher = m.owner,
            title = m.title
        ));
    }
    sideload::check_prefix_removable(path, &protected(data_paths, home), Path::new(home)).map_err(|e| anyhow!(e))?;
    trash::delete(path).map_err(|e| anyhow!(tr!("Could not move {path} to the trash: {error}", path = path.display(), error = e)))?;
    // The home its programs had in the sandbox goes with it (left behind when
    // it cannot: the prefix is already in the trash).
    let sandbox_home = data_paths.sandbox_home_of_prefix(&path.to_string_lossy());
    if sandbox_home.exists() {
        if let Err(e) = trash::delete(&sandbox_home) {
            log::warn!("Could not move {} to the trash: {e}", sandbox_home.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Setup {
        _dir: tempfile::TempDir,
        paths: DataPaths,
        home: String,
        prefixes: PathBuf,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let home = dir.path().join("home");
        let prefixes = dir.path().join("pfx");
        fs::create_dir_all(paths.games_config_dir()).unwrap();
        fs::create_dir_all(&home).unwrap();
        let global = serde_json::json!({"defaultSettings": {
            "defaultWinePrefixDir": prefixes, "winePrefix": prefixes.join("shared"), "defaultInstallPath": dir.path().join("games")
        }});
        fs::write(paths.global_config(), global.to_string()).unwrap();
        Setup { home: home.to_string_lossy().into_owned(), _dir: dir, paths, prefixes }
    }

    fn game(s: &Setup, app: &str, prefix: Option<&Path>) {
        let section = match prefix {
            Some(p) => serde_json::json!({ app: { "winePrefix": p } }),
            None => serde_json::json!({ app: {} }),
        };
        fs::write(s.paths.game_config(app), section.to_string()).unwrap();
    }

    fn proton_prefix(path: &Path) {
        fs::create_dir_all(path.join("pfx/drive_c/windows")).unwrap();
        fs::write(path.join("pfx/system.reg"), "x").unwrap();
    }

    #[test]
    fn lists_used_and_found_prefixes() {
        let s = setup();
        proton_prefix(&s.prefixes.join("Control"));
        fs::create_dir_all(s.prefixes.join("Old/drive_c")).unwrap();
        fs::create_dir_all(s.prefixes.join("not-a-prefix")).unwrap();
        let elsewhere = s._dir.path().join("elsewhere/Hollow Knight");
        proton_prefix(&elsewhere);
        proton_prefix(&s._dir.path().join("elsewhere/Sibling"));
        game(&s, "control", Some(&s.prefixes.join("Control")));
        game(&s, "hk", Some(&elsewhere));
        game(&s, "a", None);
        game(&s, "b", None);

        let list = list(&s.paths, &s.home);
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        // Used ones first, then the rest by name; a folder without Wine files is not a prefix.
        assert_eq!(names, ["Control", "Hollow Knight", "shared", "Old", "Sibling"]);
        let shared = &list[2];
        assert!(shared.default && shared.kind == PrefixKind::NotCreated);
        assert_eq!(shared.games, ["a", "b"], "games without their own prefix use the default");
        assert_eq!(list[0].games, ["control"]);
        assert_eq!(list[0].kind, PrefixKind::Proton);
        assert_eq!(list[3].kind, PrefixKind::Wine);
    }

    #[test]
    fn create_and_duplicate_check_the_name_and_the_source() {
        let s = setup();
        let made = create(&s.paths, &s.home, " Fresh ").unwrap();
        assert_eq!(made, s.prefixes.join("Fresh"));
        assert!(made.is_dir());
        for bad in ["", "..", ".hidden", "a/b", "Fresh"] {
            assert!(create(&s.paths, &s.home, bad).is_err(), "{bad:?}");
        }
        assert!(duplicate(&made, "Copy", &Cancel::default()).is_err(), "an empty folder is not a prefix yet");
    }

    // The copy runs `cp`: Wine prefixes are Linux's.
    #[cfg(unix)]
    #[test]
    fn a_prefix_is_copied_once_under_a_new_name() {
        let s = setup();
        let source = s.prefixes.join("Control");
        proton_prefix(&source);
        fs::write(source.join("pfx/drive_c/save.dat"), "progress").unwrap();
        let copy = duplicate(&source, "Control copy", &Cancel::default()).unwrap();
        assert_eq!(fs::read_to_string(copy.join("pfx/drive_c/save.dat")).unwrap(), "progress");
        assert!(duplicate(&source, "Control copy", &Cancel::default()).is_err(), "taken");
    }

    #[test]
    fn default_and_shared_folders_are_never_trashed() {
        let s = setup();
        let shared = s.prefixes.join("shared");
        proton_prefix(&shared);
        let err = trash(&s.paths, &s.home, &shared).unwrap_err().to_string();
        assert!(err.contains("shared or default"), "{err}");
        assert!(shared.exists());
        assert!(trash(&s.paths, &s.home, &s.prefixes).is_err());
        let not_prefix = s.prefixes.join("music");
        fs::create_dir_all(&not_prefix).unwrap();
        fs::write(not_prefix.join("song.flac"), "x").unwrap();
        assert!(trash(&s.paths, &s.home, &not_prefix).is_err());
        assert!(not_prefix.exists());
    }

    #[test]
    fn the_prefix_of_a_lutris_game_is_never_trashed() {
        let s = setup();
        let lutris = s._dir.path().join("home/Games/heroes-iii");
        fs::create_dir_all(lutris.join("drive_c")).unwrap();
        let meta = serde_json::json!({"games": {"lutris_7": {"appName": "lutris_7", "runner": "sideload", "source": "lutris",
            "launchKind": "launcher", "title": "Heroes III", "heroniteLauncherPrefix": lutris}}});
        heronite_data::json_store::write(&s.paths.local_meta(), &meta).unwrap();
        let err = trash(&s.paths, &s.home, &lutris).unwrap_err().to_string();
        assert!(err.contains("Lutris keeps Heroes III there"), "{err}");
        assert!(lutris.join("drive_c").exists());
    }

    #[test]
    fn the_prefix_of_a_store_client_is_listed_and_never_trashed() {
        use heronite_data::clients::Client;
        let s = setup();
        let battlenet = s.prefixes.join("Battle.net");
        proton_prefix(&battlenet);
        game(&s, Client::BattleNet.config_app(), Some(&battlenet));
        let listed = list(&s.paths, &s.home);
        let found = listed.iter().find(|p| p.path == battlenet).unwrap();
        assert_eq!(found.games, [Client::BattleNet.config_app()]);

        let err = trash(&s.paths, &s.home, &battlenet).unwrap_err().to_string();
        assert!(err.contains("Battle.net keeps Battle.net there. Uninstall it from Battle.net"), "{err}");
        assert!(battlenet.join("pfx/drive_c").exists());
        // Nor the folder that holds it.
        assert!(trash(&s.paths, &s.home, &s.prefixes).is_err());
    }
}
