//! Removing items. What may hold saves or settings (prefixes, Steam's
//! Proton prefixes, leftover folders) goes to the trash, where it can be
//! restored from; caches and Wine Manager builds are deleted. Every path is
//! checked again first: it must still be what the inventory said it was, and
//! what goes to the trash, and the builds, must still be unused, as a fresh
//! scan finds it (a game installed there, or a prefix or build taken into use,
//! since the list was made). A batch takes that scan once for all its items.

use std::fs;
use std::path::{Component, Path};

use anyhow::{anyhow, bail, Result};
use heronite_i18n::tr;

use crate::{leftovers, Ctx, Inventory, Item, Kind, State};

/// What a removal did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Removed {
    /// Bytes deleted for good.
    pub deleted: u64,
    /// Bytes moved to the trash.
    pub trashed: u64,
}

/// Removes `item`. Blocking. Games are refused: they go through their own
/// uninstall.
pub fn remove(ctx: &Ctx, item: &Item) -> Result<Removed> {
    remove_all(ctx, std::slice::from_ref(item)).remove(0)
}

/// Removes `items` one after the other, with the result of each. Blocking.
/// The library and the disk are read once for the whole batch, however many
/// items there are, and every item is checked against that one scan.
pub fn remove_all(ctx: &Ctx, items: &[Item]) -> Vec<Result<Removed>> {
    let fresh = items.iter().any(needs_fresh_scan).then(|| fresh_scan(ctx)).flatten();
    items.iter().map(|item| remove_one(ctx, item, fresh.as_ref())).collect()
}

/// Whether `item` must be unused in a fresh scan before it goes: what is
/// trashed, and the builds (deleted for good, and games may use them).
fn needs_fresh_scan(item: &Item) -> bool {
    item.removable() && (item.goes_to_trash() || matches!(item.kind, Kind::Build { .. }))
}

/// `fresh` is the scan of [`fresh_scan`], `None` when it could not be made.
fn remove_one(ctx: &Ctx, item: &Item, fresh: Option<&Inventory>) -> Result<Removed> {
    if !item.removable() {
        bail!(tr!("{name} cannot be removed from here", name = item.title));
    }
    let size = item.size.unwrap_or(0);
    if needs_fresh_scan(item) {
        still_unused(item, fresh)?;
    }
    match &item.kind {
        Kind::Prefix { .. } => {
            heronite_wine::prefixes::trash(&ctx.data_paths, &ctx.home_str(), item.path())?;
            Ok(Removed { trashed: size, ..Default::default() })
        }
        Kind::SteamPrefix { .. } | Kind::Leftover => {
            check_folder(ctx, item.path(), matches!(item.kind, Kind::SteamPrefix { .. }).then_some("compatdata"))?;
            trash::delete(item.path())
                .map_err(|e| anyhow!(tr!("Could not move {path} to the trash: {error}", path = item.path().display(), error = e)))?;
            Ok(Removed { trashed: size, ..Default::default() })
        }
        Kind::SteamShaders { .. } => {
            check_folder(ctx, item.path(), Some("shadercache"))?;
            delete(item.path())?;
            Ok(Removed { deleted: size, ..Default::default() })
        }
        Kind::Build { version, .. } => {
            let info = heronite_wine::store::read_releases(&ctx.data_paths)
                .into_iter()
                .find(|r| &r.version == version && r.installed())
                .ok_or_else(|| anyhow!(tr!("{name} is not installed", name = version)))?;
            heronite_wine::install::remove_version(&ctx.data_paths, &info)?;
            heronite_wine::steam_link::sync(&ctx.data_paths);
            Ok(Removed { deleted: size, ..Default::default() })
        }
        Kind::Cache(_) => {
            for path in &item.paths {
                check_folder(ctx, path, None)?;
                delete(path)?;
            }
            Ok(Removed { deleted: size, ..Default::default() })
        }
        Kind::Game { .. } | Kind::Runtime => bail!(tr!("{name} cannot be removed from here", name = item.title)),
    }
}

