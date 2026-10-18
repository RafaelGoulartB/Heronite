//! The Wine prefixes of this app's games (`heronite_wine::prefixes::list`).
//! A prefix is in use while an installed game is set to it, or when it is
//! the default or shared one (games without a prefix of their own use it).

use std::collections::HashMap;

use heronite_data::install_size::SizeCache;
use heronite_data::{game_config, Library};
use heronite_wine::prefixes::{self, PrefixKind};

use crate::{Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx, lib: &Library, walked: &SizeCache) -> Vec<Item> {
    // App name -> (key, installed). A prefix names its games by app name.
    let games: HashMap<&str, (&str, bool)> = lib.games.iter().map(|g| (g.app_name.as_str(), (g.key.as_str(), g.installed))).collect();
    let shared = std::path::PathBuf::from(game_config::shared_wine_prefix(&ctx.home_str()));
    prefixes::list(&ctx.data_paths, &ctx.home_str())
        .into_iter()
        .filter(|p| p.kind != PrefixKind::NotCreated)
        .map(|p| {
            let installed: Vec<String> = p
                .games
                .iter()
                .filter_map(|app| games.get(app.as_str()))
                .filter(|(_, installed)| *installed)
                .map(|(key, _)| key.to_string())
                .collect();
            let default = p.default || p.path == shared;
            let state = if default || !installed.is_empty() { State::InUse } else { State::Unused };
            let size = walked.get(&p.path);
            let mut item = Item::new(Group::Prefixes, Kind::Prefix { default }, state, p.name, p.path);
            item.size = size;
            item.needs_walk = size.is_none();
            item.games = installed;
            item
        })
        .collect()
}
