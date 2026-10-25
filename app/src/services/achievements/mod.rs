//! Achievements of the Collection's games (`heronite-achievements`). Each
//! game's local copy is read once, then refreshed in the background when it
//! is old, a few seconds after a session ends, from the focus panel and the
//! dialog, and for every game from Settings. The Collection, the dialog and
//! Settings observe it (`cx.notify`); new unlocks become a desktop
//! notification.

mod emulators;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use gpui_kit::*;
use heronite_achievements::accounts::{self, Accounts};
use heronite_achievements::prefs::{self, Prefs};
use heronite_achievements::sources::{retro, Ctx};
use heronite_achievements::target::order_of;
use heronite_achievements::{store, sync, Achievement, GameAchievements, Problem, Source, SourceTarget};
use heronite_data::{AppPaths, CollectionGame, DataPaths};
use heronite_i18n::{tr, trn};
use heronite_launch::LaunchEvent;
use heronite_metadata::{Endpoints, Http};

/// A copy read this long ago is refreshed when the game is focused.
const STALE: chrono::Duration = chrono::Duration::hours(6);
/// A source that said "no achievements" is asked again after this long.
const STALE_NONE: chrono::Duration = chrono::Duration::days(3);
/// Wait after a game exits, so the store (or Steam's files) catch up.
const AFTER_EXIT: Duration = Duration::from_secs(20);
/// A game is not refreshed again sooner than this, whatever asks.
const MIN_GAP: Duration = Duration::from_secs(60);

/// What the service knows about one game (`runner_appName`).
#[derive(Clone, Default)]
pub struct GameEntry {
    pub game: Arc<GameAchievements>,
    /// The order of its sources (the set shown is the first with achievements).
    pub order: Vec<Source>,
    /// Why a source gave nothing at the last refresh.
    pub problems: Vec<(Source, Problem)>,
    /// The copy on disk was read.
    pub loaded: bool,
    pub refreshing: bool,
    refreshed_at: Option<Instant>,
}

impl GameEntry {
    pub fn primary(&self) -> Option<&heronite_achievements::AchievementSet> {
        self.game.primary(&self.order)
    }

    /// The problem of the first source, when it has no set to show.
    pub fn problem(&self) -> Option<&(Source, Problem)> {
        if self.primary().is_some() {
            return None;
        }
        self.problems.first()
    }
}

/// A game the service can refresh without the Collection.
#[derive(Clone)]
struct Known {
    runner: String,
    app_name: String,
    title: String,
    targets: Vec<SourceTarget>,
}

/// "Update now" of Settings: how far it got.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Bulk {
    pub done: usize,
    pub total: usize,
    pub running: bool,
}

pub struct Achievements {
    data_paths: DataPaths,
    app_paths: AppPaths,
    accounts: Accounts,
    prefs: Prefs,
    games: HashMap<String, GameEntry>,
    known: HashMap<String, Known>,
    queue: VecDeque<String>,
    pub bulk: Bulk,
    /// What each emulator's RetroAchievements login looks like, at the last
    /// write or check.
    emulators: Vec<heronite_achievements::emulators::Status>,
    /// The user's home (where the emulators keep their settings).
    home: std::path::PathBuf,
}

struct AchievementsGlobal(Entity<Achievements>);
impl Global for AchievementsGlobal {}

/// The service (created on first use; it moves the old GOG cache over).
pub fn get(cx: &mut App) -> Entity<Achievements> {
    if let Some(g) = cx.try_global::<AchievementsGlobal>() {
        return g.0.clone();
    }
    let data_paths = DataPaths::detect();
    let app_paths = AppPaths::for_data(&data_paths);
    let entity = cx.new(|cx| Achievements::new(data_paths, app_paths, cx));
    cx.set_global(AchievementsGlobal(entity.clone()));
    // A newly installed emulator gets the login too.
    entity.update(cx, |s, cx| s.sync_emulators(None, cx));
    entity
}

/// Achievements are on: their game section (Settings → Appearance → Game
/// sections, which Settings → Achievements switches too) is shown. Off, no
/// game is read or refreshed and nothing is notified.
pub fn enabled() -> bool {
    crate::theme::appearance().sections.shown(heronite_data::appearance::Section::Achievements)
}

