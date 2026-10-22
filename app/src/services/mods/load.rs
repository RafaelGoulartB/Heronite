//! Reading a game for the manager: its list, folder and prefix, links,
//! conflicts, what the game wrote, and whether Steam needs the DLL overrides.
//! Each part lands on its own (`ModsEvent::Changed`).

use std::sync::Arc;

use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_mods::games::{self, Profile};
use heronite_mods::{conflicts, links, overwrite, store, tool, GameLinks};

use super::facts;
use super::{GameModInfo, GameMods, ModsEvent, ModsService};

impl ModsService {
    pub(super) fn load_game(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        let key = game.key.clone();
        let game = Arc::new(game.clone());
        self.sources.insert(key.clone(), game.clone());
        let shared = self.shared_for(&key);
        let (paths, root) = (self.data_paths.clone(), self.root.clone());
        cx.spawn(async move |this, cx| {
            let (info, list, support) = cx
                .background_spawn(async move {
                    let found = facts::resolve(&paths, &game);
                    let list = shared.ensure_loaded(&root, &game.key, &game.title);
                    let known = games::find(game.steam_app_id.as_deref(), &game.title);
                    let info = GameModInfo {
                        title: game.title.clone(),
                        game_dir: found.dir,
                        prefix: found.prefix,
                        steam_app_id: game.steam_app_id.clone().filter(|id| !id.is_empty()),
                        through_steam: found.through_steam,
                        native: found.native,
                        known,
                        profile: Profile::of(&list.links, known),
                        nexus_domain: list.links.nexus_domain.clone(),
                        thunderstore: list.links.thunderstore.clone(),
                    };
                    (info, list, tool::support(&paths.root.join("tools/bin")))
                })
                .await;
            this.update(cx, |s, cx| {
                s.support = Some(support);
                s.counts.remove(&key);
                let kept = s.games.remove(&key);
                s.games.insert(
                    key.clone(),
                    GameMods {
                        info,
                        list,
                        conflicts: kept.as_ref().and_then(|g| g.conflicts.clone()),
                        overwrite: kept.as_ref().map(|g| g.overwrite).unwrap_or_default(),
                        steam_overrides_missing: kept.and_then(|g| g.steam_overrides_missing),
                    },
                );
                cx.emit(ModsEvent::Changed { game_key: key.clone() });
                s.recheck(&key, cx);
                s.find_links(&key, true, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Looks for the Nexus game and the Thunderstore community the list has no
    /// link for (the table first, then the sites). `keep`: remembers what it
    /// finds in the list, so the answer stays the same.
    pub(super) fn find_links(&mut self, game_key: &str, keep: bool, cx: &mut Context<Self>) {
        let (Some(game), Some(shared)) = (self.games.get(game_key), self.shared.get(game_key).cloned()) else { return };
        if game.list.links.nexus_domain.is_some() && game.list.links.thunderstore.is_some() {
            return;
        }
        let (cache, steam, title) = (self.root.cache_dir(), game.info.steam_app_id.clone(), game.info.title.clone());
        let have = game.list.links.clone();
        let key = game_key.to_string();
        cx.spawn(async move |this, cx| {
            let (nexus, thunderstore) = cx
                .background_spawn(async move {
                    let nexus = have.nexus_domain.is_none().then(|| links::nexus_domain_for(&cache, steam.as_deref(), &title)).flatten();
                    let ts = have.thunderstore.is_none().then(|| links::thunderstore_for(&cache, steam.as_deref(), &title)).flatten();
                    (nexus, ts)
                })
                .await;
            if nexus.is_none() && thunderstore.is_none() {
                return;
            }
            this.update(cx, |s, cx| {
                let Some(game) = s.games.get_mut(&key) else { return };
                let mut own = game.list.links.clone();
                own.nexus_domain = own.nexus_domain.or(nexus.clone());
                own.thunderstore = own.thunderstore.or(thunderstore.clone());
                game.info.nexus_domain = own.nexus_domain.clone();
                game.info.thunderstore = own.thunderstore.clone();
                game.info.profile = Profile::of(&GameLinks { mod_root: game.list.links.mod_root.clone(), ..own }, game.info.known);
                if keep {
                    let (changed, list) = shared.edit(|l| {
                        let before = l.links.clone();
                        l.links.nexus_domain = l.links.nexus_domain.take().or(nexus);
                        l.links.thunderstore = l.links.thunderstore.take().or(thunderstore);
                        l.links != before
                    });
                    if changed {
                        if let Some(game) = s.games.get_mut(&key) {
                            game.list = list;
                        }
                        s.save_later(&key, cx);
                    }
                }
                cx.emit(ModsEvent::Changed { game_key: key });
            })
            .ok();
        })
        .detach();
    }

    /// Computes the conflicts and the overwrite size, then reads Steam's
    /// launch options. Only the latest run of a game lands.
    pub(super) fn recheck(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let Some(shared) = self.shared.get(game_key).cloned() else { return };
        let run = self.checks.entry(game_key.to_string()).or_insert(0);
        *run += 1;
        let run = *run;
        let dirs = self.root.game(game_key);
        let steam = self.games.get(game_key).filter(|g| g.info.through_steam).and_then(|g| g.info.steam_app_id.clone());
        let (paths, key) = (self.data_paths.clone(), game_key.to_string());
        cx.spawn(async move |this, cx| {
            let list = shared.clone();
            let (found, written) = cx
                .background_spawn(async move {
                    let list = list.list();
                    (conflicts::compute(&dirs, &list), overwrite::summary(&dirs))
                })
                .await;
            let k = key.clone();
            let current = this
                .update(cx, |s, cx| {
                    if s.checks.get(&k) != Some(&run) {
                        return false;
                    }
                    if let Some(game) = s.games.get_mut(&k) {
                        game.conflicts = Some(Arc::new(found));
                        game.overwrite = written;
                        cx.emit(ModsEvent::Changed { game_key: k.clone() });
                    }
                    true
                })
                .unwrap_or(false);
            let Some(app_id) = steam.filter(|_| current) else { return };
            let missing = cx.background_spawn(async move { super::steam::missing_overrides(&paths, &app_id, &shared.list()) }).await;
            this.update(cx, |s, cx| {
                if s.checks.get(&key) != Some(&run) {
                    return;
                }
                if let Some(game) = s.games.get_mut(&key) {
                    if game.steam_overrides_missing != missing {
                        game.steam_overrides_missing = missing;
                        cx.emit(ModsEvent::Changed { game_key: key });
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn read_counts(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let all = cx
                .background_spawn(async move {
                    store::game_keys(&root)
                        .into_iter()
                        .map(|key| {
                            let list = store::load(&root.game(&key));
                            let counts = (list.mods.len(), list.mods.iter().filter(|m| m.enabled).count());
                            (key, counts)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |s, cx| {
                for (key, counts) in all {
                    if s.games.contains_key(&key) || s.counts.get(&key) == Some(&counts) {
                        continue;
                    }
                    s.counts.insert(key.clone(), counts);
                    cx.emit(ModsEvent::Changed { game_key: key });
                }
            })
            .ok();
        })
        .detach();
    }
}
