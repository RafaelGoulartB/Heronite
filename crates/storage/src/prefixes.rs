//! The Wine prefixes of this app's games (`heronite_wine::prefixes::list`).
//! A prefix is in use while an installed game is set to it, or when it is
//! the default or shared one (games without a prefix of their own use it).
//! The prefixes of the games Lutris, Bottles and Faugus start, and those of the
//! store clients, are theirs: always in use here (a game added by hand into one
//! makes their folder show up). A client's prefix lists the client's games.

use std::collections::HashMap;

use heronite_data::install_size::SizeCache;
use heronite_data::launchers::{managed_in, managed_paths};
use heronite_data::{game_config, Library};
use heronite_wine::prefixes::{self, PrefixKind};

use crate::{Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx, lib: &Library, walked: &SizeCache) -> Vec<Item> {
    // App name -> (key, installed). A prefix names its games by app name.
    let games: HashMap<&str, (&str, bool)> = lib.games.iter().map(|g| (g.app_name.as_str(), (g.key.as_str(), g.installed))).collect();
    let shared = std::path::PathBuf::from(game_config::shared_wine_prefix(&ctx.home_str()));
    let managed = managed_paths(&ctx.data_paths, &ctx.home_str());
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
            let keepers = managed_in(&managed, &p.path);
            for (key, _) in keepers.iter().filter_map(|m| games.get(m.app_name.as_str())) {
                if !installed.iter().any(|k| k == key) {
                    installed.push(key.to_string());
                }
            }
            let default = p.default || p.path == shared;
            let state = if default || !keepers.is_empty() || !installed.is_empty() { State::InUse } else { State::Unused };
            let size = walked.get(&p.path);
            let mut item = Item::new(Group::Prefixes, Kind::Prefix { default }, state, p.name, p.path);
            item.size = size;
            item.needs_walk = size.is_none();
            item.games = installed;
            item.clients = keepers.iter().filter(|m| m.app_name.is_empty()).map(|m| m.owner.to_string()).collect();
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

    /// Battle.net's prefix, set in the client's own settings file: in use with
    /// the client's games, not a prefix to clean up.
    #[test]
    fn the_prefix_of_a_store_client_is_in_use_with_its_games() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let ctx = Ctx {
            data_paths: DataPaths::at(dir.path().join("data")),
            app: AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") },
            home: home.clone(),
        };
        let (battlenet, stray) = (home.join("Games/Prefixes/Battle.net"), home.join("Games/Prefixes/old"));
        for p in [&battlenet, &stray] {
            std::fs::create_dir_all(p.join("drive_c")).unwrap();
        }
        let paths = &ctx.data_paths;
        let app = heronite_data::clients::Client::BattleNet.config_app();
        game_config::set_game_setting(paths, app, &ctx.home_str(), "winePrefix", json!(battlenet)).unwrap();
        let library = |installed: bool| {
            let game = |app: &str, title: &str, folder: &std::path::Path| {
                json!({"runner": "sideload", "app_name": app, "title": title, "is_installed": installed,
                       "install": {"install_path": folder, "platform": "windows"}})
            };
            json_store::write(
                &paths.sideload_library(),
                &json!({"games": [
                    game("battlenet_wow", "World of Warcraft", &battlenet.join("drive_c/WoW")),
                    game("ubisoft_635", "Anno 1800", &home.join("Games/Anno"))]}),
            )
            .unwrap();
        };
        library(true);
        let meta = |app: &str, source: &str| json!({"appName": app, "runner": "sideload", "source": source, "launchKind": "client"});
        json_store::write(
            &paths.local_meta(),
            &json!({"games": {"battlenet_wow": meta("battlenet_wow", "battlenet"), "ubisoft_635": meta("ubisoft_635", "ubisoft")}}),
        )
        .unwrap();

        let found = items(&ctx, &Library::load(paths).unwrap(), &SizeCache::default());
        let of = |p: &std::path::Path| found.iter().find(|i| i.folder == p).unwrap();
        assert_eq!(of(&battlenet).state, State::InUse);
        assert_eq!(of(&battlenet).games, ["sideload_battlenet_wow"], "the games inside it, not the other client's");
        assert_eq!(of(&battlenet).clients, ["Battle.net"]);
        assert_eq!((of(&stray).state, of(&stray).clients.len()), (State::Unused, 0));

        // Even with every game gone from the client, the prefix stays its own.
        library(false);
        let found = items(&ctx, &Library::load(paths).unwrap(), &SizeCache::default());
        assert_eq!(found.iter().find(|i| i.folder == battlenet).unwrap().state, State::InUse);
    }
}
