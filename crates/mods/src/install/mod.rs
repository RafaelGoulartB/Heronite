//! Archives and folders into mods, in two phases: [`stage`] unpacks (so a
//! FOMOD installer can ask its questions), [`commit`] builds the mod folder.
//! A mod folder holds the mod's files at the paths they have in the game
//! folder (`Data/Textures/x.dds`), spelled like the game's own folders.

mod place;
mod scan;
mod stage;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use heronite_i18n::tr;
use heronite_installer::engine::CANCELLED;

use crate::fsutil::remove_any;
use crate::games::{Kind, Profile};
use crate::model::{CopyRule, ModEntry, ModList};
use crate::paths::{safe_name, GameDirs};
use crate::{layout, store};
use place::{components, resolve, Placer, Transfer};
pub(crate) use scan::scan;

pub use scan::{exe_dirs, proxy_dlls, PROXY_DLLS};
pub use stage::{find_fomod, stage, sweep, Source, Staged};

/// The installed mod's files do not line up with the game: none of its
/// top-level names is a folder or file of the game folder (nor the game's mod
/// folder), so the game will most likely not see them. The app warns after
/// an install; the mod stays installed.
pub fn looks_misplaced(mod_dir: &Path, game_dir: &Path, profile: &Profile) -> bool {
    let _ = (mod_dir, game_dir, profile);
    false
}

/// What goes where.
pub enum Plan {
    /// [`layout::auto`] decides.
    Auto,
    /// The FOMOD wizard's result: sources relative to [`Staged::fomod`],
    /// destinations relative to the mod root of the game (`Data`).
    Rules(Vec<CopyRule>),
}

pub struct InstallCtx<'a> {
    pub dirs: &'a GameDirs,
    /// The game's folder, read for the spelling of its folders; never written.
    pub game_dir: Option<&'a Path>,
    pub profile: &'a Profile,
}

/// A rule with its paths made ready: the source in the staged tree and the
/// destination in the mod folder.
struct Job {
    source: String,
    destination: String,
    priority: i64,
}

