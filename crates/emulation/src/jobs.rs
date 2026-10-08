//! Idle maintenance (the emulation/backup part of `scheduleIdleMaintenance`
//! in `init.ts`): 90 seconds after start Heronite auto-scans ROM folders and
//! runs the scheduled config backup.

use std::thread::JoinHandle;
use std::time::Duration;

use heronite_data::HeroicPaths;

use crate::backup::{maybe_run_scheduled_backup, CollectionBackupResult};
use crate::import::run_auto_rom_scanners;

pub const IDLE_DELAY: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Default)]
pub struct IdleReport {
    /// Games imported by the ROM auto-scan.
    pub roms_imported: usize,
    pub rom_scan_error: Option<String>,
    pub backup: CollectionBackupResult,
}

/// Runs the jobs now (blocking): `autoScanRomFolders()` then
/// `maybeRunScheduledBackup()`. The caller refreshes the library when
/// `roms_imported > 0`.
pub fn run_idle_jobs(paths: &HeroicPaths) -> IdleReport {
    let mut report = IdleReport::default();
    match run_auto_rom_scanners(paths) {
        Ok(n) => report.roms_imported = n,
        Err(e) => {
            log::error!("ROM auto-scan failed: {e:#}");
            report.rom_scan_error = Some(format!("{e:#}"));
        }
    }
    report.backup = maybe_run_scheduled_backup(paths);
    report
}

/// Runs the jobs on a background thread after `delay` (use [`IDLE_DELAY`]).
pub fn spawn_idle_jobs(
    paths: HeroicPaths,
    delay: Duration,
    on_done: impl FnOnce(IdleReport) + Send + 'static,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        on_done(run_idle_jobs(&paths));
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{UpsertRomScannerArgs, UpsertUserEmulatorArgs};

    #[test]
    fn idle_jobs_scan_and_report_backup_state() {
        let heroic = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(heroic.path());
        let roms = tempfile::tempdir().unwrap();
        std::fs::write(roms.path().join("A.iso"), "x").unwrap();
        let e = crate::store::upsert_user_emulator(
            &paths,
            UpsertUserEmulatorArgs {
                definition_id: "pcsx2".into(),
                name: "PCSX2".into(),
                executable: "/opt/pcsx2".into(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        crate::store::upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                emulator_id: e.id.clone(),
                profile_id: e.profiles[0].id.clone(),
                directory: roms.path().to_str().unwrap().into(),
                ..Default::default()
            },
        )
        .unwrap();
        let report = run_idle_jobs(&paths);
        assert_eq!(report.roms_imported, 1);
        assert!(report.rom_scan_error.is_none());
        // No backup folder configured.
        assert_eq!(report.backup.skipped_reason.as_deref(), Some("disabled"));

        let (tx, rx) = std::sync::mpsc::channel();
        let h = spawn_idle_jobs(paths, Duration::from_millis(10), move |r| tx.send(r.roms_imported).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        h.join().unwrap();
    }
}
