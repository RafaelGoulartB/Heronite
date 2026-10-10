//! Data of the focus panel's left column that the Library does not hold: the
//! user's star rating (`local_library/ratings.json`), the last Ludusavi save
//! backup, the Linux facts (ProtonDB tier, anti-cheat) and the folders of
//! the Details panel. Everything is read
//! off the UI thread when the focused game changes; ProtonDB is fetched (and
//! cached for a week) only when the cache is missing or stale.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::*;
use heronite_data::anticheat::{self, AntiCheatInfo};
use heronite_data::models::LocalGameSession;
use heronite_data::timeline::{build_timeline, TimelineDay};
use heronite_data::{ratings, settings, CollectionGame};
use heronite_emulation::ludusavi::last_backup;
use heronite_metadata::{protondb, Endpoints, Http};

use super::open_links::{game_folders, FolderKind};
use super::CollectionView;

/// Days of the timeline shown before "Show more".
pub(super) const TIMELINE_DAYS: usize = 4;

/// The last Ludusavi backup of the focused game.
#[derive(Clone)]
pub(super) enum Backup {
    /// Ludusavi is off (the row is hidden).
    Off,
    /// On, but nothing backed up for this game yet.
    Never,
    At(SystemTime),
}

#[derive(Clone, Default)]
pub(super) struct Linux {
    pub native: bool,
    /// Steam id of a game rated by ProtonDB (its page opens on click).
    pub steam_app_id: Option<String>,
    /// ProtonDB tier, lowercase (`gold`, `borked`...).
    pub tier: Option<String>,
    pub anticheat: Option<AntiCheatInfo>,
}

impl Linux {
    pub fn is_empty(&self) -> bool {
        !self.native && self.tier.is_none() && self.anticheat.is_none()
    }
}

#[derive(Default)]
pub(super) struct Facts {
    /// Game the facts below belong to (or are loading for).
    key: Option<SharedString>,
    pub backup: Option<Backup>,
    pub linux: Option<Linux>,
    /// Folders that exist here (install, or emulator + ROM), for Details.
    pub folders: Vec<(FolderKind, PathBuf)>,
    /// Stars by game key; `None` until `ratings.json` was read.
    ratings: Option<HashMap<String, u8>>,
    ratings_loading: bool,
    /// Star under the pointer (preview of a new rating).
    pub rating_hover: Option<u8>,
    pub status_open: bool,
    /// The other ways to install, under the Install button.
    pub install_menu_open: bool,
    /// Days of the timeline shown.
    pub timeline_days: usize,
    /// Timeline of the focused game and what it was built from.
    timeline: Option<(String, Arc<Vec<TimelineDay>>)>,
}

impl Facts {
    pub fn rating(&self, key: &str) -> Option<u8> {
        self.ratings.as_ref().and_then(|r| r.get(key).copied())
    }
}

