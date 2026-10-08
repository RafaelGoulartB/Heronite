//! Emulation hooks of the Collection: the "Add emulated games" dialog, the
//! pre-launch flow of emulated games (pick a disc, pick a save state), what
//! happens after a session (save-state key, Ludusavi backup) and the idle jobs
//! (ROM auto-scan, scheduled config backup).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_emulation::ludusavi::{notification, AfterPlay, LudusaviEnv, SystemRunner};
use heronite_emulation::{
    list_emulator_save_states, remember_save_state_key, set_emulator_launch_rom, set_pending_launch_state, HostEnv,
    PendingLaunchStates,
};
use heronite_launch::SystemProbe;

use super::CollectionView;
use crate::dialogs::launch_state::{LaunchStateDialog, StateRow, ThumbRaw};
use crate::dialogs::pick_rom::PickRomDialog;
use crate::dialogs::scan_roms::{ScanInitial, ScanRomsDialog};
use crate::dialogs::settings::{SettingsDialog, Tab};
use crate::dialogs::DialogRequest;

#[derive(Default)]
pub struct EmuState {
    /// A dialog asked the Collection to open another one.
    pub requested: Option<DialogRequest>,
    /// ROM of each running emulated session (`app_name` -> rom).
    sessions: HashMap<String, Option<String>>,
}

/// Result of listing a game's save states in the background.
struct Listing {
    emulator: String,
    rom: Option<String>,
    rows: Vec<StateRow>,
}

fn list_states(paths: &heronite_data::HeroicPaths, app_name: &str, rom: Option<&str>) -> Result<Listing, String> {
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
    /// Opens whatever a dialog asked for (needs a `Window`, so it runs from
    /// `render`).
    pub(super) fn drive_emulation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(req) = self.emu.requested.take() {
            match req {
                DialogRequest::ScanRoms(init) => self.open_scan_roms(init, window, cx),
            }
        }
    }

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

    fn open_pick_rom(
        &mut self,
        game: &CollectionGame,
        roms: Vec<(String, String)>,
        selected: Option<String>,
        cx: &mut Context<Self>,
    ) {
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
        let paths = self.heroic.clone();
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
                        let _ = set_pending_launch_state(
                            &paths,
                            &HostEnv::system(),
                            PendingLaunchStates::global(),
                            &app,
                            r.as_deref(),
                            None,
                        );
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
        let paths = self.heroic.clone();
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
                    Err(e) => this.show_toast(format!("Could not set the save state: {e:#}"), true, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn launch_emulated(&mut self, key: &str, rom: Option<String>, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        match self.runtime.play_emulated(&game, rom) {
            Ok(played) => {
                self.emu.sessions.insert(game.app_name.clone(), played);
            }
            Err(err) => self.show_toast(err, true, cx),
        }
    }

    /// After a session: learn the save-state key and run the Ludusavi backup
    /// (when enabled), both off the UI thread.
    pub(super) fn emu_session_ended(&mut self, app_name: &str, elapsed: Duration, cx: &mut Context<Self>) {
        let rom = self.emu.sessions.remove(app_name);
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.games.iter().find(|g| g.app_name == app_name).cloned() else { return };
        let is_emulator = game.launch_kind.as_deref() == Some("emulator");
        let ludusavi = lib.settings.ludusavi.enabled;
        if !is_emulator && !ludusavi {
            return;
        }
        let rom = rom.flatten().or_else(|| game.meta.as_ref().and_then(|m| m.selected_rom_path.clone()));
        let paths = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    if is_emulator {
                        let started = SystemTime::now().checked_sub(elapsed).unwrap_or_else(SystemTime::now);
                        if let Err(e) = remember_save_state_key(&paths, &HostEnv::system(), &game.app_name, rom.as_deref(), started) {
                            log::warn!("remembering save state key: {e:#}");
                        }
                    }
                    if !ludusavi {
                        return None;
                    }
                    let runner = SystemRunner::default();
                    let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &SystemProbe };
                    let result = heronite_emulation::ludusavi::backup_after_play(
                        &env,
                        &AfterPlay {
                            app_name: game.app_name.clone(),
                            title: game.title.clone(),
                            runner: game.runner.clone(),
                            elapsed_seconds: elapsed.as_secs(),
                        },
                    )?;
                    if let Some((title, body)) = notification(&result, &game.title) {
                        // Best-effort desktop notification.
                        let _ = std::process::Command::new("notify-send")
                            .args(["--app-name=Heronite", &title, &body])
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .and_then(|mut c| c.wait());
                        return Some((body, false));
                    }
                    result.error.map(|e| (format!("Ludusavi backup failed: {e}"), true))
                })
                .await;
            if let Some((text, error)) = outcome {
                this.update(cx, |this, cx| this.show_toast(text, error, cx)).ok();
            }
        })
        .detach();
    }

    /// ROM auto-scan and the scheduled config backup, 90 s after start.
    /// `HERONITE_DISABLE_IDLE_JOBS=1` turns it off (tests against real data).
    pub(super) fn start_idle_jobs(&self, cx: &mut Context<Self>) {
        if std::env::var_os("HERONITE_DISABLE_IDLE_JOBS").is_some() {
            return;
        }
        let (tx, rx) = futures::channel::oneshot::channel();
        heronite_emulation::jobs::spawn_idle_jobs(self.heroic.clone(), heronite_emulation::jobs::IDLE_DELAY, move |report| {
            let _ = tx.send(report);
        });
        cx.spawn(async move |this, cx| {
            let Ok(report) = rx.await else { return };
            this.update(cx, |this, cx| {
                if report.roms_imported > 0 {
                    this.show_toast(format!("Added {} new game(s) from saved ROM folders.", report.roms_imported), false, cx);
                    this.reload(cx);
                }
                if let Some(e) = report.rom_scan_error.or(report.backup.error) {
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
        let tab = match which {
            "settings:emulators" => Some(Tab::Emulators),
            "settings:backup" => Some(Tab::Backup),
            "settings:saves" => Some(Tab::Saves),
            _ => None,
        };
        if let Some(tab) = tab {
            self.open_settings(window, cx);
            if let Some(d) = self.dialog.clone().and_then(|v| v.downcast::<SettingsDialog>().ok()) {
                d.update(cx, |d, _| d.tab = tab);
            }
            return true;
        }
        if which == "scanroms" {
            self.open_scan_roms(ScanInitial::default(), window, cx);
            return true;
        }
        let fake = std::env::var_os("HERONITE_DEBUG_FAKE_STATES").is_some();
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
                list_states(&self.heroic, &game.app_name, None)
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