/// Lists everything again, from the library as it is now, hidden games
/// included (they are installed too). `None` when the library cannot be read.
fn fresh_scan(ctx: &Ctx) -> Option<Inventory> {
    let lib = leftovers::full_library(ctx)?;
    Some(crate::scan(ctx, &lib, Some(&lib)))
}

/// Refuses `item` unless the fresh scan finds it too and still unused: the
/// state it was listed with is from the earlier scan, which may be old.
fn still_unused(item: &Item, fresh: Option<&Inventory>) -> Result<()> {
    let Some(fresh) = fresh else { bail!(tr!("Could not check {name} again, so it was kept", name = item.title)) };
    let same_kind = |a: &Kind, b: &Kind| std::mem::discriminant(a) == std::mem::discriminant(b);
    let found = fresh.items.iter().any(|i| i.id == item.id && same_kind(&i.kind, &item.kind) && i.state == State::Unused && i.removable());
    if !found {
        bail!(tr!("{name} is in use or changed since it was listed, so it was kept. Scan again", name = item.title));
    }
    Ok(())
}

/// Refuses the home folder and anything above it, shallow paths (`/data`),
/// relative ones, and with `parent`, a folder not directly in a folder of
/// that name.
fn check_folder(ctx: &Ctx, path: &Path, parent: Option<&str>) -> Result<()> {
    let depth = path.components().filter(|c| matches!(c, Component::Normal(_))).count();
    if !path.is_absolute() || depth < 3 || ctx.home.starts_with(path) || path.components().any(|c| c == Component::ParentDir) {
        bail!(tr!("Refusing to remove {path}", path = path.display()));
    }
    if let Some(name) = parent {
        if path.parent().and_then(Path::file_name).is_none_or(|n| n != name) {
            bail!(tr!("Refusing to remove {path}", path = path.display()));
        }
    }
    Ok(())
}