pub fn key_of(runner: &str, app_name: &str) -> String {
    format!("{runner}_{app_name}")
}

impl Achievements {
    fn new(data_paths: DataPaths, app_paths: AppPaths, cx: &mut Context<Self>) -> Self {
        let accounts = accounts::read(&data_paths);
        let prefs = prefs::read(&app_paths);
        if heronite_achievements::migrate::needs_gog_cache(&data_paths) {
            let paths = data_paths.clone();
            cx.background_spawn(async move {
                match heronite_achievements::migrate::gog_cache(&paths) {
                    Ok(n) => log::info!("moved the achievements of {n} GOG games into the local copy"),
                    Err(e) => log::warn!("moving the GOG achievements cache: {e:#}"),
                }
            })
            .detach();
        }
        Self {
            data_paths,
            app_paths,
            accounts,
            prefs,
            games: HashMap::new(),
            known: HashMap::new(),
            queue: VecDeque::new(),
            bulk: Bulk::default(),
            emulators: Vec::new(),
            home: dirs::home_dir().unwrap_or_default(),
        }
    }

    pub fn entry(&self, key: &str) -> Option<&GameEntry> {
        self.games.get(key)
    }

    pub fn prefs(&self) -> Prefs {
        self.prefs
    }

    pub fn accounts(&self) -> &Accounts {
        &self.accounts
    }

