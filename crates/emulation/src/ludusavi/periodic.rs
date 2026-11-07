//! Save backups while a game runs (`ludusavi.intervalMinutes`, off unless
//! set): a thread per running game backs it up every interval until the
//! session ends, and the after-play backup still follows.
//!
//! Ludusavi copies the save files as they are at that moment. A game that
//! writes while the copy runs can leave a backup that mixes old and new
//! files: this is not an application-consistent snapshot, and the after-play
//! backup is the one taken with the game closed. Windows registry entries
//! are copied the same way and are missing from the restore preview. Nothing
//! is ever restored automatically.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use heronite_data::settings;

use super::backup::{backup_for_game, BackupReason, LudusaviBackupResult, LudusaviEnv, LudusaviGame};

/// The backups of one running game. Dropping it cancels them without
/// waiting; [`Self::finish`] also waits for one still running.
pub struct PeriodicBackups {
    stop: Arc<Stop>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Stop {
    cancelled: Mutex<bool>,
    wake: Condvar,
}

impl Stop {
    /// Waits `d`; `false` when cancelled first (or before).
    fn sleep(&self, d: Duration) -> bool {
        let deadline = Instant::now() + d;
        let mut cancelled = self.cancelled.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if *cancelled {
                return false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            cancelled = self.wake.wait_timeout(cancelled, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn cancel(&self) {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_all();
    }
}

impl PeriodicBackups {
    /// Runs `job` after `first`, then after each interval it returns, on its
    /// own thread, until it returns `None` or the backups are cancelled. One
    /// job runs at a time: the next wait starts when it returns.
    pub fn start(name: &str, first: Duration, mut job: impl FnMut() -> Option<Duration> + Send + 'static) -> std::io::Result<Self> {
        let stop = Arc::new(Stop::default());
        let s = stop.clone();
        let thread = std::thread::Builder::new().name(format!("ludusavi-periodic-{name}")).spawn(move || {
            let mut wait = first;
            while s.sleep(wait) {
                match job() {
                    Some(next) => wait = next,
                    None => break,
                }
            }
        })?;
        Ok(Self { stop, thread: Some(thread) })
    }

    /// No more backups; returns at once (one already running ends on its own).
    pub fn cancel(&self) {
        self.stop.cancel();
    }

    /// Cancels and waits for a backup still running, so the after-play one
    /// does not find Ludusavi busy. Blocking: never on the UI thread.
    pub fn finish(mut self) {
        self.stop.cancel();
        if let Some(t) = self.thread.take() {
            if t.join().is_err() {
                log::error!("the periodic Ludusavi backup thread panicked");
            }
        }
    }
}

impl Drop for PeriodicBackups {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// What one periodic backup did.
#[derive(Debug, Default, PartialEq)]
pub struct PeriodicTick {
    /// `None` when no backup was tried (turned off since the game started).
    pub result: Option<LudusaviBackupResult>,
    /// Time to the next backup from the settings read now; `None` stops them
    /// for this session (interval or automatic backups turned off, or the
    /// game left out of them).
    pub next: Option<Duration>,
}

/// One backup while `game` runs; `current` is the interval in use, kept when
/// the settings cannot be read.
pub fn periodic_backup(env: &LudusaviEnv, game: &LudusaviGame, current: Duration) -> PeriodicTick {
    let next = match settings::load(env.paths) {
        Ok(s) => s.ludusavi.periodic_interval_for(&game.runner, &game.app_name),
        Err(e) => {
            let error = format!("{e:#}");
            log::error!("reading the settings for the periodic Ludusavi backup of {}: {error}", game.title);
            return PeriodicTick { result: Some(LudusaviBackupResult { error: Some(error), ..Default::default() }), next: Some(current) };
        }
    };
    if next.is_none() {
        log::info!("periodic Ludusavi backups of {} were turned off; none until it starts again", game.title);
        return PeriodicTick::default();
    }
    let result = backup_for_game(env, game, BackupReason::Periodic);
    if let Some(e) = &result.error {
        log::error!("periodic Ludusavi backup of {} failed: {e}", game.title);
    }
    PeriodicTick { result: Some(result), next }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use heronite_data::DataPaths;
    use heronite_launch::probe::FakeProbe;

    use super::*;
    use crate::ludusavi::backup::{test_lock, InFlight};
    use crate::ludusavi::fake::{out, Fake};

    const MS: Duration = Duration::from_millis(1);

    /// Polls `f` for up to 5 s (the scheduler runs on its own thread).
    fn eventually(f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if f() {
                return true;
            }
            std::thread::sleep(5 * MS);
        }
        false
    }

    /// `finish` on a helper thread, so a cancel that never lands fails the test instead of hanging the run.
    fn finish_within(p: PeriodicBackups, limit: Duration) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            p.finish();
            let _ = tx.send(());
        });
        rx.recv_timeout(limit).is_ok()
    }

    #[test]
    fn runs_every_interval_until_finished() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let p = PeriodicBackups::start("t", 20 * MS, move || {
            r.fetch_add(1, Ordering::SeqCst);
            Some(20 * MS)
        })
        .unwrap();
        assert!(eventually(|| runs.load(Ordering::SeqCst) >= 3), "runs again after each interval");
        assert!(finish_within(p, Duration::from_secs(5)), "finish ends the worker");
        let after = runs.load(Ordering::SeqCst);
        std::thread::sleep(100 * MS);
        assert_eq!(runs.load(Ordering::SeqCst), after, "none after the session ended");
    }

