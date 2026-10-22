//! The Wine prefixes of this app's games (`heronite_wine::prefixes::list`).
//! A prefix is in use while an installed game is set to it, or when it is
//! the default or shared one (games without a prefix of their own use it).
//! The prefixes of the games Lutris and Faugus start are theirs: always in
//! use here (a game added by hand into one makes their folder show up).

use std::collections::HashMap;

use heronite_data::install_size::SizeCache;
use heronite_data::launchers::{managed_by, managed_paths};
use heronite_data::{game_config, Library};
use heronite_wine::prefixes::{self, PrefixKind};

use crate::{Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx, lib: &Library, walked: &SizeCache) -> Vec<Item> {
    // App name -> (key, installed). A prefix names its games by app name.
    let games: HashMap<&str, (&str, bool)> = lib.games.iter().map(|g| (g.app_name.as_str(), (g.key.as_str(), g.installed))).collect();
    let shared = std::path::PathBuf::from(game_config::shared_wine_prefix(&ctx.home_str()));
    let managed = managed_paths(&ctx.data_paths);
    prefixes::list(&ctx.data_paths, &ctx.home_str())
        .into_iter()
        .filter(|p| p.kind != PrefixKind::NotCreated)
        .map(|p| {
            let mut installed: Vec<String> = p
                .games
                .iter()
                .filter_map(|app| games.get(app.as_str()))
                .filter(|(_, installed)| *installed)
                .map(|(key, _)| key.to_string())
                .collect();
            let launcher = managed_by(&managed, &p.path);
            if let Some((key, _)) = launcher.and_then(|m| games.get(m.app_name.as_str())) {
                installed.push(key.to_string());
            }
            let default = p.default || p.path == shared;
            let state = if default || launcher.is_some() || !installed.is_empty() { State::InUse } else { State::Unused };
            let size = walked.get(&p.path);
            let mut item = Item::new(Group::Prefixes, Kind::Prefix { default }, state, p.name, p.path);
            item.size = size;
            item.needs_walk = size.is_none();
            item.games = installed;
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::{json_store, AppPaths, DataPaths};
    use serde_json::json;

    /// A game added by hand into Lutris' folder of prefixes, then removed
    /// (merged into Lutris' entry): Lutris' prefix shows, but in use.
    #[test]
    fn the_prefixes_of_another_launcher_are_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let ctx = Ctx {
            data_paths: DataPaths::at(dir.path().join("data")),
            app: AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") },
            home: home.clone(),
        };
        let (lutris, stray) = (home.join("Games/heroes-iii"), home.join("Games/old-prefix"));
        for p in [&lutris, &stray] {
            std::fs::create_dir_all(p.join("drive_c")).unwrap();
        }
        let paths = &ctx.data_paths;
        game_config::set_game_setting(paths, "mine", &ctx.home_str(), "winePrefix", json!(lutris)).unwrap();
        json_store::write(
            &paths.sideload_library(),
            &json!({"games": [{"runner": "sideload", "app_name": "lutris_7", "title": "Heroes III", "is_installed": true,
                               "install": {"install_path": lutris, "platform": "windows"}}]}),
        )
        .unwrap();
        json_store::write(
            &paths.local_meta(),
            &json!({"games": {"lutris_7": {"appName": "lutris_7", "runner": "sideload", "source": "lutris",
                "launchKind": "launcher", "title": "Heroes III", "heroniteLauncherPrefix": lutris}}}),
        )
        .unwrap();
        let lib = Library::load(paths).unwrap();
        let found = items(&ctx, &lib, &SizeCache::default());
        let of = |p: &std::path::Path| found.iter().find(|i| i.folder == p).unwrap();
        assert_eq!(of(&lutris).state, State::InUse);
        assert_eq!(of(&lutris).games, ["sideload_lutris_7"]);
        assert_eq!(of(&stray).state, State::Unused);
    }
}
