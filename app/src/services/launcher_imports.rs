//! The other launchers' games (`heronite_import`): which launchers are on
//! this PC and how many games they have, turning the import of one on or
//! off, and keeping the ones turned on in sync (a few seconds after start,
//! then every quarter of an hour). Everything runs off the UI thread; the
//! Collection reloads on [`LauncherImportsEvent::Imported`].
//!
//! After each import it looks for the games then in the Collection twice
//! (`heronite_import::duplicates`: added here by hand, and brought in from
//! Lutris, Bottles or Faugus), tells the Collection about the new ones
//! ([`LauncherImportsEvent::Duplicates`]) and settles each as the user picks
//! in Settings → Other launchers ([`Resolve`]).
//!
//! Debug: `HERONITE_DEBUG_RESOLVE=merge|replace|keep` settles the first pair
//! found that way (for screenshots).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::*;
use heronite_data::launchers::{ImportPrefs, LastImport, Launcher};
use heronite_data::{DataPaths, Library};
use heronite_import::duplicates::Duplicate;
use heronite_import::Report;

const FIRST_DELAY: Duration = Duration::from_secs(8);
const INTERVAL: Duration = Duration::from_secs(15 * 60);

/// A launcher found on this PC.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub launcher: Launcher,
    /// Games it has that can come in.
    pub games: usize,
    pub flatpak: bool,
}

pub enum LauncherImportsEvent {
    Imported(Launcher, Report),
    /// Games found in the Collection twice, not told about before.
    Duplicates(Vec<Duplicate>),
    /// A pair was merged or the game added by hand removed.
    Resolved,
}

/// What to do with a game in the Collection twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    /// Keeps the launcher's entry and carries the other's play time,
    /// sessions, status, notes and journal over to it.
    Merge,
    /// Keeps the launcher's entry; the one added by hand goes with its data.
    Replace,
    /// Keeps the two, and never asks again.
    KeepBoth,
}

pub struct LauncherImports {
    paths: DataPaths,
    /// `None` until the first look.
    pub found: Option<Vec<Summary>>,
    pub prefs: ImportPrefs,
    /// Collection entries that came from each launcher.
    pub in_collection: BTreeMap<Launcher, usize>,
    /// Importing (or removing) right now.
    pub busy: BTreeSet<Launcher>,
    pub errors: BTreeMap<Launcher, String>,
    /// Games in the Collection twice (see [`Resolve`]).
    pub duplicates: Vec<Duplicate>,
    /// Pairs being merged or replaced right now ([`Duplicate::id`]).
    pub resolving: BTreeSet<String>,
    /// The last pair settled, or why it could not be.
    pub resolved: Option<(String, bool)>,
    /// Pairs the Collection was told about this run.
    announced: BTreeSet<String>,
    /// `HERONITE_DEBUG_RESOLVE` was acted on.
    debug_resolved: bool,
    /// What the next write of the file holds: imports finishing together
    /// write in any order, each one the latest state.
    saving: Arc<Mutex<ImportPrefs>>,
}

impl EventEmitter<LauncherImportsEvent> for LauncherImports {}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(Into::into).unwrap_or_default()
}

impl LauncherImports {
    pub fn new(paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let prefs = ImportPrefs::load(&paths);
        let saving = Arc::new(Mutex::new(prefs.clone()));
        let this = Self {
            paths,
            found: None,
            prefs,
            in_collection: BTreeMap::new(),
            busy: BTreeSet::new(),
            errors: BTreeMap::new(),
            duplicates: Vec::new(),
            resolving: BTreeSet::new(),
            resolved: None,
            announced: BTreeSet::new(),
            debug_resolved: false,
            saving,
        };
        let idle = !crate::env::flag("HERONITE_DISABLE_IDLE_JOBS");
        cx.spawn(async move |this, cx| {
            if this.update(cx, |this, cx| this.look(cx)).is_err() || !idle {
                return;
            }
            cx.background_executor().timer(FIRST_DELAY).await;
            loop {
                let Ok(()) = this.update(cx, |this, cx| this.import_enabled(cx)) else {
                    return;
                };
                cx.background_executor().timer(INTERVAL).await;
            }
        })
        .detach();
        this
    }

