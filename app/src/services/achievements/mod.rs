//! Achievements of the Collection's games (`heronite-achievements`). Each
//! game's local copy is read once, then refreshed in the background when it
//! is old, a few seconds after a session ends, from the focus panel and the
//! dialog, and for every game from Settings. The Collection, the dialog and
//! Settings observe it (`cx.notify`); new unlocks become a desktop
//! notification.
//!
//! While a game runs, its local achievement files are checked every few
//! seconds (`heronite_achievements::live`): what the session unlocks shows
//! over the game (the overlay's achievement notices) and, when asked, as a
//! desktop notification. The check stops when the game ends.

mod emulators;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use gpui_kit::*;
use heronite_achievements::accounts::{self, Accounts};
use heronite_achievements::live;
use heronite_achievements::prefs::{self, Prefs};
use heronite_achievements::sources::{retro, Ctx};
use heronite_achievements::target::order_of;
use heronite_achievements::{store, sync, Achievement, GameAchievements, Problem, Source, SourceTarget};
use heronite_data::{AppPaths, CollectionGame, DataPaths};
use heronite_i18n::{tr, trn};
use heronite_launch::LaunchEvent;
use heronite_metadata::{Endpoints, Http};

use crate::overlay::Unlock;

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

/// The live check of a running game.
struct Live {
    key: String,
    /// When the session started: what was unlocked before is history.
    since: DateTime<Utc>,
    /// The sources it reads (local ones the options allow).
    sources: Vec<Source>,
    /// A check was asked while the game was being read: run it after.
    pending: bool,
    failures: live::Failures,
    /// Drops (and so stops) with the session.
    _task: Task<()>,
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
    /// The user signed out while an emulator was open: their login is
    /// cleared from it once it closes (`retry_running_emulators`).
    emulator_sign_out: Option<String>,
    /// The user's home (where the emulators keep their settings).
    home: std::path::PathBuf,
    /// The games that run, and which of them are checked live.
    sessions: live::Sessions,
    /// The live checks, by app name.
    lives: HashMap<String, Live>,
    /// When each running game started (its live check counts from there,
    /// even one that starts later, when the options change mid-session).
    started_at: HashMap<String, DateTime<Utc>>,
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
            emulator_sign_out: None,
            home: dirs::home_dir().unwrap_or_default(),
            sessions: live::Sessions::default(),
            lives: HashMap::new(),
            started_at: HashMap::new(),
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
        let targets = heronite_achievements::targets_with(game, &self.prefs);
        if targets.is_empty() {
            return;
        }
        let key = key_of(&game.runner, &game.app_name);
        self.known.insert(
            game.app_name.clone(),
            Known { runner: game.runner.clone(), app_name: game.app_name.clone(), title: game.title.clone(), targets },
        );
        if self.games.contains_key(&key) {
            return;
        }
        self.read_copy(key, false, cx);
    }

    /// The entry of a game by its app name (how the overlay knows the game
    /// that runs).
    pub fn entry_of_app(&self, app_name: &str) -> Option<&GameEntry> {
        let k = self.known.get(app_name)?;
        self.games.get(&key_of(&k.runner, &k.app_name))
    }

    /// The overlay is open over `app_name`: read its copy (the Collection
    /// may never have shown it), and with `refresh` read its sources again
    /// (at most once a minute, like every refresh).
    pub fn want_playing(&mut self, app_name: &str, refresh: bool, cx: &mut Context<Self>) {
        if !enabled() {
            return;
        }
        if let Some(known) = self.known.get(app_name) {
            let key = key_of(&known.runner, &known.app_name);
            if !self.games.contains_key(&key) {
                self.read_copy(key, refresh, cx);
            } else if refresh {
                self.refresh(&key, cx);
            }
            return;
        }
        let (paths, name, prefs) = (self.data_paths.clone(), app_name.to_string(), self.prefs);
        cx.spawn(async move |this, cx| {
            let Some(known) = cx.background_spawn(async move { find_game(&paths, &name, &prefs) }).await else { return };
            this.update(cx, |this, cx| {
                let key = key_of(&known.runner, &known.app_name);
                this.known.insert(known.app_name.clone(), known);
                if !this.games.contains_key(&key) {
                    this.read_copy(key, refresh, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Reads the copy of a known game, then refreshes it when it is old (or
    /// with `refresh`).
    fn read_copy(&mut self, key: String, refresh: bool, cx: &mut Context<Self>) {
        let Some(known) = self.known.values().find(|k| key_of(&k.runner, &k.app_name) == key) else { return };
        let (paths, runner, app_name) = (self.data_paths.clone(), known.runner.clone(), known.app_name.clone());
        self.games.insert(key.clone(), GameEntry { order: order_of(&known.targets), ..Default::default() });
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
                if stale || refresh {
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
        // The game runs: its live sources are read as its live check reads them.
        let session = self.lives.values().find(|l| l.key == key).map(|l| (l.sources.clone(), l.since));
        let (paths, accounts, key) = (self.data_paths.clone(), self.accounts.clone(), key.to_string());
        let lang = heronite_i18n::current().code();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    let http = Http::new(Endpoints::default());
                    let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang };
                    let out = match &session {
                        Some((sources, since)) => {
                            let session = sync::Session { sources, since: *since };
                            sync::refresh_in_session(&ctx, &known.runner, &known.app_name, &known.targets, session)
                        }
                        None => sync::refresh(&ctx, &known.runner, &known.app_name, &known.targets),
                    };
                    (out, known.title)
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
        // A refresh while the game runs (the overlay's panel asked) may
        // find the session's unlocks before its live check does.
        let session = self.lives.values().find(|l| l.key == key).map(|l| (l.since, l.sources.clone()));
        let over_game = match session {
            Some((since, sources)) => over_game(&entry.game, in_session(&out.new_unlocks, since, &sources)),
            None => Vec::new(),
        };
        self.announce(&title, &out.new_unlocks, &over_game, cx);
        cx.notify();
        self.next_in_queue(cx);
        // A live check that waited for this refresh.
        let waiting = self.lives.iter_mut().find(|(_, l)| l.key == key && l.pending).map(|(app, l)| {
            l.pending = false;
            app.clone()
        });
        if let Some(app) = waiting {
            self.live_check(&app, cx);
        }
    }

    /// Starts and stops live checks.
    fn apply(&mut self, changes: Vec<live::Change>, cx: &mut Context<Self>) {
        for change in changes {
            match change {
                live::Change::Start(app_name) => self.start_live(app_name, cx),
                live::Change::Stop(app_name) => {
                    if let Some(l) = self.lives.remove(&app_name) {
                        log::info!("achievements: stopped the live check of {}", l.key);
                    }
                }
            }
        }
    }

    /// The options changed: the live checks start again with them (their
    /// sources may differ), for the games that run now too.
    fn restart_live(&mut self, cx: &mut Context<Self>) {
        let wanted = enabled() && live::wanted(&self.prefs);
        let changes = self.sessions.restart(wanted);
        self.apply(changes, cx);
    }

    /// Achievements may have been switched on or off (any appearance
    /// change calls this): checks start for the games that run, or all
    /// stop. Checks already running keep going.
    pub fn sync_live(&mut self, cx: &mut Context<Self>) {
        let wanted = enabled() && live::wanted(&self.prefs);
        let changes = self.sessions.sync(wanted);
        self.apply(changes, cx);
    }

    /// Finds the running game (the Collection may never have shown it),
    /// then starts its check.
    fn start_live(&mut self, app_name: String, cx: &mut Context<Self>) {
        let known = self.known.get(&app_name).cloned();
        let (paths, prefs) = (self.data_paths.clone(), self.prefs);
        cx.spawn(async move |this, cx| {
            let known = match known {
                Some(k) => Some(k),
                None => {
                    let name = app_name.clone();
                    cx.background_spawn(async move { find_game(&paths, &name, &prefs) }).await
                }
            };
            this.update(cx, |this, cx| this.begin_live(app_name, known, cx)).ok();
        })
        .detach();
    }

    fn begin_live(&mut self, app_name: String, known: Option<Known>, cx: &mut Context<Self>) {
        // The game ended, or the options changed, while it was found.
        if !self.sessions.is_watched(&app_name) || self.lives.contains_key(&app_name) {
            return;
        }
        let sources = known.as_ref().map(|k| live::sources_of(&self.prefs, &k.targets)).unwrap_or_default();
        let Some(known) = known.filter(|_| !sources.is_empty()) else {
            log::debug!("achievements: {app_name} has no source to check while it runs");
            self.sessions.not_watched(&app_name);
            return;
        };
        let key = key_of(&known.runner, &known.app_name);
        let since = self.started_at.get(&app_name).copied().unwrap_or_else(Utc::now);
        let labels: Vec<&str> = sources.iter().map(|s| s.label()).collect();
        log::info!("achievements: checking {key} while it runs ({})", labels.join(", "));
        self.known.insert(app_name.clone(), known.clone());
        let targets: Vec<SourceTarget> = known.targets.iter().filter(|t| sources.contains(&t.source())).cloned().collect();
        let task = self.poll_files(app_name.clone(), known, targets, cx);
        self.lives.insert(app_name, Live { key, since, sources, pending: false, failures: live::Failures::default(), _task: task });
    }

    /// Looks at the game's files every few seconds; a change that settles
    /// asks for a check.
    fn poll_files(&self, app_name: String, known: Known, targets: Vec<SourceTarget>, cx: &mut Context<Self>) -> Task<()> {
        let (paths, accounts) = (self.data_paths.clone(), self.accounts.clone());
        cx.spawn(async move |this, cx| {
            let mut poll = live::Poll::default();
            let mut files: Arc<Vec<std::path::PathBuf>> = Arc::default();
            let mut n = 0u32;
            loop {
                if n.is_multiple_of(live::RELOCATE_EVERY) {
                    let (paths, accounts, targets) = (paths.clone(), accounts.clone(), targets.clone());
                    let (runner, app) = (known.runner.clone(), known.app_name.clone());
                    files = Arc::new(
                        cx.background_spawn(async move {
                            let http = Http::new(Endpoints::default());
                            let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
                            let pin = store::read(&paths, &runner, &app).local;
                            targets.iter().flat_map(|t| live::watched_files(&ctx, t, pin.as_ref())).collect::<Vec<_>>()
                        })
                        .await,
                    );
                }
                n = n.wrapping_add(1);
                let look = |files: &Arc<Vec<std::path::PathBuf>>| {
                    let files = files.clone();
                    cx.background_spawn(async move { live::stamp(&files) })
                };
                if poll.see(look(&files).await) == live::Seen::Changed {
                    for _ in 0..live::SETTLE_TRIES {
                        cx.background_executor().timer(live::SETTLE).await;
                        if poll.see(look(&files).await) == live::Seen::Same {
                            break;
                        }
                    }
                    if this.update(cx, |this, cx| this.live_check(&app_name, cx)).is_err() {
                        return;
                    }
                }
                cx.background_executor().timer(live::POLL).await;
                let go_on = this
                    .update(cx, |this, cx| {
                        if !enabled() {
                            this.sync_live(cx);
                        }
                        this.lives.contains_key(&app_name)
                    })
                    .unwrap_or(false);
                if !go_on {
                    return;
                }
            }
        })
    }

    /// Reads the running game's local sources again.
    fn live_check(&mut self, app_name: &str, cx: &mut Context<Self>) {
        let (Some(live), Some(known)) = (self.lives.get_mut(app_name), self.known.get(app_name).cloned()) else { return };
        let key = live.key.clone();
        let entry = self.games.entry(key.clone()).or_insert_with(|| GameEntry { order: order_of(&known.targets), ..Default::default() });
        if entry.refreshing {
            live.pending = true;
            return;
        }
        entry.refreshing = true;
        cx.notify();
        let (sources, since) = (live.sources.clone(), live.since);
        let (paths, accounts, app_name) = (self.data_paths.clone(), self.accounts.clone(), app_name.to_string());
        let lang = heronite_i18n::current().code();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    let http = Http::new(Endpoints::default());
                    let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang };
                    let out = sync::refresh_live(&ctx, &known.runner, &known.app_name, &known.targets, &sources, since);
                    (out, known.title)
                })
                .await;
            this.update(cx, |this, cx| this.live_checked(&app_name, &key, out, cx)).ok();
        })
        .detach();
    }

    fn live_checked(&mut self, app_name: &str, key: &str, (out, title): (sync::Outcome, String), cx: &mut Context<Self>) {
        let read: Vec<Source> = self.lives.get(app_name).map(|l| l.sources.clone()).unwrap_or_default();
        if let Some(entry) = self.games.get_mut(key) {
            entry.game = Arc::new(out.game);
            // The other sources' problems stay: they were not read.
            entry.problems.retain(|(s, _)| !read.contains(s));
            entry.problems.extend(out.problems.iter().cloned());
            entry.loaded = true;
            entry.refreshing = false;
        }
        let mut again = false;
        let mut over_game = Vec::new();
        if let Some(l) = self.lives.get_mut(app_name) {
            for (source, e) in l.failures.after_read(&read, &out.problems) {
                log::warn!("achievements of {key} from {} while it runs: {e}", source.label());
            }
            again = std::mem::take(&mut l.pending);
            let game = self.games.get(key).map(|e| e.game.clone()).unwrap_or_default();
            over_game = self::over_game(&game, in_session(&out.new_unlocks, l.since, &l.sources));
        }
        self.announce(&title, &out.new_unlocks, &over_game, cx);
        cx.notify();
        if again {
            self.live_check(app_name, cx);
        }
        self.next_in_queue(cx);
    }

    /// New unlocks: a desktop notification of all of them (when asked),
    /// and the ones of a running game's session over it (when asked).
    fn announce(&self, title: &str, unlocks: &[(Source, Achievement)], over_game: &[Unlock], cx: &mut Context<Self>) {
        if !unlocks.is_empty() && self.prefs.notify {
            notify_unlocks(title, unlocks);
        }
        if over_game.is_empty() || !self.prefs.overlay_notices {
            return;
        }
        let Some(overlay) = crate::overlay::get(cx) else { return };
        let shown = overlay
            .update(cx, |o, cx| over_game.iter().fold(true, |shown, u| shown & o.show_achievement(u.clone(), self.prefs.notice, cx)));
        if !shown {
            log::info!(
                "{} new achievements in {title}: the overlay cannot show them (off, or the game is in exclusive fullscreen)",
                over_game.len()
            );
        }
    }

    /// Every launch event, from the hub: a game that runs is checked live,
    /// and one that ended is read again a little later.
    pub fn on_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        match ev {
            LaunchEvent::Running { app_name, .. } => {
                self.started_at.entry(app_name.clone()).or_insert_with(Utc::now);
                let wanted = enabled() && live::wanted(&self.prefs);
                let change = self.sessions.started(app_name, wanted);
                self.apply(change.into_iter().collect(), cx);
                return;
            }
            LaunchEvent::Error { app_name, .. } => {
                self.started_at.remove(app_name);
                let change = self.sessions.ended(app_name);
                self.apply(change.into_iter().collect(), cx);
                return;
            }
            LaunchEvent::Exited { app_name, .. } => {
                self.started_at.remove(app_name);
                let change = self.sessions.ended(app_name);
                self.apply(change.into_iter().collect(), cx);
            }
            _ => return,
        }
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
                    let prefs = this.update(cx, |this, _| this.prefs).unwrap_or_default();
                    cx.background_spawn(async move { paths.and_then(|p| find_game(&p, &name, &prefs)) }).await
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
            let prefs = this.update(cx, |this, _| this.prefs).unwrap_or_default();
            let games = cx.background_spawn(async move { every_game(&paths, &prefs) }).await;
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
        let game_files_changed = prefs.game_files != self.prefs.game_files;
        let live_changed = (prefs.overlay_notices, prefs.notify, prefs.live_steam, prefs.game_files)
            != (self.prefs.overlay_notices, self.prefs.notify, self.prefs.live_steam, self.prefs.game_files);
        let user = self.signed_in_user();
        self.prefs = prefs;
        if emulators_changed {
            self.sync_emulators(user, cx);
        }
        if game_files_changed {
            // Every game's sources change: the Collection asks for them again.
            self.known.clear();
            self.games.clear();
        }
        if live_changed {
            self.restart_live(cx);
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

    /// Sets the game-files choice of a game (a folder, an app id, or off;
    /// `None` = found by itself again), or with `forget` starts its game-files
    /// set over. Then reads it again.
    pub fn pin_local(&mut self, key: &str, pin: Option<heronite_achievements::LocalPin>, forget: bool, cx: &mut Context<Self>) {
        let Some(known) = self.known.values().find(|k| key_of(&k.runner, &k.app_name) == key).cloned() else { return };
        let (paths, accounts, key) = (self.data_paths.clone(), self.accounts.clone(), key.to_string());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let http = Http::new(Endpoints::default());
                    let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
                    sync::pin_local(&ctx, &known.runner, &known.app_name, pin, forget, &order_of(&known.targets))
                })
                .await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    log::warn!("choosing the game files: {e:#}");
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

/// The unlocks of a running game's session, from its live sources, oldest
/// first: the ones to show over the game.
fn in_session(unlocks: &[(Source, Achievement)], since: DateTime<Utc>, sources: &[Source]) -> Vec<(Source, Achievement)> {
    let cutoff = since - heronite_achievements::merge::SESSION_SLACK;
    let mut out: Vec<(Source, Achievement)> =
        unlocks.iter().filter(|(s, a)| sources.contains(s) && a.unlocked_at.is_some_and(|t| t >= cutoff)).cloned().collect();
    out.sort_by_key(|(_, a)| a.unlocked_at);
    out
}

/// The notices of a game's new unlocks, each with the name of its Steam
/// Achievement Set when the game has more than one ("Base game" for the
/// unnamed one). The set comes from the merged copy: a live read (game
/// files) has no groups, and its unlocks get the stored memberships only
/// there.
fn over_game(game: &GameAchievements, unlocks: Vec<(Source, Achievement)>) -> Vec<Unlock> {
    unlocks
        .into_iter()
        .map(|(source, achievement)| {
            let set = game.sets.iter().find(|s| s.source == source).filter(|s| !s.sections().is_empty());
            let group = set
                .and_then(|s| s.group_of(s.achievements.iter().find(|a| a.id == achievement.id).unwrap_or(&achievement)))
                .map(|g| crate::ui::achievements::section_label(Some(g)));
            Unlock { achievement, source, group }
        })
        .collect()
}

/// A game of the Collection by its app name, with its sources.
fn find_game(paths: &DataPaths, app_name: &str, prefs: &Prefs) -> Option<Known> {
    let lib = heronite_data::Library::load(paths).ok()?;
    let game = lib.games.iter().find(|g| g.app_name == app_name)?;
    known_of(game, prefs)
}

fn known_of(game: &CollectionGame, prefs: &Prefs) -> Option<Known> {
    let targets = heronite_achievements::targets_with(game, prefs);
    (!targets.is_empty()).then(|| Known {
        runner: game.runner.clone(),
        app_name: game.app_name.clone(),
        title: game.title.clone(),
        targets,
    })
}

fn every_game(paths: &DataPaths, prefs: &Prefs) -> Vec<Known> {
    match heronite_data::Library::load(paths) {
        Ok(lib) => lib.games.iter().filter_map(|g| known_of(g, prefs)).collect(),
        Err(e) => {
            log::warn!("reading the Collection for the achievements: {e:#}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's own `test` macro would shadow `#[test]`.
    use super::{in_session, over_game};
    use chrono::{TimeZone, Utc};
    use heronite_achievements::{Achievement, AchievementGroup, AchievementSet, GameAchievements, Source};

    fn unlock(id: &str, minute: u32) -> Achievement {
        let at = Utc.with_ymd_and_hms(2026, 9, 19, 12, minute, 0).unwrap();
        Achievement { id: id.into(), name: id.into(), unlocked: true, unlocked_at: Some(at), ..Default::default() }
    }

    #[test]
    fn only_the_sessions_unlocks_of_live_sources_show_over_the_game() {
        let since = Utc.with_ymd_and_hms(2026, 9, 19, 12, 30, 0).unwrap();
        let unlocks = vec![
            (Source::Local, unlock("second", 50)),
            (Source::Local, unlock("before", 0)),
            (Source::Steam, unlock("steam", 45)),
            (Source::Local, unlock("first", 40)),
        ];
        let shown: Vec<String> = in_session(&unlocks, since, &[Source::Local]).into_iter().map(|(_, a)| a.id).collect();
        assert_eq!(shown, ["first", "second"]);
    }

    fn group(id: &str, name: &str, dlc: u32) -> AchievementGroup {
        AchievementGroup { id: id.into(), name: name.into(), dlc_app_id: Some(dlc), ..Default::default() }
    }

    fn in_group(id: &str, group: &str) -> Achievement {
        Achievement { group: Some(group.into()), ..unlock(id, 40) }
    }

    fn set(source: Source, achievements: Vec<Achievement>, groups: Vec<AchievementGroup>) -> AchievementSet {
        AchievementSet {
            source,
            external_id: "1".into(),
            title: None,
            fetched_at: Utc::now(),
            achievements,
            groups,
            unknown: Default::default(),
        }
    }

    #[test]
    fn the_set_shows_only_when_the_game_has_more_than_one() {
        let two =
            set(Source::Local, vec![in_group("a", "0"), in_group("b", "7")], vec![group("0", "", 0), group("7", "The Pale Reach", 7)]);
        let game = GameAchievements { sets: vec![two], ..Default::default() };
        let unlocks = vec![(Source::Local, in_group("b", "7")), (Source::Local, in_group("a", "0"))];
        let groups: Vec<Option<String>> = over_game(&game, unlocks).into_iter().map(|u| u.group).collect();
        assert_eq!(groups, [Some("The Pale Reach".to_string()), Some("Base game".to_string())]);
        // A live read's unlock has no group of its own: the copy's membership counts.
        let u = over_game(&game, vec![(Source::Local, unlock("b", 40))]);
        assert_eq!(u[0].group.as_deref(), Some("The Pale Reach"));

        // One group (Steam's unnamed base one): nothing to tell apart.
        let one = set(Source::Local, vec![in_group("a", "0"), in_group("b", "0")], vec![group("0", "", 0)]);
        let game = GameAchievements { sets: vec![one], ..Default::default() };
        let u = over_game(&game, vec![(Source::Local, in_group("a", "0"))]);
        assert_eq!(u[0].group, None);
        assert_eq!(u[0].source, Source::Local);

        // Another source's sets say nothing about this one.
        let u = over_game(&GameAchievements::default(), vec![(Source::Steam, in_group("b", "7"))]);
        assert_eq!(u[0].group, None);
    }
}
