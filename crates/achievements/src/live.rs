//! The live check of a running game: its local achievement files are looked
//! at every few seconds (their modified time and size only: nothing is held
//! open), and when they change the game's local sources are read again
//! (`sync::refresh_live`), so a new unlock shows while the game runs.
//!
//! Only sources this PC writes are checked: the game files of the Steam API
//! emulators and, when the user asks, the Steam client's stats files. Cloud
//! sources (GOG, Epic, RetroAchievements) wait for the refresh after the
//! game exits; their own overlays already announce unlocks during play.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use heronite_launch::probe::SystemProbe;
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;

use crate::model::{LocalPin, Source};
use crate::prefs::Prefs;
use crate::sources::{local, steam, Ctx};
use crate::sync::Problem;
use crate::target::SourceTarget;

/// How often the files are looked at.
pub const POLL: Duration = Duration::from_secs(2);
/// After a change, the files must stay the same this long before they are
/// read (a game writes its file in more than one go).
pub const SETTLE: Duration = Duration::from_millis(500);
/// A change that never settles is read after this many waits anyway.
pub const SETTLE_TRIES: u32 = 4;
/// The places are found again this often (in polls): the emulator writes
/// its saves folder at the first unlock, a Steam account may sign in.
pub const RELOCATE_EVERY: u32 = 15;
/// A source is reported as failing after this many live reads in a row
/// failed (a file read while the game writes it fails once, then reads).
pub const WARN_AFTER: u32 = 3;

/// The sources the options let a live check read.
pub fn sources(prefs: &Prefs) -> Vec<Source> {
    let mut out = Vec::new();
    if prefs.game_files {
        out.push(Source::Local);
    }
    if prefs.live_steam {
        out.push(Source::Steam);
    }
    out
}

/// A running game is checked at all: something would announce what it finds.
pub fn wanted(prefs: &Prefs) -> bool {
    (prefs.overlay_notices || prefs.notify) && !sources(prefs).is_empty()
}

/// The sources of a game a live check reads.
pub fn sources_of(prefs: &Prefs, targets: &[SourceTarget]) -> Vec<Source> {
    let allowed = sources(prefs);
    targets.iter().map(SourceTarget::source).filter(|s| allowed.contains(s)).collect()
}

