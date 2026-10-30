//! Emulation hooks of the Collection: the "Add emulated games" dialog, the
//! pre-launch flow of emulated games (pick a disc, pick a save state) and the
//! idle jobs (ROM auto-scan, scheduled config backup). What happens after a
//! session (save-state key, Ludusavi backup) is `services::session`.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_emulation::{list_emulator_save_states, set_emulator_launch_rom, set_pending_launch_state, HostEnv, PendingLaunchStates};
use heronite_emulation::{run_rom_scanners, AutoScanReport};

use super::CollectionView;
use crate::dialogs::launch_state::{LaunchStateDialog, StateRow, ThumbRaw};
use crate::dialogs::pick_rom::PickRomDialog;
use crate::dialogs::scan_roms::{ScanInitial, ScanRomsDialog};
use crate::services::steam_library::SteamLibraryEvent;
use heronite_i18n::{tr, trn};

/// How long after start the Collection scans the saved ROM folders.
const STARTUP_SCAN_DELAY: Duration = Duration::from_secs(4);

/// Brings `is_installed` of the local games (Steam, executables, ROMs) in
/// line with the disk. Blocking. Returns whether something changed.
fn refresh_install_states(paths: &heronite_data::DataPaths) -> bool {
    match heronite_launch::install_state::refresh_local_install_states(paths, &heronite_launch::SystemProbe) {
        Ok(report) => report.changed(),
        Err(e) => {
            log::warn!("refreshing install states: {e:#}");
            false
        }
    }
}

/// Result of listing a game's save states in the background.
struct Listing {
    emulator: String,
    rom: Option<String>,
    rows: Vec<StateRow>,
}

fn list_states(paths: &heronite_data::DataPaths, app_name: &str, rom: Option<&str>) -> Result<Listing, String> {
    let list = list_emulator_save_states(paths, &HostEnv::system(), app_name, rom, true).map_err(|e| format!("{e:#}"))?;
    let rows = list
        .states
        .iter()
        .map(|s| StateRow {
            path: s.path.clone(),
            resume: matches!(s.kind, heronite_emulation::save_states::SaveStateKind::Resume),
            slot: s.slot,
            modified_at: s.modified_at.clone(),
            thumb: s.thumbnail.as_ref().and_then(|t| ThumbRaw::decode(&t.bytes)),
        })
        .collect();
    Ok(Listing { emulator: list.emulator_name.unwrap_or_default(), rom: list.rom_path.or(rom.map(str::to_string)), rows })
}

impl CollectionView {
    pub(super) fn open_scan_roms(&mut self, initial: ScanInitial, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let view = cx.new(|cx| ScanRomsDialog::new(ctx, initial, window, cx));
        self.open_dialog(view, cx);
    }

    fn close_dialog(&mut self) {
        self.dialog = None;
        self.dialog_sub = None;
    }

