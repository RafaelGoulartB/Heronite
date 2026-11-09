//! What a launch calls: applies the enabled mods to the game's folder (a
//! `fuse-overlayfs` mount on Linux, the files put in place on Windows),
//! lists their plugins and tells which environment the game needs. Dropping
//! the result takes all of it away again.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use heronite_i18n::{tr, trn};

use crate::bethesda_ini::{self, IniGuard};
use crate::games::{KnownGame, Profile};
use crate::model::Progress;
use crate::paths::{GameDirs, ModsRoot};
use crate::plugins::{self, PluginsGuard, UserHome};
use crate::store;

pub struct ApplyRequest<'a> {
    pub root: &'a ModsRoot,
    /// `<data>/tools/bin`: where `fuse-overlayfs` is kept or downloaded to.
    pub tools_bin: &'a Path,
    pub game_key: &'a str,
    pub game_dir: &'a Path,
    /// Where the game keeps its user's files (`Plugins.txt`, `*Custom.ini`):
    /// its Wine prefix, or on Windows the user's own folders. `None`: a
    /// native Linux game.
    pub home: Option<UserHome<'a>>,
    pub known: Option<&'static KnownGame>,
}

/// The mods applied to the game folder.
#[derive(Debug)]
enum Deployment {
    #[cfg(not(windows))]
    Mount(crate::deploy::Mount),
    #[cfg_attr(not(windows), allow(dead_code))]
    Files(crate::journal_deploy::Deployed),
}

impl Deployment {
    fn game_dir(&self) -> &Path {
        match self {
            #[cfg(not(windows))]
            Deployment::Mount(m) => m.game_dir(),
            Deployment::Files(d) => d.game_dir(),
        }
    }
}

/// Mods in effect. Dropping it restores `Plugins.txt`, then unmounts.
#[derive(Debug)]
pub struct Applied {
    /// Variables to add to the game's environment (`WINEDLLOVERRIDES`; none
    /// on Windows); merge the value with the launch's own through
    /// [`merge_dll_overrides`].
    pub env: Vec<(String, String)>,
    /// One English line for the game's log.
    pub summary: String,
    // Fields drop in this order: the user's files go back before the mods
    // leave the game folder.
    _plugins: Option<PluginsGuard>,
    _ini: Option<IniGuard>,
    deployment: Option<Deployment>,
}

impl Applied {
    pub fn game_dir(&self) -> Option<&Path> {
        self.deployment.as_ref().map(Deployment::game_dir)
    }
}

/// `Ok(None)`: the game has no mods to apply (no `mods.json`, "Play with
/// mods" off, or nothing enabled). An `Err` is a sentence for the user and
/// the game should not start.
pub fn apply(req: &ApplyRequest, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool) -> Result<Option<Applied>, String> {
    let dirs = req.root.game(req.game_key);
    if !dirs.list_file().is_file() {
        return Ok(None);
    }
    let list = store::load(&dirs);
    if !list.applies() {
        return Ok(None);
    }
    let failed = |e: String| tr!("Mods could not be applied: {error}", error = e);

    let enabled: Vec<_> = list.enabled().collect();
    let mut layers: Vec<PathBuf> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for m in &enabled {
        let dir = dirs.mod_dir(&m.id);
        if dir.is_dir() {
            layers.push(dir);
        } else {
            log::warn!("mods: the folder of {} is missing ({})", m.id, dir.display());
            missing.push(if m.name.trim().is_empty() { &m.id } else { &m.name });
        }
    }
    // A game whose saves expect its mods must not start without some of them.
    if !missing.is_empty() {
        let names = missing.join(", ");
        return Err(failed(trn!(
            missing.len(),
            "{n} enabled mod is missing its files: {names}. Reinstall or remove it in Mods.",
            "{n} enabled mods are missing their files: {names}. Reinstall or remove them in Mods.",
            names
        )));
    }
    if layers.is_empty() {
        return Ok(None);
    }

    #[cfg(not(windows))]
    let overlay = {
        let support = crate::tool::support(req.tools_bin);
        if support.fusermount.is_none() || !support.dev_fuse {
            return Err(failed(support.problem().unwrap_or_default()));
        }
        crate::tool::ensure(req.tools_bin, progress, cancel).map_err(failed)?
    };
    // Nothing to download on Windows.
    #[cfg(windows)]
    let _ = (progress, cancel);
    // Before mounting: the game's own folders, not the mods'.
    let exe_dirs = crate::install::exe_dirs(req.game_dir);

    // Before mounting too: old games are ordered by the time of the game's own
    // plugins, and once the mods are over the folder theirs would count.
    let mut summary = String::new();
    let spec = Profile::of(&list.links, req.known).plugins;
    let mut guard = None;
    if let (Some(spec), Some(home)) = (spec, req.home) {
        match plugins::write(home, &spec, &dirs, &list, req.game_dir) {
            Ok(Some(g)) => {
                summary.push_str(&format!("; Plugins.txt: {} plugins", g.count()));
                guard = Some(g);
            }
            Ok(None) => summary.push_str("; Plugins.txt: no user folder yet"),
            Err(e) => {
                log::warn!("mods: Plugins.txt: {e}");
                summary.push_str(&format!("; Plugins.txt failed: {e}"));
            }
        }
    }
    let mut ini = None;
    if let Some(home) = req.home {
        match bethesda_ini::ensure_loose_files(home, req.known) {
            Ok(Some(g)) => {
                summary.push_str(&format!("; {}: loose files on", g.file().file_name().unwrap_or_default().to_string_lossy()));
                ini = Some(g);
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("mods: loose files setting: {e}");
                summary.push_str(&format!("; loose files setting failed: {e}"));
            }
        }
    }
    #[cfg(not(windows))]
    let (deployment, how) = {
        let mount = crate::deploy::mount(&overlay, &dirs, req.game_dir, &layers).map_err(failed)?;
        let how = format!("mounted over {} with fuse-overlayfs", mount.game_dir().display());
        (Deployment::Mount(mount), how)
    };
    #[cfg(windows)]
    let (deployment, how) = deploy_files(&dirs, req.game_dir, &layers).map_err(failed)?;
    let mut summary = format!("Mods: {} of {} enabled, {how}{summary}", layers.len(), list.mods.len());

    // Looked at again now, not only at install: mods installed before a DLL
    // was known as a proxy get their override too.
    let mut dlls: Vec<String> = enabled.iter().flat_map(|m| m.dll_overrides.iter().map(|d| d.to_lowercase())).collect();
    dlls.extend(layers.iter().flat_map(|l| crate::install::proxy_dlls(l, &exe_dirs)));
    dlls.sort();
    dlls.dedup();
    let env = dll_env(&dlls);
    if !dlls.is_empty() {
        let label = if env.is_empty() { "proxy DLLs (loaded from the game folder)" } else { "DLL overrides" };
        summary.push_str(&format!("; {label}: {}", dlls.join(", ")));
    }
    Ok(Some(Applied { env, summary, _plugins: guard, _ini: ini, deployment: Some(deployment) }))
}