    /// The Collection shows `game`: read its copy, and refresh it when old.
    pub fn want(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        if !enabled() {
            return;
        }
        let targets = heronite_achievements::targets_of(game);
        if targets.is_empty() {
            return;
        }
        let key = key_of(&game.runner, &game.app_name);
        self.known.insert(
            game.app_name.clone(),
            Known { runner: game.runner.clone(), app_name: game.app_name.clone(), title: game.title.clone(), targets: targets.clone() },
        );
        if self.games.contains_key(&key) {
            return;
        }
        self.games.insert(key.clone(), GameEntry { order: order_of(&targets), ..Default::default() });
        let (paths, runner, app_name) = (self.data_paths.clone(), game.runner.clone(), game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let copy = cx.background_spawn(async move { store::read(&paths, &runner, &app_name) }).await;
            this.update(cx, |this, cx| {
                let Some(entry) = this.games.get_mut(&key) else { return };
                let now = Utc::now();
                let stale = entry.order.iter().any(|s| match (copy.set(*s), copy.empty_at.get(s)) {
                    (Some(set), _) => now - set.fetched_at > STALE,
                    (None, Some(at)) => now - *at > STALE_NONE,
                    (None, None) => true,
                });
                entry.game = Arc::new(copy);
                entry.loaded = true;
                cx.notify();
                if stale {
                    this.refresh(&key, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Reads every source of a game again (no-op while it is being read, or
    /// when it was read less than a minute ago).
    pub fn refresh(&mut self, key: &str, cx: &mut Context<Self>) {
        if !enabled() {
            return;
        }
        let Some(known) = self.known.values().find(|k| key_of(&k.runner, &k.app_name) == key).cloned() else { return };
        let entry =
            self.games.entry(key.to_string()).or_insert_with(|| GameEntry { order: order_of(&known.targets), ..Default::default() });
        if entry.refreshing || entry.refreshed_at.is_some_and(|t| t.elapsed() < MIN_GAP) {
            return;
        }
        entry.refreshing = true;
        cx.notify();
        let (paths, accounts, key) = (self.data_paths.clone(), self.accounts.clone(), key.to_string());
        let lang = heronite_i18n::current().code();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    let http = Http::new(Endpoints::default());
                    let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang };
                    (sync::refresh(&ctx, &known.runner, &known.app_name, &known.targets), known.title)
                })
                .await;
            this.update(cx, |this, cx| this.refreshed(&key, out, cx)).ok();
        })
        .detach();
    }

    fn refreshed(&mut self, key: &str, (out, title): (sync::Outcome, String), cx: &mut Context<Self>) {
        let Some(entry) = self.games.get_mut(key) else { return };
        entry.game = Arc::new(out.game);
        entry.problems = out.problems;
        entry.loaded = true;
        entry.refreshing = false;
        entry.refreshed_at = Some(Instant::now());
        if !out.new_unlocks.is_empty() && self.prefs.notify {
            notify_unlocks(&title, &out.new_unlocks);
        }
        cx.notify();
        self.next_in_queue(cx);
    }

    /// Every launch event, from the hub: a game that ended is read again a
    /// little later.
    pub fn on_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        let LaunchEvent::Exited { app_name, .. } = ev else { return };
        self.retry_running_emulators(cx);
        if !enabled() {
            return;
        }
        let app_name = app_name.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AFTER_EXIT).await;
            let known = this.update(cx, |this, _| this.known.get(&app_name).cloned()).ok().flatten();
            let known = match known {
                Some(k) => Some(k),
                // The Collection never showed it (closed to the tray): find it.
                None => {
                    let paths = this.update(cx, |this, _| this.data_paths.clone()).ok();
                    let name = app_name.clone();
                    cx.background_spawn(async move { paths.and_then(|p| find_game(&p, &name)) }).await
                }
            };
            let Some(known) = known else { return };
            this.update(cx, |this, cx| {
                let key = key_of(&known.runner, &known.app_name);
                this.known.insert(known.app_name.clone(), known);
                if let Some(e) = this.games.get_mut(&key) {
                    e.refreshed_at = None;
                }
                this.refresh(&key, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Settings → "Update now": every game of the Collection with a source.
    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.bulk.running || !enabled() {
            return;
        }
        self.bulk = Bulk { done: 0, total: 0, running: true };
        cx.notify();
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let games = cx.background_spawn(async move { every_game(&paths) }).await;
            this.update(cx, |this, cx| {
                this.bulk.total = games.len();
                for k in games {
                    let key = key_of(&k.runner, &k.app_name);
                    this.known.insert(k.app_name.clone(), k);
                    if let Some(e) = this.games.get_mut(&key) {
                        e.refreshed_at = None;
                    }
                    this.queue.push_back(key);
                }
                cx.notify();
                this.next_in_queue(cx);
            })
            .ok();
        })
        .detach();
    }

    /// One game of "Update now" at a time, so the stores see a polite pace.
    fn next_in_queue(&mut self, cx: &mut Context<Self>) {
        if !self.bulk.running || self.games.values().any(|e| e.refreshing) {
            return;
        }
        while let Some(key) = self.queue.pop_front() {
            self.bulk.done += 1;
            self.refresh(&key, cx);
            if self.games.get(&key).is_some_and(|e| e.refreshing) {
                cx.notify();
                return;
            }
        }
        self.bulk.running = false;
        cx.notify();
    }

    /// Saves the RetroAchievements account (`None` signs out) and reads the
    /// games that were waiting for it.
    pub fn set_retro_account(&mut self, account: Option<accounts::RetroAccount>, cx: &mut Context<Self>) {
        let before = self.signed_in_user();
        self.accounts.retro_achievements = account;
        self.save_accounts(Source::RetroAchievements, cx);
        self.sync_emulators(before, cx);
    }

    /// The optional Web API key of the RetroAchievements account.
    pub fn set_retro_api_key(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(account) = self.accounts.retro_achievements.as_mut() else { return };
        let key = key.trim().to_string();
        if account.api_key == key {
            return;
        }
        account.api_key = key;
        self.save_accounts(Source::RetroAchievements, cx);
    }

    pub fn set_steam_key(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        self.accounts.steam_web_api_key = key.filter(|k| !k.trim().is_empty()).map(|k| k.trim().to_string());
        self.save_accounts(Source::Steam, cx);
    }

    fn save_accounts(&mut self, source: Source, cx: &mut Context<Self>) {
        let (paths, accounts) = (self.data_paths.clone(), self.accounts.clone());
        cx.background_spawn(async move {
            if let Err(e) = accounts::write(&paths, &accounts) {
                log::warn!("saving the achievement accounts: {e:#}");
            }
        })
        .detach();
        let waiting: Vec<String> = self
            .games
            .iter_mut()
            .filter(|(_, e)| e.order.contains(&source))
            .map(|(k, e)| {
                e.refreshed_at = None;
                k.clone()
            })
            .collect();
        for key in waiting {
            self.refresh(&key, cx);
        }
        cx.notify();
    }

    pub fn set_prefs(&mut self, prefs: Prefs, cx: &mut Context<Self>) {
        let emulators_changed = (prefs.emulators, prefs.hardcore) != (self.prefs.emulators, self.prefs.hardcore);
        let user = self.signed_in_user();
        self.prefs = prefs;
        if emulators_changed {
            self.sync_emulators(user, cx);
        }
        let app = self.app_paths.clone();
        cx.background_spawn(async move {
            if let Err(e) = prefs::write(&app, &prefs) {
                log::warn!("saving the achievement options: {e:#}");
            }
        })
        .detach();
        cx.notify();
    }

    /// A search of RetroAchievements games for the ROM of `key` (its
    /// consoles first), to run off the UI thread.
    pub fn retro_search(&self, key: &str, query: String) -> Option<impl FnOnce() -> Vec<retro::GameHit> + Send + 'static> {
        let target = self.known.values().find(|k| key_of(&k.runner, &k.app_name) == key)?.targets.iter().find_map(|t| match t {
            SourceTarget::Retro(r) => Some(r.clone()),
            _ => None,
        })?;
        let (paths, accounts) = (self.data_paths.clone(), self.accounts.clone());
        Some(move || {
            let http = Http::new(Endpoints::default());
            let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
            retro::search_games(&ctx, &retro::consoles::for_target(&target), &query)
        })
    }

