//! Removing one item. What may hold saves or settings (prefixes, Steam's
//! Proton prefixes, leftover folders) goes to the trash, where it can be
//! restored from; caches and Wine Manager builds are deleted. Every path is
//! checked again first: it must still be what the inventory said it was, and
//! what goes to the trash must still be unused, as a fresh scan finds it (a
//! game installed there, or a prefix taken into use, since the list was made).

use std::fs;
use std::path::{Component, Path};

use anyhow::{anyhow, bail, Result};
use heronite_i18n::tr;

use crate::{leftovers, Ctx, Item, Kind, State};

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
    if !item.removable() {
        bail!(tr!("{name} cannot be removed from here", name = item.title));
    }
    let size = item.size.unwrap_or(0);
    if item.goes_to_trash() {
        still_unused(ctx, item)?;
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

/// Lists everything again, from the library as it is now, and refuses `item`
/// unless that finds it too and still unused: the state it was listed with is
/// from the scan, which may be old.
fn still_unused(ctx: &Ctx, item: &Item) -> Result<()> {
    // Hidden games are installed too.
    let lib =
        leftovers::full_library(ctx).ok_or_else(|| anyhow!(tr!("Could not check {name} again, so it was kept", name = item.title)))?;
    let same_kind = |a: &Kind, b: &Kind| std::mem::discriminant(a) == std::mem::discriminant(b);
    let found = crate::inventory(ctx, &lib)
        .items
        .iter()
        .any(|i| i.id == item.id && same_kind(&i.kind, &item.kind) && i.state == State::Unused && i.removable());
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
        assert!(still_unused(&c, &listed).is_ok());

        json_store::write(
            &c.data_paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "g", "title": "G", "is_installed": true, "install": {"install_path": folder}}]}),
        )
        .unwrap();
        let err = remove(&c, &listed).unwrap_err().to_string();
        assert!(err.contains("Old Game") && err.contains("kept"), "{err}");
        assert!(folder.join(".egstore").exists());
    }
}