/// Windows: the files go into the game folder ([`crate::journal_deploy`]).
#[cfg_attr(not(windows), allow(dead_code))]
fn deploy_files(dirs: &GameDirs, game_dir: &Path, layers: &[PathBuf]) -> Result<(Deployment, String), String> {
    let deployed = crate::journal_deploy::deploy(dirs, game_dir, layers)?;
    let s = deployed.summary();
    let how = format!(
        "{} files put into {} ({} linked, {} copied, {} game files set aside until the game ends)",
        s.files,
        deployed.game_dir().display(),
        s.linked,
        s.copied,
        s.replaced
    );
    Ok((Deployment::Files(deployed), how))
}

/// Wine and Proton prefer their own build of a DLL unless told otherwise;
/// Windows loads the one in the game's folder first by itself (none of the
/// proxy names is a KnownDLL), so it needs no variable.
fn dll_env(dlls: &[String]) -> Vec<(String, String)> {
    if dlls.is_empty() || cfg!(windows) {
        Vec::new()
    } else {
        vec![("WINEDLLOVERRIDES".to_string(), merge_dll_overrides(None, dlls))]
    }
}

/// `existing` (a `WINEDLLOVERRIDES` value, `a,b=n,b;c=d`) with `add` set to
/// `n,b` (native, then built-in): the other entries stay, the DLLs in `add`
/// are taken out of the entries that named them and listed last.
pub fn merge_dll_overrides(existing: Option<&str>, add: &[String]) -> String {
    let ours: Vec<String> = add.iter().map(|d| d.to_lowercase()).collect();
    let mut parts: Vec<String> = Vec::new();
    for entry in existing.unwrap_or_default().split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let Some((modules, mode)) = entry.split_once('=') else {
            parts.push(entry.to_string());
            continue;
        };
        let kept: Vec<&str> = modules.split(',').map(str::trim).filter(|m| !m.is_empty() && !ours.contains(&m.to_lowercase())).collect();
        if !kept.is_empty() {
            parts.push(format!("{}={mode}", kept.join(",")));
        }
    }
    let mut seen = Vec::new();
    for dll in &ours {
        if !seen.contains(dll) {
            parts.push(format!("{dll}=n,b"));
            seen.push(dll.clone());
        }
    }
    parts.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::find;
    use crate::model::{ModEntry, ModList};
    use crate::tool;
    use std::fs;

    #[test]
    fn merges_dll_overrides() {
        let add = ["winhttp".to_string(), "version".to_string()];
        assert_eq!(merge_dll_overrides(None, &add), "winhttp=n,b;version=n,b");
        assert_eq!(merge_dll_overrides(Some(""), &["dinput8".into()]), "dinput8=n,b");
        assert_eq!(merge_dll_overrides(Some("mscoree=d;winhttp=b"), &add), "mscoree=d;winhttp=n,b;version=n,b");
        assert_eq!(merge_dll_overrides(Some("winhttp,Version,mshtml=n"), &add), "mshtml=n;winhttp=n,b;version=n,b");
        assert_eq!(merge_dll_overrides(Some(" a=n ; ;junk;b=d "), &["a".into(), "A".into()]), "junk;b=d;a=n,b");
    }

    /// Needs the tool and `/dev/fuse` (`HERONITE_FUSE_OVERLAYFS=...`).
    fn can_mount() -> bool {
        Path::new("/dev/fuse").exists() && tool::fusermount().is_some() && tool::find(Path::new("/nonexistent")).is_some()
    }

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn game(t: &Path) -> PathBuf {
        let game = t.join("game");
        put(game.join("Data/Skyrim.esm"), "game");
        put(game.join("Data/base.txt"), "game");
        game
    }

    #[test]
    fn nothing_to_apply_is_not_an_error() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let game = game(t.path());
        let req = ApplyRequest { root: &root, tools_bin: t.path(), game_key: "g", game_dir: &game, home: None, known: None };
        let cancel = AtomicBool::new(false);
        assert!(apply(&req, &mut |_| {}, &cancel).unwrap().is_none(), "no mods.json");

        let dirs = root.game("g");
        let mut list = ModList::new("g", "G");
        list.mods.push(ModEntry { id: "a".into(), enabled: true, ..Default::default() });
        list.active = false;
        store::save(&dirs, &list).unwrap();
        assert!(apply(&req, &mut |_| {}, &cancel).unwrap().is_none(), "play with mods is off");
        list.active = true;
        list.mods[0].enabled = false;
        store::save(&dirs, &list).unwrap();
        assert!(apply(&req, &mut |_| {}, &cancel).unwrap().is_none(), "nothing enabled");
        list.mods[0].enabled = true;
        store::save(&dirs, &list).unwrap();
        let missing = apply(&req, &mut |_| {}, &cancel).unwrap_err();
        assert!(missing.contains("1 enabled mod is missing its files: a."), "{missing}");
    }

    #[test]
    fn mounts_lists_plugins_and_cleans_up() {
        if !can_mount() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = game(t.path());
        put(dirs.mod_dir("a").join("Data/Mod.esp"), "mod");
        put(dirs.mod_dir("a").join("Data/base.txt"), "mod");
        put(dirs.mod_dir("a").join("winhttp.dll"), "dll");
        let mut list = ModList::new("g", "Skyrim");
        list.mods.push(ModEntry {
            id: "a".into(),
            enabled: true,
            plugins: vec!["Mod.esp".into()],
            dll_overrides: vec!["winhttp".into()],
            ..Default::default()
        });
        store::save(&dirs, &list).unwrap();
        let prefix = t.path().join("prefix");
        let appdata = prefix.join("drive_c/users/steamuser/AppData/Local/Skyrim Special Edition");
        fs::create_dir_all(&appdata).unwrap();
        fs::write(appdata.join("Plugins.txt"), "*Mine.esp\r\n").unwrap();

        let known = find(Some("489830"), "");
        let req =
            ApplyRequest { root: &root, tools_bin: t.path(), game_key: "g", game_dir: &game, home: Some(UserHome::Prefix(&prefix)), known };
        let applied = apply(&req, &mut |_| {}, &AtomicBool::new(false)).unwrap().unwrap();
        assert_eq!(applied.env, [("WINEDLLOVERRIDES".to_string(), "winhttp=n,b".to_string())]);
        assert!(applied.summary.starts_with("Mods: 1 of 1 enabled, mounted over "), "{}", applied.summary);
        assert!(applied.summary.contains("Plugins.txt: 1 plugins"), "{}", applied.summary);
        assert_eq!(fs::read_to_string(game.join("Data/base.txt")).unwrap(), "mod");
        assert!(game.join("Data/Mod.esp").exists() && game.join("winhttp.dll").exists());
        assert!(fs::read_to_string(appdata.join("Plugins.txt")).unwrap().contains("*Mod.esp"));
        drop(applied);
        assert!(!game.join("Data/Mod.esp").exists());
        assert_eq!(fs::read_to_string(game.join("Data/base.txt")).unwrap(), "game");
        assert_eq!(fs::read_to_string(appdata.join("Plugins.txt")).unwrap(), "*Mine.esp\r\n");
    }

    #[test]
    fn plugin_times_do_not_drift_from_launch_to_launch() {
        if !can_mount() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = t.path().join("game");
        put(game.join("Data/Oblivion.esm"), "game");
        put(dirs.mod_dir("a").join("Data/Mod.esp"), "mod");
        let mut list = ModList::new("g", "Oblivion");
        list.mods.push(ModEntry { id: "a".into(), enabled: true, plugins: vec!["Mod.esp".into()], ..Default::default() });
        store::save(&dirs, &list).unwrap();
        let prefix = t.path().join("prefix");
        fs::create_dir_all(prefix.join("drive_c/users/steamuser")).unwrap();
        let req = ApplyRequest {
            root: &root,
            tools_bin: t.path(),
            game_key: "g",
            game_dir: &game,
            home: Some(UserHome::Prefix(&prefix)),
            known: find(Some("22330"), ""),
        };
        let time = || dirs.mod_dir("a").join("Data/Mod.esp").metadata().unwrap().modified().unwrap();
        drop(apply(&req, &mut |_| {}, &AtomicBool::new(false)).unwrap().unwrap());
        let first = time();
        drop(apply(&req, &mut |_| {}, &AtomicBool::new(false)).unwrap().unwrap());
        assert_eq!(time(), first, "the mod's plugin is not taken for the game's");
    }

    #[test]
    fn fallout_4_reads_loose_files_while_mounted_only() {
        if !can_mount() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = t.path().join("game");
        put(game.join("Data/Fallout4.esm"), "game");
        put(dirs.mod_dir("a").join("Data/Textures/a.dds"), "mod");
        let mut list = ModList::new("g", "Fallout 4");
        list.mods.push(ModEntry { id: "a".into(), enabled: true, ..Default::default() });
        store::save(&dirs, &list).unwrap();
        let prefix = t.path().join("prefix");
        let ini = prefix.join("drive_c/users/steamuser/Documents/My Games/Fallout4/Fallout4Custom.ini");
        put(ini.clone(), "[Display]\r\niSize W=1\r\n");
        let req = ApplyRequest {
            root: &root,
            tools_bin: t.path(),
            game_key: "g",
            game_dir: &game,
            home: Some(UserHome::Prefix(&prefix)),
            known: find(Some("377160"), ""),
        };
        let applied = apply(&req, &mut |_| {}, &AtomicBool::new(false)).unwrap().unwrap();
        assert!(fs::read_to_string(&ini).unwrap().contains("bInvalidateOlderFiles=1"));
        drop(applied);
        assert_eq!(fs::read_to_string(&ini).unwrap(), "[Display]\r\niSize W=1\r\n");
    }

    /// Windows: the files go into the game folder, the plugins into
    /// `%LOCALAPPDATA%`, and no `WINEDLLOVERRIDES` is needed.
    #[cfg(windows)]
    #[test]
    fn puts_files_in_place_lists_plugins_and_cleans_up() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = game(t.path());
        put(dirs.mod_dir("a").join("Data/Mod.esp"), "mod");
        put(dirs.mod_dir("a").join("Data/base.txt"), "mod");
        put(dirs.mod_dir("a").join("winhttp.dll"), "dll");
        let mut list = ModList::new("g", "Skyrim");
        list.mods.push(ModEntry { id: "a".into(), enabled: true, plugins: vec!["Mod.esp".into()], ..Default::default() });
        store::save(&dirs, &list).unwrap();
        let (local, documents) = (t.path().join("Local"), t.path().join("Documents"));
        fs::create_dir_all(&local).unwrap();
        let req = ApplyRequest {
            root: &root,
            tools_bin: t.path(),
            game_key: "g",
            game_dir: &game,
            home: Some(UserHome::Native { local_app_data: &local, documents: &documents }),
            known: find(Some("489830"), ""),
        };
        let applied = apply(&req, &mut |_| {}, &AtomicBool::new(false)).unwrap().unwrap();
        assert!(applied.env.is_empty(), "{:?}", applied.env);
        assert!(applied.summary.contains("3 files put into"), "{}", applied.summary);
        assert!(applied.summary.contains("proxy DLLs (loaded from the game folder): winhttp"), "{}", applied.summary);
        assert_eq!(fs::read_to_string(game.join("Data/base.txt")).unwrap(), "mod");
        assert!(game.join("winhttp.dll").exists());
        let plugins = local.join("Skyrim Special Edition/Plugins.txt");
        assert!(fs::read_to_string(&plugins).unwrap().contains("*Mod.esp"));
        assert!(crate::deployment::in_use(&root, &game));
        drop(applied);
        assert!(!crate::deployment::in_use(&root, &game));
        assert!(!game.join("winhttp.dll").exists() && !game.join("Data/Mod.esp").exists());
        assert_eq!(fs::read_to_string(game.join("Data/base.txt")).unwrap(), "game");
        assert!(!plugins.exists());
    }
}