    /// Pins the RetroAchievements game of a ROM (`None` = by its hash again).
    pub fn pin_retro_game(&mut self, key: &str, game_id: Option<u32>, cx: &mut Context<Self>) {
        let Some(known) = self.known.values().find(|k| key_of(&k.runner, &k.app_name) == key).cloned() else { return };
        let paths = self.data_paths.clone();
        let accounts = self.accounts.clone();
        let key = key.to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let http = Http::new(Endpoints::default());
                    let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
                    sync::pin_retro_game(&ctx, &known.runner, &known.app_name, game_id, &order_of(&known.targets))
                })
                .await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    log::warn!("pinning the RetroAchievements game: {e:#}");
                }
                if let Some(e) = this.games.get_mut(&key) {
                    e.refreshed_at = None;
                }
                this.refresh(&key, cx);
            })
            .ok();
        })
        .detach();
    }
}

/// Settings → Achievements' test buttons: the notification of `n` made-up
/// unlocks, as a real refresh would send it.
pub fn notify_test(n: usize) {
    let names = [tr!("First Steps"), tr!("Treasure Hunter"), tr!("Untouchable"), tr!("Speed Demon")];
    let unlocks: Vec<(Source, Achievement)> = names
        .iter()
        .cycle()
        .take(n)
        .map(|name| (Source::Steam, Achievement { name: name.to_string(), unlocked: true, ..Default::default() }))
        .collect();
    notify_unlocks(tr!("Heronite (test)"), &unlocks);
}

fn notify_unlocks(title: &str, unlocks: &[(Source, Achievement)]) {
    let names: Vec<&str> = unlocks.iter().map(|(_, a)| a.name.as_str()).take(3).collect();
    let more = unlocks.len().saturating_sub(names.len());
    let mut body = names.join(", ");
    if more > 0 {
        body.push_str(&tr!(" and {n} more", n = more));
    }
    let n = unlocks.len();
    crate::system::notify::send(trn!(n, "{n} new achievement in {title}", "{n} new achievements in {title}", title), body);
}

/// A game of the Collection by its app name, with its sources.
fn find_game(paths: &DataPaths, app_name: &str) -> Option<Known> {
    let lib = heronite_data::Library::load(paths).ok()?;
    let game = lib.games.iter().find(|g| g.app_name == app_name)?;
    known_of(game)
}

fn known_of(game: &CollectionGame) -> Option<Known> {
    let targets = heronite_achievements::targets_of(game);
    (!targets.is_empty()).then(|| Known {
        runner: game.runner.clone(),
        app_name: game.app_name.clone(),
        title: game.title.clone(),
        targets,
    })
}

fn every_game(paths: &DataPaths) -> Vec<Known> {
    match heronite_data::Library::load(paths) {
        Ok(lib) => lib.games.iter().filter_map(|g| known_of(g)).collect(),
        Err(e) => {
            log::warn!("reading the Collection for the achievements: {e:#}");
            Vec::new()
        }
    }
}
