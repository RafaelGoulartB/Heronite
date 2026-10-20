//! The other launchers' games (`heronite_import`): which launchers are on
//! this PC and how many games they have, turning the import of one on or
//! off, and keeping the ones turned on in sync (a few seconds after start,
//! then every quarter of an hour). Everything runs off the UI thread; the
//! Collection reloads on [`LauncherImportsEvent::Imported`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::*;
use heronite_data::launchers::{ImportPrefs, LastImport, Launcher};
use heronite_data::DataPaths;
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
        let this =
            Self { paths, found: None, prefs, in_collection: BTreeMap::new(), busy: BTreeSet::new(), errors: BTreeMap::new(), saving };
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
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn summary(&self, launcher: Launcher) -> Option<&Summary> {
        self.found.iter().flatten().find(|s| s.launcher == launcher)
    }

    /// Turns the import of `launcher` on (and imports now) or off (its games
    /// stay; [`Self::remove`] takes them out).
    pub fn set_enabled(&mut self, launcher: Launcher, on: bool, cx: &mut Context<Self>) {
        if on {
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
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let found = heronite_import::detect(&home(), &paths).into_iter().find(|f| f.launcher == launcher);
                    let Some(found) = found else { return Err(heronite_i18n::tr!("Not found on this PC").to_string()) };
                    let report = heronite_import::sync(&paths, &found).map_err(|e| format!("{e:#}"))?;
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

    /// Takes `launcher`'s games out of the Collection.
    pub fn remove(&mut self, launcher: Launcher, cx: &mut Context<Self>) {
        if !self.busy.insert(launcher) {
            return;
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