    /// Emulated games get the Collection pre-launch flow. Returns `true` when
    /// the click was handled here.
    pub(super) fn start_emulated(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> bool {
        if game.launch_kind.as_deref() != Some("emulator") {
            return false;
        }
        let roms: Vec<(String, String)> = game
            .meta
            .as_ref()
            .and_then(|m| m.roms.as_ref())
            .map(|r| r.iter().map(|r| (r.name.clone().unwrap_or_default(), r.path.clone())).collect())
            .unwrap_or_default();
        if roms.len() > 1 {
            let selected = game.meta.as_ref().and_then(|m| m.selected_rom_path.clone());
            self.open_pick_rom(game, roms, selected, cx);
        } else {
            self.emu_continue(game.key.clone(), None, cx);
        }
        true
    }

    fn open_pick_rom(&mut self, game: &CollectionGame, roms: Vec<(String, String)>, selected: Option<String>, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let key = game.key.clone();
        let on_play: Rc<dyn Fn(String, &mut App)> = Rc::new(move |rom, cx| {
            let key = key.clone();
            weak.update(cx, |this, cx| {
                this.close_dialog();
                this.emu_continue(key, Some(rom), cx);
                cx.notify();
            })
            .ok();
        });
        let view = cx.new(|_| PickRomDialog::new(game.title.clone(), roms, selected, on_play));
        self.open_dialog(view, cx);
    }

    /// After the ROM is known: remember it, offer save states if there are any,
    /// otherwise launch.
    fn emu_continue(&mut self, key: String, rom: Option<String>, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(&key)).cloned() else { return };
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let (app, r) = (game.app_name.clone(), rom.clone());
            let listing = cx
                .background_spawn(async move {
                    if let Some(rom) = &r {
                        if let Err(e) = set_emulator_launch_rom(&paths, &app, rom) {
                            log::warn!("remembering ROM: {e:#}");
                        }
                    }
                    let listing = list_states(&paths, &app, r.as_deref());
                    if listing.as_ref().map(|l| l.rows.is_empty()).unwrap_or(true) {
                        // No states: clear any stale choice (and apply "fresh" arguments).
                        let _ =
                            set_pending_launch_state(&paths, &HostEnv::system(), PendingLaunchStates::global(), &app, r.as_deref(), None);
                    }
                    listing
                })
                .await;
            this.update(cx, |this, cx| {
                match listing {
                    Ok(l) if !l.rows.is_empty() => this.open_launch_state(&game, l, cx),
                    Ok(_) | Err(_) => this.launch_emulated(&game.key, rom, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_launch_state(&mut self, game: &CollectionGame, listing: Listing, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let (key, rom) = (game.key.clone(), listing.rom.clone());
        let on_play: Rc<dyn Fn(Option<String>, &mut App)> = Rc::new(move |state, cx| {
            let (key, rom) = (key.clone(), rom.clone());
            weak.update(cx, |this, cx| {
                this.close_dialog();
                this.emu_play_state(key, rom, state, cx);
                cx.notify();
            })
            .ok();
        });
        let view = cx.new(|_| LaunchStateDialog::new(game.title.clone(), listing.emulator, listing.rows, on_play));
        self.open_dialog(view, cx);
    }

    /// Sets the one-shot launch state, then launches.
    fn emu_play_state(&mut self, key: String, rom: Option<String>, state: Option<String>, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(&key)).cloned() else { return };
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let (app, r) = (game.app_name.clone(), rom.clone());
            let res = cx
                .background_spawn(async move {
                    set_pending_launch_state(
                        &paths,
                        &HostEnv::system(),
                        PendingLaunchStates::global(),
                        &app,
                        r.as_deref(),
                        state.as_deref(),
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(true) => this.launch_emulated(&game.key, rom, cx),
                    // The state vanished between listing and launch: show what is left.
                    Ok(false) => this.emu_continue(game.key.clone(), rom, cx),
                    Err(e) => this.show_toast(tr!("Could not set the save state: {err}", err = format!("{e:#}")), true, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn launch_emulated(&mut self, key: &str, rom: Option<String>, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        // The ROM played is kept by the runtime for the post-game work.
        if let Err(err) = self.runtime.play_emulated(&game, rom) {
            self.show_toast(err, true, cx);
        }
    }

    /// The refresh button: updates the install states (a game uninstalled
    /// from Steam), scans the saved ROM folders (`autoScanOnRefresh` ones)
    /// off the UI thread, then reloads the library; the spinner runs
    /// for both. The Steam library and the other launchers turned on are
    /// read again too. Like Heronite's `handleRefreshAll`: new games are added, none
    /// are ever removed.
    pub(super) fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    refresh_install_states(&paths);
                    run_rom_scanners(&paths, None)
                })
                .await;
            this.update(cx, |this, cx| {
                crate::services::steam_library(cx).update(cx, |s, cx| s.sync_if_ready(cx));
                crate::services::launcher_imports(cx).update(cx, |s, cx| s.import_enabled(cx));
                // `reload` skips when a reload is in flight.
                this.refreshing = false;
                this.after_rom_scan(res, false, cx);
                this.reload(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Toast for the games a ROM scan added. `reload` = reload the library
    /// when something was added (the refresh button reloads anyway).
    fn after_rom_scan(&mut self, res: anyhow::Result<AutoScanReport>, reload: bool, cx: &mut Context<Self>) {
        match res {
            Ok(report) => {
                for e in &report.errors {
                    log::warn!("ROM scan: {e}");
                }
                if report.imported > 0 {
                    self.show_toast(trn!(report.imported, "Added {n} new game", "Added {n} new games"), false, cx);
                    if reload {
                        self.reload(cx);
                    }
                }
            }
            Err(e) => log::warn!("ROM scan failed: {e:#}"),
        }
    }

    /// The install states of local games right away (a game uninstalled
    /// from Steam while the app was closed), the ROM auto-scan and the
    /// scheduled config backup 90 s after start (the idle jobs), plus a ROM
    /// scan a few seconds after the Collection opens so new games show up
    /// without pressing refresh.
    /// `HERONITE_DISABLE_IDLE_JOBS=1` turns both off (tests against real data).
    pub(super) fn start_idle_jobs(&self, cx: &mut Context<Self>) {
        if crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            return;
        }
        let steam = crate::services::steam_library(cx);
        cx.subscribe(&steam, |this, _, ev: &SteamLibraryEvent, cx| {
            let SteamLibraryEvent::Imported(report) = ev;
            if report.added > 0 {
                this.show_toast(
                    trn!(report.added, "Added {n} game from your Steam library", "Added {n} games from your Steam library"),
                    false,
                    cx,
                );
            }
            if report.added > 0 || report.playtime > 0 {
                this.reload(cx);
            }
        })
        .detach();
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let p = paths.clone();
            if cx.background_spawn(async move { refresh_install_states(&p) }).await {
                this.update(cx, |this, cx| this.reload(cx)).ok();
            }
            cx.background_executor().timer(STARTUP_SCAN_DELAY).await;
            let res = cx.background_spawn(async move { run_rom_scanners(&paths, None) }).await;
            this.update(cx, |this, cx| this.after_rom_scan(res, true, cx)).ok();
        })
        .detach();
        let (tx, rx) = futures::channel::oneshot::channel();
        heronite_emulation::jobs::spawn_idle_jobs(self.data_paths.clone(), heronite_emulation::jobs::IDLE_DELAY, move |report| {
            let _ = tx.send(report);
        });
        cx.spawn(async move |this, cx| {
            let Ok(report) = rx.await else { return };
            this.update(cx, |this, cx| {
                if report.roms_imported > 0 {
                    this.show_toast(
                        trn!(
                            report.roms_imported,
                            "Added {n} new game from saved ROM folders.",
                            "Added {n} new games from saved ROM folders."
                        ),
                        false,
                        cx,
                    );
                    this.reload(cx);
                }
                if let Some(e) = report.rom_scan_error {
                    log::warn!("idle jobs: {e}");
                }
            })
            .ok();
        })
        .detach();
    }

    /// `HERONITE_DEBUG_DIALOG` values for the emulation dialogs. Returns
    /// whether `which` was one of them.
    pub(super) fn open_emu_debug_dialog(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if which == "refresh" {
            // Same as pressing the header refresh button.
            self.refresh_all(cx);
            return true;
        }
        if which == "scanroms" {
            self.open_scan_roms(ScanInitial::default(), window, cx);
            return true;
        }
        let fake = crate::env::flag("HERONITE_DEBUG_FAKE_STATES");
        if let Some(key) = which.strip_prefix("pickrom:") {
            let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return true };
            let mut roms: Vec<(String, String)> = game
                .meta
                .as_ref()
                .and_then(|m| m.roms.as_ref())
                .map(|r| r.iter().map(|r| (r.name.clone().unwrap_or_default(), r.path.clone())).collect())
                .unwrap_or_default();
            if roms.len() < 2 {
                roms = (1..=2).map(|i| (format!("{} (Disc {i})", game.title), format!("/roms/{}/disc{i}.iso", game.title))).collect();
            }
            let selected = roms.first().map(|r| r.1.clone());
            self.open_pick_rom(&game, roms, selected, cx);
            return true;
        }
        if let Some(key) = which.strip_prefix("launchstate:") {
            let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return true };
            let listing = if fake {
                let now = chrono::Utc::now();
                let rows = (0..3u32)
                    .map(|i| StateRow {
                        path: format!("/tmp/fake-{i}.p2s"),
                        resume: i == 0,
                        slot: (i > 0).then_some(i),
                        modified_at: (now - chrono::Duration::hours(i as i64 * 20 + 1)).to_rfc3339(),
                        thumb: Some(fake_thumb(i)),
                    })
                    .collect();
                Ok(Listing { emulator: "PCSX2".into(), rom: None, rows })
            } else {
                list_states(&self.data_paths, &game.app_name, None)
            };
            match listing {
                Ok(l) => self.open_launch_state(&game, l, cx),
                Err(e) => self.show_toast(e, true, cx),
            }
            return true;
        }
        false
    }
}

/// A synthetic gradient thumbnail for screenshots.
fn fake_thumb(seed: u32) -> ThumbRaw {
    let (w, h) = (160u32, 120u32);
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w) as u8;
            let g = (y * 255 / h) as u8;
            let b = (60 + seed * 70) as u8;
            bgra.extend_from_slice(&[b, g, r, 255]);
        }
    }
    ThumbRaw { width: w, height: h, bgra }
}