    /// Looks for the launchers again (the Settings section shows).
    pub fn look(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let (found, counts) = cx
                .background_spawn(async move {
                    let found: Vec<Summary> = heronite_import::detect(&home(), &paths)
                        .iter()
                        .map(|f| Summary { launcher: f.launcher, games: f.count(), flatpak: f.flatpak })
                        .collect();
                    (found, heronite_import::collection::counts(&paths))
                })
                .await;
            this.update(cx, |this, cx| {
                this.found = Some(found);
                this.in_collection = counts;
                this.find_duplicates(false, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Looks for the games in the Collection twice; with `announce`, tells
    /// the Collection about the ones it was not told about yet.
    pub fn find_duplicates(&mut self, announce: bool, cx: &mut Context<Self>) {
        let (paths, kept) = (self.paths.clone(), self.prefs.kept_apart.clone());
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    let lib = Library::load_with(&paths, true)?;
                    anyhow::Ok(heronite_import::duplicates::find(&lib, &paths, &home().to_string_lossy(), &kept))
                })
                .await;
            this.update(cx, |this, cx| {
                let found = match found {
                    Ok(found) => found,
                    Err(e) => return log::warn!("Looking for games in the Collection twice: {e:#}"),
                };
                // Pairs settled while this ran stay settled.
                let found: Vec<Duplicate> =
                    found.into_iter().filter(|d| !this.prefs.kept_apart.contains(&d.id()) && !this.resolving.contains(&d.id())).collect();
                if announce {
                    let new: Vec<Duplicate> = found.iter().filter(|d| !this.announced.contains(&d.id())).cloned().collect();
                    if !new.is_empty() {
                        log::info!("{} game(s) in the Collection twice", new.len());
                        cx.emit(LauncherImportsEvent::Duplicates(new));
                    }
                    this.announced.extend(found.iter().map(Duplicate::id));
                }
                if this.duplicates != found {
                    this.duplicates = found;
                    cx.notify();
                }
                this.debug_resolve(cx);
            })
            .ok();
        })
        .detach();
    }

    fn debug_resolve(&mut self, cx: &mut Context<Self>) {
        let Some(how) = crate::env::value("HERONITE_DEBUG_RESOLVE") else { return };
        if std::mem::replace(&mut self.debug_resolved, true) {
            return;
        }
        let how = match how.as_str() {
            "replace" => Resolve::Replace,
            "keep" => Resolve::KeepBoth,
            _ => Resolve::Merge,
        };
        if let Some(dup) = self.duplicates.first().cloned() {
            self.resolve(dup, how, cx);
        }
    }