/// The files whose change may mean new unlocks for `target`, including the
/// ones that do not exist yet. `pin` is the game's game-files choice (its
/// copy keeps it). Blocking: it may walk the game's folder.
pub fn watched_files(ctx: &Ctx, target: &SourceTarget, pin: Option<&LocalPin>) -> Vec<PathBuf> {
    match target {
        SourceTarget::Local(t) => {
            if pin.is_some_and(|p| p.off) {
                return Vec::new();
            }
            let mut t = t.clone();
            t.pin = pin.cloned();
            local::locate::locate_watching(&t, &local::places(ctx, &t)).map(|(_, files)| files).unwrap_or_default()
        }
        SourceTarget::Steam { app_id } => {
            let locator = SteamLocator::detect(&GlobalSettings::load(ctx.paths), &SystemProbe);
            locator
                .installs()
                .into_iter()
                .filter_map(|root| {
                    let account = heronite_steam::files::current_user(&root, &locator.home)?;
                    Some(steam::local::user_file(&root, &account, app_id))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// What the watched files look like: each one's modified time and size,
/// `None` while it does not exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stamp(Vec<(PathBuf, Option<(SystemTime, u64)>)>);

pub fn stamp(files: &[PathBuf]) -> Stamp {
    Stamp(
        files
            .iter()
            .map(|f| {
                let meta = std::fs::metadata(f).ok().filter(|m| m.is_file());
                (f.clone(), meta.and_then(|m| Some((m.modified().ok()?, m.len()))))
            })
            .collect(),
    )
}

/// What a look at the files found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// The first look: what the files were when the check started.
    Baseline,
    Same,
    Changed,
}

/// The looks at one game's files.
#[derive(Debug, Default)]
pub struct Poll {
    last: Option<Stamp>,
}

impl Poll {
    pub fn see(&mut self, now: Stamp) -> Seen {
        let seen = match &self.last {
            None => Seen::Baseline,
            Some(last) if *last == now => Seen::Same,
            Some(_) => Seen::Changed,
        };
        self.last = Some(now);
        seen
    }
}

/// The failures of a session's live reads, so each is said once, and only
/// when it lasts.
#[derive(Debug, Default)]
pub struct Failures {
    streak: HashMap<Source, u32>,
    said: HashSet<(Source, String)>,
}

impl Failures {
    /// The problems of a live read of `read`: the ones to report now.
    pub fn after_read(&mut self, read: &[Source], problems: &[(Source, Problem)]) -> Vec<(Source, String)> {
        let mut out = Vec::new();
        for source in read {
            let failed = problems.iter().find_map(|(s, p)| match p {
                Problem::Failed(e) if s == source => Some(e.clone()),
                _ => None,
            });
            let Some(message) = failed else {
                self.streak.remove(source);
                continue;
            };
            let streak = self.streak.entry(*source).or_default();
            *streak += 1;
            if *streak >= WARN_AFTER && self.said.insert((*source, message.clone())) {
                out.push((*source, message));
            }
        }
        out
    }
}

/// What to do about a game's live check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Start(String),
    Stop(String),
}

/// The games that run and the ones checked live (by app name), as games
/// start and end and the options change.
#[derive(Debug, Default)]
pub struct Sessions {
    running: BTreeSet<String>,
    watched: BTreeSet<String>,
}

impl Sessions {
    pub fn is_running(&self, app_name: &str) -> bool {
        self.running.contains(app_name)
    }

    pub fn is_watched(&self, app_name: &str) -> bool {
        self.watched.contains(app_name)
    }

    /// A game runs (again: the event may repeat once its process is known).
    pub fn started(&mut self, app_name: &str, wanted: bool) -> Option<Change> {
        self.running.insert(app_name.to_string());
        (wanted && self.watched.insert(app_name.to_string())).then(|| Change::Start(app_name.to_string()))
    }

    /// A game ended (exited or failed).
    pub fn ended(&mut self, app_name: &str) -> Option<Change> {
        self.running.remove(app_name);
        self.watched.remove(app_name).then(|| Change::Stop(app_name.to_string()))
    }

    /// A check could not start (the game has no live source): it is no
    /// longer watched, and starts again only with the next session or a
    /// change of options.
    pub fn not_watched(&mut self, app_name: &str) {
        self.watched.remove(app_name);
    }

    /// The options changed (or achievements were switched): every check
    /// stops, and the running games start again when still `wanted`.
    pub fn restart(&mut self, wanted: bool) -> Vec<Change> {
        let mut out: Vec<Change> = std::mem::take(&mut self.watched).into_iter().map(Change::Stop).collect();
        if wanted {
            self.watched = self.running.clone();
            out.extend(self.watched.iter().cloned().map(Change::Start));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::LocalTarget;

    #[test]
    fn the_options_pick_the_sources() {
        let p = Prefs { game_files: true, ..Prefs::default() };
        assert_eq!(sources(&p), [Source::Local]);
        assert!(wanted(&p));
        assert_eq!(sources(&Prefs { live_steam: true, ..p }), [Source::Local, Source::Steam]);
        // Nothing to read, or nothing to announce it: no check.
        assert!(!wanted(&Prefs::default()));
        assert!(!wanted(&Prefs { overlay_notices: false, notify: false, ..p }));
        assert!(wanted(&Prefs { overlay_notices: false, notify: true, ..p }));
        let targets = [
            SourceTarget::Gog { product_id: "1".into() },
            SourceTarget::Local(LocalTarget::default()),
            SourceTarget::Steam { app_id: "2".into() },
        ];
        assert_eq!(sources_of(&p, &targets), [Source::Local]);
    }

    #[test]
    fn a_change_of_the_files_is_seen_after_the_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("480/achievements.json");
        let files = vec![file.clone()];
        let mut poll = Poll::default();
        assert_eq!(poll.see(stamp(&files)), Seen::Baseline);
        assert_eq!(poll.see(stamp(&files)), Seen::Same);
        // The emulator writes the file at the first unlock.
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"{}").unwrap();
        assert_eq!(poll.see(stamp(&files)), Seen::Changed);
        assert_eq!(poll.see(stamp(&files)), Seen::Same);
        std::fs::write(&file, br#"{"A":{"earned":true}}"#).unwrap();
        assert_eq!(poll.see(stamp(&files)), Seen::Changed);
        // Other places found later count as a change too.
        assert_eq!(poll.see(stamp(&[file, dir.path().join("other.json")])), Seen::Changed);
    }

    #[test]
    fn a_game_files_choice_of_off_watches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = heronite_data::paths::DataPaths::at(dir.path());
        let (http, accounts) = (heronite_metadata::Http::default(), crate::accounts::Accounts::default());
        let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
        let target = SourceTarget::Local(LocalTarget { app_name: "g".into(), folders: vec![dir.path().into()], ..Default::default() });
        let off = LocalPin { off: true, ..Default::default() };
        assert!(watched_files(&ctx, &target, Some(&off)).is_empty());
        assert!(watched_files(&ctx, &SourceTarget::Gog { product_id: "1".into() }, None).is_empty());
    }

    #[test]
    fn the_places_include_the_state_file_before_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Game");
        std::fs::create_dir_all(game.join("steam_settings")).unwrap();
        std::fs::write(game.join("steam_settings/steam_appid.txt"), "480").unwrap();
        let roaming = dir.path().join("Roaming");
        let target = LocalTarget { app_name: "g".into(), folders: vec![game], ..Default::default() };
        let places = local::locate::Places { roots: vec![roaming.clone()], env_app_id: None };
        let (found, files) = local::locate::locate_watching(&target, &places).unwrap();
        assert_eq!(found.state, None);
        assert!(files.contains(&roaming.join("GSE Saves/480/achievements.json")));
    }

    #[test]
    fn a_failure_is_said_once_and_only_when_it_lasts() {
        let mut f = Failures::default();
        let broken = vec![(Source::Local, Problem::Failed("half written".into()))];
        let read = [Source::Local];
        // A file caught mid-write fails once, then reads: nothing to say.
        assert!(f.after_read(&read, &broken).is_empty());
        assert!(f.after_read(&read, &[]).is_empty());
        assert!(f.after_read(&read, &broken).is_empty());
        assert!(f.after_read(&read, &broken).is_empty());
        assert_eq!(f.after_read(&read, &broken), [(Source::Local, "half written".to_string())]);
        // The same failure is not said again in the session.
        for _ in 0..5 {
            assert!(f.after_read(&read, &broken).is_empty());
        }
    }

    #[test]
    fn checks_follow_the_games_and_the_options() {
        let mut s = Sessions::default();
        assert_eq!(s.started("a", true), Some(Change::Start("a".into())));
        // The event repeats once the process is known: one check.
        assert_eq!(s.started("a", true), None);
        assert_eq!(s.started("b", false), None);
        assert!(s.is_running("b") && !s.is_watched("b"));
        // Turned on mid-session: the running games start.
        assert_eq!(s.restart(true), [Change::Stop("a".into()), Change::Start("a".into()), Change::Start("b".into())]);
        // Turned off: every check stops, the games still run.
        assert_eq!(s.restart(false), [Change::Stop("a".into()), Change::Stop("b".into())]);
        assert!(s.is_running("a") && !s.is_watched("a"));
        assert_eq!(s.ended("a"), None);
        s.restart(true);
        assert_eq!(s.ended("b"), Some(Change::Stop("b".into())));
        assert!(!s.is_running("b"));
        // A game without a live source is not watched until it runs again.
        assert_eq!(s.started("c", true), Some(Change::Start("c".into())));
        s.not_watched("c");
        assert_eq!(s.ended("c"), None);
    }
}