/// Deletes a file or a folder; one already gone is fine.
fn delete(path: &Path) -> Result<()> {
    let res = match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => Err(e),
    };
    res.map_err(|e| anyhow!(tr!("Could not delete {path}: {error}", path = path.display(), error = e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CacheKind, Group, State};
    use heronite_data::{AppPaths, DataPaths};
    use std::path::PathBuf;

    fn ctx(dir: &Path) -> Ctx {
        Ctx {
            data_paths: DataPaths::at(dir.join("data")),
            app: AppPaths { config: dir.join("cfg"), cache: dir.join("cache") },
            home: dir.join("home"),
        }
    }

    #[test]
    fn deletes_caches_and_refuses_games_and_odd_paths() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let cache = dir.path().join("cache/thumbs");
        fs::create_dir_all(cache.join("ab")).unwrap();
        fs::write(cache.join("ab/x"), [0u8; 10]).unwrap();
        let mut item = Item::new(Group::Caches, Kind::Cache(CacheKind::Images), State::Cache, "x", cache.clone());
        item.size = Some(10);
        assert_eq!(remove(&c, &item).unwrap(), Removed { deleted: 10, trashed: 0 });
        assert!(!cache.exists());
        // Gone already: still fine.
        assert!(remove(&c, &item).is_ok());

        let game = Item::new(Group::Games, Kind::Game { key: "k".into() }, State::InUse, "Game", dir.path().join("g"));
        assert!(remove(&c, &game).is_err());
        let used = Item::new(Group::Prefixes, Kind::Prefix { default: false }, State::InUse, "P", dir.path().join("p"));
        assert!(remove(&c, &used).is_err());

        assert!(check_folder(&c, Path::new("/data"), None).is_err());
        assert!(check_folder(&c, &c.home, None).is_err());
        assert!(check_folder(&c, &PathBuf::from("/a/b/../c"), None).is_err());
        assert!(check_folder(&c, Path::new("/s/steamapps/compatdata/10"), Some("compatdata")).is_ok());
        assert!(check_folder(&c, Path::new("/s/steamapps/common/10"), Some("compatdata")).is_err());
    }

    /// A leftover listed by a scan that is installed into before "Remove" is
    /// refused: the item says unused, the library now says otherwise.
    #[test]
    fn refuses_a_leftover_that_became_referenced_after_the_scan() {
        use heronite_data::json_store;
        use serde_json::json;
        use std::time::{Duration, SystemTime};

        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let folder = c.home.join("Games/Heronite/Old Game");
        fs::create_dir_all(folder.join(".egstore")).unwrap();
        fs::File::open(&folder).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3 * 3600)).unwrap();

        let lib = heronite_data::Library::load(&c.data_paths).unwrap();
        let listed = crate::inventory(&c, &lib).items.into_iter().find(|i| i.folder == folder).expect("listed by the scan");
        assert!(listed.removable());
        assert!(still_unused(&listed, fresh_scan(&c).as_ref()).is_ok());

        json_store::write(
            &c.data_paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "g", "title": "G", "is_installed": true, "install": {"install_path": folder}}]}),
        )
        .unwrap();
        let err = remove(&c, &listed).unwrap_err().to_string();
        assert!(err.contains("Old Game") && err.contains("kept"), "{err}");
        assert!(folder.join(".egstore").exists());
    }

    /// A batch scans once: the item that became in use is refused, and the
    /// others go (nothing here goes to the trash, which is the user's).
    #[test]
    fn a_batch_refuses_what_became_used_and_removes_the_rest() {
        use heronite_data::json_store;
        use serde_json::json;
        use std::time::{Duration, SystemTime};

        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let folder = c.home.join("Games/Heronite/Old Game");
        fs::create_dir_all(folder.join(".egstore")).unwrap();
        fs::File::open(&folder).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3 * 3600)).unwrap();
        let cache = dir.path().join("cache/thumbs");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("x"), [0u8; 10]).unwrap();

        let lib = heronite_data::Library::load(&c.data_paths).unwrap();
        let listed = crate::inventory(&c, &lib).items;
        let leftover = listed.iter().find(|i| i.folder == folder).expect("listed by the scan").clone();
        let mut thumbs = listed.iter().find(|i| i.folder == cache).expect("listed by the scan").clone();
        thumbs.size = Some(10);

        json_store::write(
            &c.data_paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "g", "title": "G", "is_installed": true, "install": {"install_path": folder}}]}),
        )
        .unwrap();
        let results = remove_all(&c, &[leftover, thumbs]);
        assert!(results[0].as_ref().unwrap_err().to_string().contains("kept"));
        assert_eq!(*results[1].as_ref().unwrap(), Removed { deleted: 10, trashed: 0 });
        assert!(folder.join(".egstore").exists() && !cache.exists());
    }

    /// A build deleted for good is checked again too: a game now set to it
    /// keeps it.
    #[test]
    fn refuses_a_build_that_a_game_now_uses() {
        use heronite_data::{game_config, json_store};
        use heronite_wine::WineVersionInfo;
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let paths = &c.data_paths;
        let install = paths.tools_dir().join("proton/GE-Proton9-1");
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("proton"), "x").unwrap();
        let info = WineVersionInfo {
            version: "GE-Proton9-1".into(),
            kind: "GE-Proton".into(),
            install_dir: Some(install.to_string_lossy().into_owned()),
            is_installed: Some(true),
            ..Default::default()
        };
        heronite_wine::store::mark_installed(paths, &info).unwrap();

        let lib = heronite_data::Library::load(paths).unwrap();
        let listed = crate::inventory(&c, &lib).items.into_iter().find(|i| i.folder == install).expect("listed by the scan");
        assert!(listed.removable());
        assert!(still_unused(&listed, fresh_scan(&c).as_ref()).is_ok());

        json_store::write(
            &paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "g", "title": "G", "is_installed": true, "install": {}}]}),
        )
        .unwrap();
        let wine_version = json!({"bin": install.join("proton"), "name": "GE-Proton9-1", "type": "proton"});
        game_config::set_game_setting(paths, "g", &c.home.to_string_lossy(), "wineVersion", wine_version).unwrap();
        let err = remove(&c, &listed).unwrap_err().to_string();
        assert!(err.contains("GE-Proton9-1") && err.contains("kept"), "{err}");
        assert!(install.join("proton").exists());
    }
}