    /// Settles a game in the Collection twice. Merge and Replace never touch
    /// the game's files: the entry added by hand leaves the Collection.
    pub fn resolve(&mut self, dup: Duplicate, how: Resolve, cx: &mut Context<Self>) {
        let id = dup.id();
        if self.resolving.contains(&id) {
            return;
        }
        if how == Resolve::KeepBoth {
            self.prefs.kept_apart.insert(id.clone());
            self.duplicates.retain(|d| d.id() != id);
            self.resolved = None;
            self.save(cx);
            cx.notify();
            return;
        }
        self.resolving.insert(id.clone());
        let paths = self.paths.clone();
        let (mine, theirs) = (dup.mine.app_name.clone(), dup.theirs.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if how == Resolve::Merge {
                        heronite_data::carry_over::carry_over(&paths, &mine, &theirs)?;
                    }
                    crate::dialogs::remove_game::remove_from_collection(&paths, "sideload", &mine)?;
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| {
                this.resolving.remove(&id);
                match res {
                    Ok(()) => {
                        // The entry added by hand is gone: so are its other pairs.
                        this.duplicates.retain(|d| d.mine.app_name != dup.mine.app_name);
                        let text = match how {
                            Resolve::Merge => {
                                heronite_i18n::tr!("{mine} was merged into {theirs}.", mine = dup.mine.title, theirs = dup.theirs.title)
                            }
                            _ => {
                                heronite_i18n::tr!("{mine} was removed; {theirs} stays.", mine = dup.mine.title, theirs = dup.theirs.title)
                            }
                        };
                        this.resolved = Some((text, false));
                        cx.emit(LauncherImportsEvent::Resolved);
                    }
                    Err(e) => {
                        log::warn!("Settling {} and {}: {e:#}", dup.mine.app_name, dup.theirs.app_name);
                        this.resolved = Some((format!("{e:#}"), true));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The launchers found that have games and are not imported yet.
    pub fn offers(&self) -> Vec<Summary> {
        self.found.iter().flatten().filter(|s| s.games > 0 && !self.prefs.enabled.contains(&s.launcher)).cloned().collect()
    }

    pub fn summary(&self, launcher: Launcher) -> Option<&Summary> {
        self.found.iter().flatten().find(|s| s.launcher == launcher)
    }

    /// Turns the import of `launcher` on (and imports now) or off (its games
    /// stay; [`Self::remove`] takes them out).
    pub fn set_enabled(&mut self, launcher: Launcher, on: bool, cx: &mut Context<Self>) {
        if on {
            // Turned on again: the games taken out before come back.
            self.prefs.removed.remove(&launcher);
            self.prefs.enabled.insert(launcher);
        } else {
            self.prefs.enabled.remove(&launcher);
        }
        self.save(cx);
        if on {
            self.import(launcher, cx);
        }
        cx.notify();
    }

    fn save(&self, cx: &mut Context<Self>) {
        *self.saving.lock().unwrap() = self.prefs.clone();
        let (saving, paths) = (self.saving.clone(), self.paths.clone());
        cx.background_spawn(async move {
            let prefs = saving.lock().unwrap();
            if let Err(e) = prefs.save(&paths) {
                log::warn!("saving the launcher imports: {e:#}");
            }
        })
        .detach();
    }

    /// Brings in what is new in every launcher turned on (the timer, the
    /// Collection's refresh button).
    pub fn import_enabled(&mut self, cx: &mut Context<Self>) {
        for launcher in self.prefs.enabled.clone() {
            self.import(launcher, cx);
        }
    }

    /// Reads `launcher`'s library and brings its games in.
    pub fn import(&mut self, launcher: Launcher, cx: &mut Context<Self>) {
        if !self.busy.insert(launcher) {
            return;
        }
        let paths = self.paths.clone();
        let removed = self.prefs.removed.get(&launcher).cloned().unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let found = heronite_import::detect(&home(), &paths).into_iter().find(|f| f.launcher == launcher);
                    let Some(found) = found else { return Err(heronite_i18n::tr!("Not found on this PC").to_string()) };
                    let report = heronite_import::sync(&paths, &found, &removed).map_err(|e| format!("{e:#}"))?;
                    Ok((report, found.count(), heronite_import::collection::counts(&paths)[&launcher]))
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy.remove(&launcher);
                match res {
                    Ok((report, games, in_collection)) => {
                        this.errors.remove(&launcher);
                        let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                        this.prefs.last.insert(launcher, LastImport { at, games });
                        this.in_collection.insert(launcher, in_collection);
                        this.save(cx);
                        if report.added > 0 || report.updated > 0 {
                            cx.emit(LauncherImportsEvent::Imported(launcher, report));
                        }
                        this.find_duplicates(true, cx);
                    }
                    Err(e) => {
                        log::warn!("Importing from {}: {e}", launcher.name());
                        this.errors.insert(launcher, e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The user took a game of `launcher` out of the Collection: its import
    /// leaves it out from now on (until it is turned off and on again).
    pub fn note_removed(&mut self, launcher: Launcher, app_name: &str, cx: &mut Context<Self>) {
        if self.prefs.removed.entry(launcher).or_default().insert(app_name.to_string()) {
            self.save(cx);
        }
    }

    /// Takes `launcher`'s games out of the Collection.
    pub fn remove(&mut self, launcher: Launcher, cx: &mut Context<Self>) {
        if !self.busy.insert(launcher) {
            return;
        }
        // All of them go: none is left out when they come back.
        if self.prefs.removed.remove(&launcher).is_some() {
            self.save(cx);
        }
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_import::collection::remove(&paths, launcher) }).await;
            this.update(cx, |this, cx| {
                this.busy.remove(&launcher);
                match res {
                    Ok(n) => {
                        this.in_collection.insert(launcher, 0);
                        if n > 0 {
                            cx.emit(LauncherImportsEvent::Imported(launcher, Report::default()));
                        }
                        this.find_duplicates(false, cx);
                    }
                    Err(e) => {
                        this.errors.insert(launcher, format!("{e:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
