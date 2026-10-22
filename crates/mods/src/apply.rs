//! What a launch calls: mounts the enabled mods over the game's folder, lists
//! their plugins and tells which environment the game needs. Dropping the
//! result takes all of it away again.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use heronite_i18n::tr;

use crate::deploy::{self, Mount};
use crate::games::{KnownGame, Profile};
use crate::model::Progress;
use crate::paths::ModsRoot;
use crate::plugins::{self, PluginsGuard};
use crate::{store, tool};

pub struct ApplyRequest<'a> {
    pub root: &'a ModsRoot,
    /// `<data>/tools/bin`: where `fuse-overlayfs` is kept or downloaded to.
    pub tools_bin: &'a Path,
    pub game_key: &'a str,
    pub game_dir: &'a Path,
    /// The Wine prefix, when the game runs in one (for `Plugins.txt`).
    pub prefix: Option<&'a Path>,
    pub known: Option<&'static KnownGame>,
}

/// Mods in effect. Dropping it restores `Plugins.txt`, then unmounts.
pub struct Applied {
    /// Variables to add to the game's environment (`WINEDLLOVERRIDES`); merge
    /// the value with the launch's own through [`merge_dll_overrides`].
    pub env: Vec<(String, String)>,
    /// One English line for the game's log.
    pub summary: String,
    // Fields drop in this order: the plugin list goes back before the unmount.
    _plugins: Option<PluginsGuard>,
    mount: Option<Mount>,
}

impl Applied {
    pub fn game_dir(&self) -> Option<&Path> {
        self.mount.as_ref().map(Mount::game_dir)
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
    for m in &enabled {
        let dir = dirs.mod_dir(&m.id);
        if dir.is_dir() {
            layers.push(dir);
        } else {
            log::warn!("mods: the folder of {} is missing ({}); leaving it out", m.id, dir.display());
        }
    }
    if layers.is_empty() {
        return Ok(None);
    }

    let support = tool::support(req.tools_bin);
    if support.fusermount.is_none() || !support.dev_fuse {
        return Err(failed(support.problem().unwrap_or_default()));
    }
    let overlay = tool::ensure(req.tools_bin, progress, cancel).map_err(failed)?;
    // Before mounting: the game's own folders, not the mods'.
    let exe_dirs = crate::install::exe_dirs(req.game_dir);
    let mount = deploy::mount(&overlay, &dirs, req.game_dir, &layers).map_err(failed)?;

    let mut summary =
        format!("Mods: {} of {} enabled, mounted over {} with fuse-overlayfs", layers.len(), list.mods.len(), mount.game_dir().display());
    let spec = Profile::of(&list.links, req.known).plugins;
    let mut guard = None;
    if let (Some(spec), Some(prefix)) = (spec, req.prefix) {
        match plugins::write(prefix, &spec, &dirs, &list, req.game_dir) {
            Ok(Some(g)) => {
                summary.push_str(&format!("; Plugins.txt: {} plugins", g.count()));
                guard = Some(g);
            }
            Ok(None) => summary.push_str("; Plugins.txt: the prefix has no user folder yet"),
            Err(e) => {
                log::warn!("mods: Plugins.txt: {e}");
                summary.push_str(&format!("; Plugins.txt failed: {e}"));
            }
        }
    }

    // Looked at again now, not only at install: mods installed before a DLL
    // was known as a proxy get their override too.
    let mut dlls: Vec<String> = enabled.iter().flat_map(|m| m.dll_overrides.iter().map(|d| d.to_lowercase())).collect();
    dlls.extend(layers.iter().flat_map(|l| crate::install::proxy_dlls(l, &exe_dirs)));
    dlls.sort();
    dlls.dedup();
    let env = if dlls.is_empty() { Vec::new() } else { vec![("WINEDLLOVERRIDES".to_string(), merge_dll_overrides(None, &dlls))] };
    if !dlls.is_empty() {
        summary.push_str(&format!("; DLL overrides: {}", dlls.join(", ")));
    }
    Ok(Some(Applied { env, summary, _plugins: guard, mount: Some(mount) }))
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
        let req = ApplyRequest { root: &root, tools_bin: t.path(), game_key: "g", game_dir: &game, prefix: None, known: None };
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
        assert!(apply(&req, &mut |_| {}, &cancel).unwrap().is_none(), "the only mod has no folder");
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
        list.mods.push(ModEntry { id: "gone".into(), enabled: true, ..Default::default() });
        store::save(&dirs, &list).unwrap();
        let prefix = t.path().join("prefix");
        let appdata = prefix.join("drive_c/users/steamuser/AppData/Local/Skyrim Special Edition");
        fs::create_dir_all(&appdata).unwrap();
        fs::write(appdata.join("Plugins.txt"), "*Mine.esp\r\n").unwrap();

        let known = find(Some("489830"), "");
        let req = ApplyRequest { root: &root, tools_bin: t.path(), game_key: "g", game_dir: &game, prefix: Some(&prefix), known };
        let applied = apply(&req, &mut |_| {}, &AtomicBool::new(false)).unwrap().unwrap();
        assert_eq!(applied.env, [("WINEDLLOVERRIDES".to_string(), "winhttp=n,b".to_string())]);
        assert!(applied.summary.starts_with("Mods: 1 of 2 enabled, mounted over "), "{}", applied.summary);
        assert!(applied.summary.contains("Plugins.txt: 1 plugins"), "{}", applied.summary);
        assert_eq!(fs::read_to_string(game.join("Data/base.txt")).unwrap(), "mod");
        assert!(game.join("Data/Mod.esp").exists() && game.join("winhttp.dll").exists());
        assert!(fs::read_to_string(appdata.join("Plugins.txt")).unwrap().contains("*Mod.esp"));
        drop(applied);
        assert!(!game.join("Data/Mod.esp").exists());
        assert_eq!(fs::read_to_string(game.join("Data/base.txt")).unwrap(), "game");
        assert_eq!(fs::read_to_string(appdata.join("Plugins.txt")).unwrap(), "*Mine.esp\r\n");
    }
}