impl CollectionView {
    /// Runs on every render: loads the ratings once and the facts of a newly
    /// focused game.
    pub(super) fn sync_facts(&mut self, cx: &mut Context<Self>) {
        if self.facts.ratings.is_none() && !self.facts.ratings_loading {
            self.facts.ratings_loading = true;
            let paths = self.heroic.clone();
            cx.spawn(async move |this, cx| {
                let map = cx.background_spawn(async move { ratings::load(&paths) }).await;
                this.update(cx, |this, cx| {
                    // Ratings set while the file loaded win over the file.
                    let have = this.facts.ratings.get_or_insert_with(HashMap::new);
                    for (k, v) in map {
                        have.entry(k).or_insert(v);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }

        let target = self.selected.clone();
        if self.facts.key == target {
            return;
        }
        self.facts.key = target.clone();
        self.facts.backup = None;
        self.facts.linux = None;
        self.facts.folders.clear();
        self.facts.rating_hover = None;
        // `HERONITE_DEBUG_FOCUS_STATUS=1` opens the status selector (screenshots).
        self.facts.status_open = crate::env::value("HERONITE_DEBUG_FOCUS_STATUS").is_some();
        // `HERONITE_DEBUG_INSTALL_MENU=1`: the other ways to install.
        self.facts.install_menu_open = crate::env::value("HERONITE_DEBUG_INSTALL_MENU").is_some();
        self.facts.timeline_days = TIMELINE_DAYS;
        let (Some(lib), Some(key)) = (self.lib.clone(), target) else { return };
        let Some(game) = lib.get(&key).cloned() else { return };
        self.load_facts(key, game, cx);
    }

    fn load_facts(&mut self, key: SharedString, game: Arc<CollectionGame>, cx: &mut Context<Self>) {
        let paths = self.heroic.clone();
        let cache = self.app_paths.cache.join("protondb");
        let emulated = game.launch_kind.as_deref() == Some("emulator");
        // The game's own Steam id, else the one the metadata found.
        let known_steam = [game.steam_app_id.as_deref(), game.metadata.as_ref().and_then(|m| m.steam_app_id.as_deref())]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .map(str::to_owned);
        // Steam games are rated by ProtonDB (Heronite flags its `steam-uri`
        // entries as native because the Steam client is); the rest are native
        // when their install is a Linux build. Emulated games get no Linux row.
        let native = known_steam.is_none()
            && !emulated
            && (game.game.is_linux_native == Some(true)
                || game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("linux")));
        // Other stores: ProtonDB only knows Steam ids, so look it up by title.
        let by_title = known_steam.is_none() && !native && !emulated;
        cx.spawn(async move |this, cx| {
            let (local, net) = cx
                .background_spawn(async move {
                    let enabled = settings::load(&paths).map(|s| s.ludusavi.enabled).unwrap_or(false);
                    let backup = if !enabled {
                        Backup::Off
                    } else {
                        match last_backup(&paths, &game.title, known_steam.as_deref()) {
                            Some(b) => Backup::At(b.at),
                            None => Backup::Never,
                        }
                    };
                    let folders = game_folders(&paths, &game);
                    let looked_up = by_title.then(|| protondb::cached_steam_id_for_title(&cache, &game.title)).flatten();
                    let steam = known_steam.or_else(|| looked_up.clone().flatten());
                    let cached = steam.as_deref().and_then(|id| protondb::cached_tier(&cache, id));
                    let linux = (!emulated).then(|| Linux {
                        native,
                        steam_app_id: steam.clone(),
                        tier: cached.as_ref().map(|(t, _)| t.clone()),
                        anticheat: anticheat::lookup(&paths, steam.as_deref(), &game.title),
                    });
                    // Next, on the network: the title search, then a missing or stale tier.
                    let search = (by_title && looked_up.is_none()).then(|| game.title.clone());
                    let fetch = steam.filter(|_| !cached.as_ref().is_some_and(|(_, fresh)| *fresh));
                    ((backup, folders, linux), (cache, search, fetch))
                })
                .await;
            let (backup, folders, linux) = local;
            let current = this
                .update(cx, |this, cx| {
                    if this.facts.key.as_ref() != Some(&key) {
                        return false;
                    }
                    this.facts.backup = Some(backup);
                    this.facts.folders = folders;
                    this.facts.linux = linux;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            let (cache, search, fetch) = net;
            if !current || (search.is_none() && fetch.is_none()) {
                return;
            }
            let (id, tier) = cx
                .background_spawn(async move {
                    let http = Http::new(Endpoints::default());
                    let id = fetch.or_else(|| search.and_then(|title| protondb::steam_id_for_title(&cache, &http, &title)));
                    let tier = id.as_deref().and_then(|id| protondb::tier(&cache, &http, id));
                    (id, tier)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.facts.key.as_ref() == Some(&key) {
                    if let Some(l) = this.facts.linux.as_mut() {
                        l.steam_app_id = id;
                        l.tier = tier;
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sets (or, with `None`, clears) the rating of `key`; saved off the UI thread.
    pub(super) fn set_rating(&mut self, key: &str, stars: Option<u8>, cx: &mut Context<Self>) {
        let map = self.facts.ratings.get_or_insert_with(HashMap::new);
        match stars {
            Some(s) => map.insert(key.to_string(), s),
            None => map.remove(key),
        };
        let (paths, key) = (self.heroic.clone(), key.to_string());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { ratings::set_rating(&paths, &key, stars) }).await;
            if let Err(err) = res {
                this.update(cx, |this, cx| this.show_toast(format!("Could not save the rating: {err:#}"), true, cx)).ok();
            }
        })
        .detach();
        cx.notify();
    }

    /// Days of play and screenshots of `key`, newest first (cached until the
    /// sessions or the listed screenshots change).
    pub(super) fn timeline_days(&mut self, key: &SharedString) -> Option<Arc<Vec<TimelineDay>>> {
        let lib = self.lib.clone()?;
        let game = lib.get(key)?;
        let sessions: &[LocalGameSession] = game.sessions.as_deref().map(Vec::as_slice).unwrap_or(&[]);
        let shots = if self.shots.game.as_ref() == Some(key) { self.shots.items.as_slice() } else { &[] };
        let sig = format!(
            "{key}:{}:{}:{}:{}",
            sessions.len(),
            sessions.first().map(|s| s.started_at.as_str()).unwrap_or(""),
            self.shots.epoch,
            shots.len()
        );
        if let Some((have, days)) = &self.facts.timeline {
            if *have == sig {
                return Some(days.clone());
            }
        }
        let taken: Vec<Option<f64>> = shots.iter().map(|s| s.taken_at).collect();
        let days = Arc::new(build_timeline(sessions, &taken, &chrono::Local));
        self.facts.timeline = Some((sig, days.clone()));
        Some(days)
    }
}