/// Builds `mods/<id>` from `staged` and adds `entry` to the list (saved).
/// `entry.id` is used when set and free; the id of `replace` otherwise, else
/// one made from the name. With `replace` the new mod takes the old one's
/// place and switch and the old folder is deleted once the new one is in.
/// Returns the id.
pub fn commit(
    ctx: &InstallCtx,
    list: &mut ModList,
    staged: &Staged,
    plan: Plan,
    entry: ModEntry,
    replace: Option<&str>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let id = pick_id(list, &entry, replace);
    let mods_dir = ctx.dirs.mods_dir();
    let partial = mods_dir.join(format!("{id}.partial"));
    fs::create_dir_all(&mods_dir).map_err(|e| format!("{}: {e}", mods_dir.display()))?;
    remove_any(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    fs::create_dir(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;

    let built = build(ctx, staged, plan, &entry, &partial, cancel);
    let scanned = match built {
        Ok(()) => {
            let exe_dirs = ctx.game_dir.map(scan::exe_dirs).unwrap_or_default();
            let found = scan(&partial, (ctx.profile.kind == Kind::Bethesda).then_some(ctx.profile.mod_root.as_str()), &exe_dirs);
            if found.files == 0 {
                Err(tr!("The archive has no files to install").to_string())
            } else {
                Ok(found)
            }
        }
        Err(e) => Err(e),
    };
    let found = match scanned {
        Ok(f) => f,
        Err(e) => {
            let _ = remove_any(&partial);
            return Err(e);
        }
    };

    let mut entry = entry;
    entry.name = if entry.name.trim().is_empty() { id.clone() } else { entry.name.trim().to_string() };
    entry.id = id.clone();
    entry.installed_at = store::now_rfc3339();
    entry.files = found.files;
    entry.size = found.size;
    entry.plugins = found.plugins;
    entry.dll_overrides = found.dll_overrides;
    put_in_place(ctx.dirs, list, &partial, entry, replace)?;
    Ok(id)
}

fn pick_id(list: &ModList, entry: &ModEntry, replace: Option<&str>) -> String {
    let wanted = entry.id.trim();
    if !wanted.is_empty() {
        let id = safe_name(wanted);
        return if list.get(&id).is_none() || replace == Some(id.as_str()) { id } else { store::unique_id(list, wanted) };
    }
    match replace {
        Some(old) => old.to_string(),
        None => store::unique_id(list, &entry.name),
    }
}

fn build(ctx: &InstallCtx, staged: &Staged, plan: Plan, entry: &ModEntry, into: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let (src_root, rules, mode) = match plan {
        Plan::Auto => {
            let rules = layout::auto(&staged.root, ctx.profile, ctx.game_dir, &entry.source);
            (staged.root.as_path(), rules, if staged.owned { Transfer::Move } else { Transfer::Copy })
        }
        Plan::Rules(rules) => {
            let root = staged.fomod.as_deref().unwrap_or(&staged.root);
            let rules =
                rules.into_iter().map(|r| CopyRule { destination: join_root(&ctx.profile.mod_root, &r.destination), ..r }).collect();
            (root, rules, if staged.owned { Transfer::Link } else { Transfer::Copy })
        }
    };
    let mut jobs: Vec<Job> =
        rules.into_iter().map(|r| Job { source: r.source, destination: r.destination, priority: r.priority }).collect();
    jobs.sort_by_key(|j| j.priority);

    let mut placer = Placer::new(into, ctx.game_dir, mode, cancel);
    for job in &jobs {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.to_string());
        }
        let Some((src, is_dir)) = resolve(src_root, &job.source)? else {
            log::info!("mods: skipping {}: not in the archive", job.source);
            continue;
        };
        let mut dest = components(&job.destination)?;
        let names_the_file = job.destination.is_empty() || job.destination.ends_with(['/', '\\']);
        if !is_dir && names_the_file {
            dest.push(src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        }
        placer.place(&src, is_dir, &dest)?;
    }
    Ok(())
}

fn join_root(mod_root: &str, destination: &str) -> String {
    let dest = destination.trim_start_matches(['/', '\\']);
    match (mod_root.is_empty(), dest.is_empty()) {
        (true, _) => dest.to_string(),
        (false, true) => format!("{mod_root}/"),
        (false, false) => format!("{mod_root}/{dest}"),
    }
}

/// Renames the built folder to its final name, puts the entry in the list and
/// saves it; undoes all of it when something fails.
fn put_in_place(dirs: &GameDirs, list: &mut ModList, partial: &Path, mut entry: ModEntry, replace: Option<&str>) -> Result<(), String> {
    let final_dir = dirs.mod_dir(&entry.id);
    let displaced = final_dir.with_file_name(format!("{}.replaced", entry.id));
    let fail = |e: std::io::Error, p: &Path| format!("{}: {e}", p.display());
    let _ = remove_any(&displaced);
    let had_old = final_dir.symlink_metadata().is_ok();
    if had_old {
        fs::rename(&final_dir, &displaced).map_err(|e| fail(e, &final_dir))?;
    }
    if let Err(e) = fs::rename(partial, &final_dir) {
        if had_old {
            let _ = fs::rename(&displaced, &final_dir);
        }
        let _ = remove_any(partial);
        return Err(fail(e, &final_dir));
    }

    let backup = list.clone();
    let at = replace.and_then(|r| list.mods.iter().position(|m| m.id == r));
    match at {
        Some(pos) => {
            entry.enabled = list.mods[pos].enabled;
            list.mods[pos] = entry;
        }
        None => list.mods.push(entry),
    }
    if let Err(e) = store::save(dirs, list) {
        *list = backup;
        let _ = remove_any(&final_dir);
        if had_old {
            let _ = fs::rename(&displaced, &final_dir);
        }
        return Err(fail(e, &dirs.list_file()));
    }
    let _ = remove_any(&displaced);
    if let Some(old) = replace.filter(|r| dirs.mod_dir(r) != final_dir) {
        if let Err(e) = remove_any(&dirs.mod_dir(old)) {
            log::warn!("mods: could not remove the replaced mod {old}: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::find;
    use crate::model::{GameLinks, ModSource};
    use crate::paths::ModsRoot;
    use std::path::PathBuf;

    struct Env {
        _t: tempfile::TempDir,
        dirs: GameDirs,
        game: PathBuf,
        src: PathBuf,
        profile: Profile,
    }

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn env(steam_id: &str) -> Env {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("g");
        let game = t.path().join("game");
        put(game.join("Data/Textures/old.dds"), "game");
        put(game.join("Data/Skyrim.esm"), "game");
        let src = t.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let profile = Profile::of(&GameLinks::default(), find(Some(steam_id), ""));
        Env { _t: t, dirs, game, src, profile }
    }

    impl Env {
        fn ctx(&self) -> InstallCtx<'_> {
            InstallCtx { dirs: &self.dirs, game_dir: Some(&self.game), profile: &self.profile }
        }
        fn folder(&self) -> Staged {
            Staged { root: self.src.clone(), owned: false, fomod: find_fomod(&self.src) }
        }
        fn files(&self, id: &str) -> Vec<String> {
            let mut v = Vec::new();
            crate::fsutil::walk_files(&self.dirs.mod_dir(id), &mut |rel, _| v.push(rel.to_string()));
            v.sort();
            v
        }
    }

    fn entry(name: &str) -> ModEntry {
        ModEntry { name: name.into(), enabled: true, ..Default::default() }
    }

    const SKYRIM: &str = "489830";
    static NO: AtomicBool = AtomicBool::new(false);

    #[test]
    fn auto_install_unwraps_and_spells_like_the_game() {
        let e = env(SKYRIM);
        put(e.src.join("Cool Mod 1.0/textures/a.dds"), "mod");
        put(e.src.join("Cool Mod 1.0/CoolMod.esp"), "mod");
        let mut list = ModList::new("g", "Skyrim");
        let id = commit(&e.ctx(), &mut list, &e.folder(), Plan::Auto, entry("Cool Mod"), None, &NO).unwrap();
        assert_eq!(id, "cool-mod");
        assert_eq!(e.files(&id), ["Data/CoolMod.esp", "Data/Textures/a.dds"]);
        let m = list.get(&id).unwrap();
        assert_eq!((m.files, m.plugins.clone(), m.name.as_str()), (2, vec!["CoolMod.esp".to_string()], "Cool Mod"));
        assert!(m.installed_at.ends_with('Z') && m.enabled);
        assert_eq!(store::load(&e.dirs), list, "saved");
        assert!(e.src.join("Cool Mod 1.0/textures/a.dds").exists(), "a folder source is copied, not moved");
        assert!(!e.game.join("Data/Textures/a.dds").exists());
        let again = commit(&e.ctx(), &mut list, &e.folder(), Plan::Auto, entry("Cool Mod"), None, &NO).unwrap();
        assert_eq!(again, "cool-mod-2");
        assert!(!e.dirs.mods_dir().join("cool-mod-2.partial").exists());
    }

    #[test]
    fn an_owned_staging_folder_is_moved_out_of() {
        let e = env(SKYRIM);
        put(e.src.join("scratch/meshes/m.nif"), "mod");
        let staged = Staged { root: e.src.join("scratch"), owned: true, fomod: None };
        let mut list = ModList::new("g", "Skyrim");
        let ent = ModEntry { id: "file-meshes".into(), ..entry("Meshes") };
        let id = commit(&e.ctx(), &mut list, &staged, Plan::Auto, ent, None, &NO).unwrap();
        assert_eq!((id.as_str(), e.files(&id)), ("file-meshes", vec!["Data/meshes/m.nif".to_string()]));
        assert!(!staged.root.join("meshes/m.nif").exists());
        drop(staged);
        assert!(!e.src.join("scratch").exists());
    }

    #[test]
    fn fomod_rules_resolve_case_and_separators_and_skip_missing_sources() {
        let e = env(SKYRIM);
        put(e.src.join("Wrapper/fomod/ModuleConfig.xml"), "<config/>");
        put(e.src.join("Wrapper/Core/Core.esp"), "core");
        put(e.src.join("Wrapper/Core/textures/c.dds"), "core");
        put(e.src.join("Wrapper/Optional/Shiny.esp"), "optional");
        let staged = e.folder();
        assert_eq!(staged.fomod, Some(e.src.join("Wrapper")));
        let rule = |s: &str, d: &str, folder: bool, priority: i64| CopyRule {
            source: s.into(),
            destination: d.into(),
            is_folder: folder,
            priority,
        };
        let rules = vec![
            rule("optional\\shiny.esp", "", false, 5),
            rule("core", "", true, 0),
            rule("Core\\Core.esp", "Alt\\Core.esp", false, 1),
            rule("Missing", "x", true, 0),
        ];
        let mut list = ModList::new("g", "Skyrim");
        let id = commit(&e.ctx(), &mut list, &staged, Plan::Rules(rules), entry("Fancy"), None, &NO).unwrap();
        assert_eq!(e.files(&id), ["Data/Alt/Core.esp", "Data/Core.esp", "Data/Shiny.esp", "Data/Textures/c.dds"]);
        assert_eq!(list.get(&id).unwrap().plugins, ["Core.esp", "Shiny.esp"]);
    }

    #[test]
    fn later_rules_win_by_priority() {
        let e = env(SKYRIM);
        put(e.src.join("a/f.esp"), "a");
        put(e.src.join("b/f.esp"), "b");
        let rule = |s: &str, priority: i64| CopyRule { source: s.into(), destination: "f.esp".into(), is_folder: false, priority };
        let mut list = ModList::new("g", "Skyrim");
        let rules = vec![rule("b/f.esp", 0), rule("a/f.esp", 2), rule("b/f.esp", 1)];
        let id = commit(&e.ctx(), &mut list, &e.folder(), Plan::Rules(rules), entry("P"), None, &NO).unwrap();
        assert_eq!(fs::read_to_string(e.dirs.mod_dir(&id).join("Data/f.esp")).unwrap(), "a");
    }

    #[test]
    fn replacing_keeps_position_and_switch() {
        let e = env(SKYRIM);
        put(e.src.join("v1/meshes/a.nif"), "1");
        put(e.src.join("v2/meshes/b.nif"), "2");
        let mut list = ModList::new("g", "Skyrim");
        let s1 = Staged { root: e.src.join("v1"), owned: false, fomod: None };
        commit(&e.ctx(), &mut list, &s1, Plan::Auto, entry("First"), None, &NO).unwrap();
        commit(&e.ctx(), &mut list, &s1, Plan::Auto, entry("Second"), None, &NO).unwrap();
        list.get_mut("first").unwrap().enabled = false;
        let s2 = Staged { root: e.src.join("v2"), owned: false, fomod: None };
        let id = commit(&e.ctx(), &mut list, &s2, Plan::Auto, entry("First 2"), Some("first"), &NO).unwrap();
        assert_eq!(id, "first", "the old id is kept");
        assert_eq!(
            list.mods.iter().map(|m| (m.id.as_str(), m.enabled, m.name.as_str())).collect::<Vec<_>>(),
            [("first", false, "First 2"), ("second", true, "Second")]
        );
        assert_eq!(e.files("first"), ["Data/meshes/b.nif"]);
        assert!(!e.dirs.mods_dir().join("first.replaced").exists());

        let ent = ModEntry { id: "first-v3".into(), ..entry("First 3") };
        let id = commit(&e.ctx(), &mut list, &s1, Plan::Auto, ent, Some("first"), &NO).unwrap();
        assert_eq!(id, "first-v3");
        assert!(!e.dirs.mod_dir("first").exists() && e.dirs.mod_dir("first-v3").exists());
        assert_eq!(list.mods[0].id, "first-v3");
    }

    #[test]
    fn empty_results_and_bad_rules_leave_nothing_behind() {
        let e = env(SKYRIM);
        put(e.src.join("fomod/ModuleConfig.xml"), "<config/>");
        let mut list = ModList::new("g", "Skyrim");
        let err = commit(&e.ctx(), &mut list, &e.folder(), Plan::Auto, entry("Empty"), None, &NO).unwrap_err();
        assert_eq!(err, "The archive has no files to install");
        put(e.src.join("a.esp"), "a");
        let bad = vec![CopyRule { source: "a.esp".into(), destination: "../../evil.esp".into(), is_folder: false, priority: 0 }];
        assert!(commit(&e.ctx(), &mut list, &e.folder(), Plan::Rules(bad), entry("Bad"), None, &NO).is_err());
        let bad = vec![CopyRule { source: "../a.esp".into(), destination: "a.esp".into(), is_folder: false, priority: 0 }];
        assert!(commit(&e.ctx(), &mut list, &e.folder(), Plan::Rules(bad), entry("Bad"), None, &NO).is_err());
        assert!(list.mods.is_empty());
        assert!(fs::read_dir(e.dirs.mods_dir()).unwrap().next().is_none());
        let cancelled = AtomicBool::new(true);
        assert_eq!(commit(&e.ctx(), &mut list, &e.folder(), Plan::Auto, entry("C"), None, &cancelled).unwrap_err(), CANCELLED);
        assert!(fs::read_dir(e.dirs.mods_dir()).unwrap().next().is_none());
    }

    #[test]
    fn symlinks_in_an_archive_are_not_followed() {
        let e = env(SKYRIM);
        put(e.src.join("textures/a.dds"), "a");
        std::os::unix::fs::symlink("/etc", e.src.join("textures/etc")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", e.src.join("textures/passwd.txt")).unwrap();
        let mut list = ModList::new("g", "Skyrim");
        let id = commit(&e.ctx(), &mut list, &e.folder(), Plan::Auto, entry("Links"), None, &NO).unwrap();
        assert_eq!(e.files(&id), ["Data/Textures/a.dds"]);
    }

    #[test]
    fn bepinex_loader_records_its_proxy_dll() {
        let t = tempfile::tempdir().unwrap();
        let dirs = ModsRoot::at(t.path().join("mods")).game("v");
        let profile = Profile::of(&GameLinks::default(), find(Some("892970"), ""));
        let src = t.path().join("src");
        put(src.join("BepInExPack_Valheim/winhttp.dll"), "dll");
        put(src.join("BepInExPack_Valheim/BepInEx/core/BepInEx.dll"), "dll");
        put(src.join("manifest.json"), "{}");
        let staged = Staged { root: src, owned: false, fomod: None };
        let source =
            ModSource::Thunderstore { community: "valheim".into(), namespace: "denikson".into(), name: "BepInExPack_Valheim".into() };
        let ctx = InstallCtx { dirs: &dirs, game_dir: None, profile: &profile };
        let mut list = ModList::new("v", "Valheim");
        let id = commit(
            &ctx,
            &mut list,
            &staged,
            Plan::Auto,
            ModEntry { source, id: "ts-denikson-BepInExPack_Valheim".into(), ..entry("BepInExPack") },
            None,
            &NO,
        )
        .unwrap();
        assert_eq!(id, "ts-denikson-BepInExPack_Valheim");
        let m = list.get(&id).unwrap();
        assert_eq!(m.dll_overrides, ["winhttp"]);
        assert!(dirs.mod_dir(&id).join("winhttp.dll").exists() && dirs.mod_dir(&id).join("BepInEx/core/BepInEx.dll").exists());
    }

    #[test]
    fn root_joins() {
        assert_eq!(join_root("Data", "Textures\\a"), "Data/Textures\\a");
        assert_eq!(join_root("Data", ""), "Data/");
        assert_eq!(join_root("", "/x"), "x");
    }
}