    #[test]
    fn nothing_runs_before_the_first_interval_and_a_cancel_ends_the_wait() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let p = PeriodicBackups::start("t", Duration::from_secs(3600), move || {
            r.fetch_add(1, Ordering::SeqCst);
            Some(MS)
        })
        .unwrap();
        std::thread::sleep(50 * MS);
        assert!(finish_within(p, Duration::from_secs(2)), "finish does not wait out the interval");
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dropping_cancels_too() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let p = PeriodicBackups::start("t", 50 * MS, move || {
            r.fetch_add(1, Ordering::SeqCst);
            Some(50 * MS)
        })
        .unwrap();
        drop(p);
        std::thread::sleep(200 * MS);
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn finish_waits_for_the_running_backup_and_starts_no_other() {
        let (started, done, runs) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        let (s, d, r) = (started.clone(), done.clone(), runs.clone());
        let p = PeriodicBackups::start("t", MS, move || {
            r.fetch_add(1, Ordering::SeqCst);
            s.store(true, Ordering::SeqCst);
            std::thread::sleep(200 * MS);
            d.store(true, Ordering::SeqCst);
            Some(MS)
        })
        .unwrap();
        assert!(eventually(|| started.load(Ordering::SeqCst)));
        assert!(finish_within(p, Duration::from_secs(5)), "finish ends the worker");
        assert!(done.load(Ordering::SeqCst), "finish returned while the backup was still copying");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn one_backup_at_a_time_even_when_one_outlasts_the_interval() {
        let (now, most, runs) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (n, m, r) = (now.clone(), most.clone(), runs.clone());
        let p = PeriodicBackups::start("t", MS, move || {
            m.fetch_max(n.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            std::thread::sleep(30 * MS);
            n.fetch_sub(1, Ordering::SeqCst);
            r.fetch_add(1, Ordering::SeqCst);
            Some(MS)
        })
        .unwrap();
        assert!(eventually(|| runs.load(Ordering::SeqCst) >= 3));
        assert!(finish_within(p, Duration::from_secs(5)), "finish ends the worker");
        assert_eq!(most.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_job_saying_none_stops_them() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let p = PeriodicBackups::start("t", MS, move || {
            r.fetch_add(1, Ordering::SeqCst);
            None
        })
        .unwrap();
        assert!(eventually(|| p.thread.as_ref().is_some_and(|t| t.is_finished())));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    fn setup(settings_json: &str) -> (tempfile::TempDir, DataPaths) {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), settings_json).unwrap();
        (d, paths)
    }

    fn game() -> LudusaviGame {
        LudusaviGame { app_name: "g".into(), title: "G".into(), runner: "sideload".into(), ..Default::default() }
    }

    const EVERY_30: &str =
        r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"backupPath":"/backups","format":"zip","intervalMinutes":30}}"#;

    #[test]
    fn periodic_backup_runs_ludusavi_and_reads_the_next_interval() {
        let _l = test_lock();
        let (_d, paths) = setup(EVERY_30);
        let runner =
            Fake { replies: vec![("backup".into(), out(r#"{"overall":{"totalGames":1},"games":{"G":{}}}"#, "", 0))], ..Default::default() };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let t = periodic_backup(&env, &game(), Duration::from_secs(60));
        assert!(t.result.as_ref().is_some_and(|r| r.ran), "{t:?}");
        assert_eq!(t.next, Some(Duration::from_secs(30 * 60)));
        assert!(runner.calls.lock().unwrap().iter().any(|c| c[0] == "backup"));
    }

    #[test]
    fn periodic_backup_turned_off_mid_session_stops_without_running() {
        let _l = test_lock();
        let runner = Fake::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        for json in [
            r#"{"ludusavi":{"enabled":true,"backupPath":"/b"}}"#,
            r#"{"ludusavi":{"enabled":false,"backupPath":"/b","intervalMinutes":30}}"#,
            r#"{"ludusavi":{"enabled":true,"backupPath":"/b","intervalMinutes":30,"autoBackupOff":["sideload:g"]}}"#,
        ] {
            let (_d, paths) = setup(json);
            let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
            assert_eq!(periodic_backup(&env, &game(), Duration::from_secs(60)), PeriodicTick::default(), "{json}");
        }
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn periodic_backup_skips_while_another_backup_runs_and_keeps_its_schedule() {
        let _l = test_lock();
        let (_d, paths) = setup(EVERY_30);
        let runner = Fake::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let _held = InFlight::acquire().unwrap();
        let t = periodic_backup(&env, &game(), Duration::from_secs(60));
        let r = t.result.unwrap();
        assert_eq!(r.skipped_reason.as_deref(), Some("already-running"));
        assert!(!r.ran && r.error.is_none(), "a skip is neither a success nor a failure");
        assert_eq!(t.next, Some(Duration::from_secs(30 * 60)));
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_periodic_backup_is_an_error_and_the_schedule_goes_on() {
        let _l = test_lock();
        let (_d, paths) = setup(EVERY_30);
        let runner = Fake::default();
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &FakeProbe::new("/h") };
        let t = periodic_backup(&env, &game(), Duration::from_secs(60));
        assert_eq!(t.result.and_then(|r| r.error).as_deref(), Some("Ludusavi was not found"));
        assert_eq!(t.next, Some(Duration::from_secs(30 * 60)));
    }
}
