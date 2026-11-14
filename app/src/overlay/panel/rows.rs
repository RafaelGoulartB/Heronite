//! The rows of each tab and what they do.

use gpui_kit::*;
use heronite_data::status::{self, SetStatusExtras};
use heronite_i18n::tr;
use heronite_launch::live_fps::LiveFps;
use heronite_overlay::audio;

use super::{mixer, Confirm, OverlayPanel, Tab};
use crate::console::list::{Outcome, Row, Value};
use crate::icons::Icon;
use crate::overlay::{LiveChange, OverlayService};

/// Home's "Show MangoHud" row and the FPS limit after it.
const MANGOHUD: &str = "mangohud";
const FPS: &str = "fps";
/// Home's "Capture" row: one round button per [`Capture`], ◀ ▶ choose one
/// and A runs it (`OverlayPanel::render_capture` draws it).
pub(super) const CAPTURES: &str = "captures";

/// What Home's "Capture" row can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Capture {
    Shot,
    Replay,
    Moment,
}

impl Capture {
    /// The row that does the same on its own (see `OverlayPanel::activate`).
    fn id(self) -> &'static str {
        match self {
            Capture::Shot => "capture",
            Capture::Replay => "replay",
            Capture::Moment => "moment",
        }
    }

    pub(super) fn icon(self) -> Icon {
        match self {
            Capture::Shot => Icon::PhotoCamera,
            Capture::Replay => Icon::Videocam,
            Capture::Moment => Icon::Flag,
        }
    }

    pub(super) fn label(self) -> String {
        match self {
            Capture::Shot => tr!("Take screenshot").into(),
            Capture::Replay => tr!("Save replay").into(),
            Capture::Moment => tr!("Mark this moment").into(),
        }
    }
}

impl OverlayPanel {
    pub(super) fn rows(&self) -> Vec<Row> {
        match self.tab() {
            Tab::Home => self.home_rows(),
            Tab::Shots => {
                let mut rows = vec![Row::action("capture", tr!("Take screenshot")).icon(Icon::PhotoCamera)];
                rows.extend(self.replay_row());
                rows.extend(self.replay_switch());
                rows
            }
            Tab::Notes => self.notes_rows(),
            Tab::Audio => self.audio_rows(),
            Tab::Controller => self.controller_rows(),
            Tab::Achievements => self.achievement_rows(),
        }
    }

    fn home_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let has_game = self.game.is_some();
        match &self.data.audio {
            Some(Some(a)) => rows.extend(mixer::rows(a, has_game, mixer::HOME_APPS)),
            // Still reading it: the mixer's rows already hold their place.
            None => rows.extend(mixer::rows(&audio::AudioState::default(), has_game, 0)),
            Some(None) => {}
        }
        let Some(game) = &self.game else {
            rows.push(Row::action("launcher", tr!("Open Heronite")).icon(Icon::Home));
            return rows;
        };
        match self.chosen_capture() {
            // Alone, the screenshot is a row like any other.
            (_, all) if all.len() < 2 => rows.push(Row::action("capture", tr!("Take screenshot")).icon(Icon::PhotoCamera)),
            (at, all) => {
                let desc = match (all[at], self.replay) {
                    (Capture::Replay, Some(replay)) => tr!("Save replay · the last {n} s", n = replay.seconds).to_string(),
                    (chosen, _) => chosen.label(),
                };
                rows.push(Row::action(CAPTURES, tr!("Capture")).icon(Icon::PhotoCamera).desc(desc));
            }
        }
        // When its limiter or MangoHud's HUD can change while it runs.
        if let Some(now) = &game.fps {
            let steps = fps_steps(now.limit);
            let options = steps.iter().map(|n| if *n == 0 { tr!("Off").into() } else { SharedString::from(format!("{n} FPS")) }).collect();
            let index = steps.iter().position(|n| *n == now.limit).unwrap_or(0);
            let desc = match now.hud {
                // Hidden, MangoHud still limits the game.
                Some(true) => tr!("Changes now"),
                Some(false) => tr!("Still applies while hidden"),
                None => tr!("Changes now (gamescope)"),
            };
            if let Some(hud) = now.hud {
                rows.push(hud_row(hud, true));
            }
            // DXVK / VKD3D-Proton hold it: only MangoHud's HUD changes.
            if now.live.changes_limit() {
                rows.push(Row::choice(FPS, tr!("FPS limit"), options, index).icon(Icon::Tune).desc(desc).compact());
            }
        } else if let Some(hud) = &self.data.watched_hud {
            rows.push(hud_row(hud.shown, false));
        }
        if let Some(id) = &self.data.status_id {
            let options: Vec<SharedString> = self.data.statuses.iter().map(|s| status::status_label(&s.name).to_string().into()).collect();
            let index = self.data.statuses.iter().position(|s| &s.id == id).unwrap_or(0);
            rows.push(Row::choice("status", tr!("Status"), options, index).icon(Icon::Flag));
        }
        rows.extend(self.achievements_home_row());
        // Only while another window has the focus (after "Back to Heronite", say).
        if self.data.game_away {
            rows.push(Row::action("game", tr!("Back to game")).icon(Icon::SportsEsports).desc(tr!("Brings its window to the front")));
        }
        rows.push(Row::action("launcher", tr!("Back to Heronite")).icon(Icon::Home).desc(tr!("The game keeps running")));
        rows.push(Row::action("quit", tr!("Quit game")).icon(Icon::PowerSettingsNew).danger());
        rows
    }

    /// What Home's "Capture" row offers now, and the one chosen in it.
    pub(super) fn chosen_capture(&self) -> (usize, Vec<Capture>) {
        let mut all = vec![Capture::Shot];
        if self.replay.is_some_and(|r| r.on) {
            all.push(Capture::Replay);
        }
        if self.data.journal.is_some() {
            all.push(Capture::Moment);
        }
        (self.capture.min(all.len() - 1), all)
    }

    /// A press on Home's "Capture" row: ◀ ▶ choose, A runs the chosen one.
    pub(super) fn capture_pad(&mut self, pad: crate::console::Pad, cx: &mut Context<Self>) -> bool {
        use crate::console::Pad;
        let (at, all) = self.chosen_capture();
        match pad {
            Pad::Left => self.capture = at.saturating_sub(1),
            Pad::Right => self.capture = (at + 1).min(all.len() - 1),
            Pad::A => self.activate(all[at].id(), cx),
            _ => return false,
        }
        true
    }

    /// "Save replay", while the replay records.
    fn replay_row(&self) -> Option<Row> {
        let replay = self.replay.filter(|r| r.on)?;
        Some(
            Row::action("replay", tr!("Save replay"))
                .icon(Icon::Videocam)
                .desc(tr!("The last {n} s of the game, marked as a moment.", n = replay.seconds)),
        )
    }

    /// Turns the replay off until the game ends (its memory comes back), or
    /// back on.
    fn replay_switch(&self) -> Option<Row> {
        let replay = self.replay?;
        let desc = if replay.on {
            tr!("Off: its memory comes back until the game ends.")
        } else {
            tr!("Off until the game ends. On: it keeps the seconds from then.")
        };
        Some(Row::toggle("replay-on", tr!("Keep the replay"), replay.on).icon(Icon::Videocam).desc(desc))
    }

    fn audio_rows(&self) -> Vec<Row> {
        let a = match &self.data.audio {
            None => return mixer::rows(&audio::AudioState::default(), self.game.is_some(), 0),
            Some(None) => {
                let why = if cfg!(windows) {
                    tr!("Windows audio devices are not available.")
                } else {
                    tr!("It needs pactl (PipeWire or PulseAudio).")
                };
                return vec![Row::info("none", tr!("Volume control is not available"), "").desc(why).enabled(false)];
            }
            Some(Some(a)) => a,
        };
        let mut rows = mixer::rows(a, self.game.is_some(), mixer::AUDIO_APPS);
        if !a.outputs.is_empty() {
            let options: Vec<SharedString> = a.outputs.iter().map(|o| mixer::short_row(&o.description).into()).collect();
            rows.push(Row::choice("output", tr!("Output"), options, a.default_output.unwrap_or(0)).icon(Icon::Headphones));
        }
        if !a.inputs.is_empty() {
            let options: Vec<SharedString> = a.inputs.iter().map(|o| mixer::short_row(&o.description).into()).collect();
            rows.push(Row::choice("input", tr!("Microphone"), options, a.default_input.unwrap_or(0)).icon(Icon::Mic));
        }
        if let Some(muted) = a.mic_muted {
            rows.push(Row::toggle("mic", tr!("Microphone on"), !muted).icon(if muted { Icon::MicOff } else { Icon::Mic }));
        }
        rows
    }

    fn controller_rows(&self) -> Vec<Row> {
        if self.data.controllers.is_empty() {
            return vec![Row::info("none", tr!("No controller found"), "").enabled(false)];
        }
        let mut rows = Vec::new();
        for (i, (c, battery)) in self.data.controllers.iter().enumerate() {
            rows.push(Row::header(c.name.clone()));
            let level = match battery {
                Some(b) if b.charging => tr!("{percent}% · charging", percent = b.percent),
                Some(b) => format!("{}%", b.percent),
                None => tr!("Not reported").to_string(),
            };
            rows.push(Row::info(SharedString::from(format!("battery-{i}")), tr!("Battery"), level).icon(Icon::BatteryFull));
            let link = if cfg!(windows) {
                tr!("Not reported by XInput")
            } else if c.bluetooth.is_some() {
                tr!("Bluetooth")
            } else {
                tr!("Wired or dongle")
            }
            .to_string();
            rows.push(Row::info(SharedString::from(format!("link-{i}")), tr!("Connection"), link).icon(Icon::SportsEsports));
            if c.bluetooth.is_some() {
                rows.push(
                    Row::action(SharedString::from(format!("disconnect-{i}")), tr!("Turn off controller"))
                        .icon(Icon::Bluetooth)
                        .desc(tr!("Disconnects it to save its battery")),
                );
            }
        }
        rows
    }

    pub(super) fn apply(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        match outcome {
            Outcome::Activate(id) => self.activate(&id, cx),
            Outcome::Changed(id, value) => self.changed(&id, value, cx),
            Outcome::Edge(crate::console::Pad::Down) if self.tab() == Tab::Shots && !self.data.shots.is_empty() => {
                self.grid = Some(0);
            }
            Outcome::Ignored | Outcome::Moved | Outcome::Edge(_) => {}
        }
    }

    fn activate(&mut self, id: &str, cx: &mut Context<Self>) {
        self.error = None;
        match id {
            "capture" => self.with_service(cx, |s, cx| s.screenshot(cx)),
            "replay" => self.with_service(cx, |s, cx| s.save_replay(cx)),
            "launcher" => self.with_service(cx, |s, cx| s.show_launcher(cx)),
            "game" => self.back_to_game(cx),
            "quit" => self.ask(Confirm::Quit),
            id if self.notes_activate(id, cx) => {}
            id if self.achievements_activate(id, cx) => {}
            _ => {
                let Some(i) = id.strip_prefix("disconnect-").and_then(|n| n.parse::<usize>().ok()) else { return };
                if let Some((c, _)) = self.data.controllers.get(i) {
                    if let Some(address) = c.bluetooth.clone() {
                        self.ask(Confirm::Disconnect(address, c.name.clone()));
                    }
                }
            }
        }
    }

    fn changed(&mut self, id: &str, value: Value, cx: &mut Context<Self>) {
        self.error = None;
        let pid = self.game.as_ref().and_then(|g| g.game.pid);
        match (id, value) {
            ("replay-on", Value::Toggle(on)) => {
                OverlayService::set_replay_on(on, cx);
                if let Some(r) = self.replay.as_mut() {
                    r.on = on;
                }
            }
            (mixer::SYSTEM, Value::Number { value, .. }) => {
                let step = self.volume_step;
                // ◀ ▶ move by the user's step, not the row's.
                let v = self.set_audio(|a| {
                    let from = i64::from(a.volume);
                    let to = if value > from { from + step } else { from - step };
                    a.volume = to.clamp(0, i64::from(a.volume.max(100))) as u16;
                    a.muted = false;
                    a.volume
                });
                if let Some(v) = v {
                    self.run_audio(move || audio::set_volume(v), cx);
                }
            }
            (mixer::GAME, Value::Number { value, .. }) => {
                let Some(pid) = pid else { return };
                let step = self.volume_step;
                let v = self.set_audio(|a| {
                    let from = i64::from(a.game_volume.unwrap_or(100));
                    let to = if value > from { from + step } else { from - step };
                    let to = to.clamp(0, from.max(100)) as u16;
                    a.game_volume = Some(to);
                    a.game_muted = false;
                    to
                });
                if let Some(v) = v {
                    self.run_audio(move || audio::set_game_volume(pid, v), cx);
                }
            }
            (id, Value::Number { value, .. }) if mixer::app_index(id).is_some() => {
                let Some(i) = mixer::app_index(id) else { return };
                let step = self.volume_step;
                let change = self.set_audio(|a| {
                    a.apps.get_mut(i).map(|app| {
                        let from = i64::from(app.volume);
                        let to = if value > from { from + step } else { from - step };
                        app.volume = to.clamp(0, from.max(100)) as u16;
                        app.muted = false;
                        (app.streams.clone(), app.volume)
                    })
                });
                if let Some(Some((streams, v))) = change {
                    self.run_audio(move || audio::set_streams_volume(&streams, v), cx);
                }
            }
            ("output", Value::Choice { index, .. }) => {
                let name = self.set_audio(|a| {
                    a.default_output = Some(index);
                    a.outputs.get(index).map(|o| o.name.clone())
                });
                if let Some(Some(name)) = name {
                    self.run_audio(move || audio::set_default_output(&name), cx);
                    // The new output's volume.
                    self.load_audio(cx);
                }
            }
            ("input", Value::Choice { index, .. }) => {
                let name = self.set_audio(|a| {
                    a.default_input = Some(index);
                    a.inputs.get(index).map(|o| o.name.clone())
                });
                if let Some(Some(name)) = name {
                    self.run_audio(move || audio::set_default_input(&name), cx);
                    // The new microphone's mute.
                    self.load_audio(cx);
                }
            }
            ("mic", Value::Toggle(on)) => {
                self.set_audio(|a| a.mic_muted = Some(!on));
                self.run_audio(move || audio::set_mic_muted(!on), cx);
            }
            ("status", Value::Choice { index, .. }) => self.set_status(index, cx),
            (FPS, Value::Choice { index, .. }) => {
                let Some(now) = self.game.as_ref().and_then(|g| g.fps.as_ref()) else { return };
                if let Some(limit) = fps_steps(now.limit).get(index).copied() {
                    let hud = now.hud;
                    self.set_live(LiveChange { limit, hud }, cx);
                }
            }
            (MANGOHUD, Value::Toggle(on)) => match self.game.as_ref().and_then(|g| g.fps.as_ref()) {
                Some(now) => {
                    if now.hud.is_some() {
                        let limit = now.limit;
                        self.set_live(LiveChange { limit, hud: Some(on) }, cx);
                    }
                }
                None => self.set_watched_hud(on, cx),
            },
            _ => {}
        }
    }

    /// Changes the shown audio state at once (the command follows).
    pub(super) fn set_audio<T>(&mut self, f: impl FnOnce(&mut audio::AudioState) -> T) -> Option<T> {
        match &mut self.data.audio {
            Some(Some(a)) => Some(f(a)),
            _ => None,
        }
    }

    pub(super) fn run_audio(&self, f: impl FnOnce() + Send + 'static, cx: &mut Context<Self>) {
        // The debug hooks drive the panel with scripted presses: they must not
        // change the machine's real volume.
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: audio change skipped");
            return;
        }
        cx.background_spawn(async move { f() }).detach();
    }

    /// Shows or hides MangoHud's HUD through the user's own config file (a
    /// Steam game's, say: see `heronite_overlay::mangohud`).
    fn set_watched_hud(&mut self, shown: bool, cx: &mut Context<Self>) {
        let Some(hud) = &mut self.data.watched_hud else { return };
        let Some(app_name) = self.game.as_ref().map(|g| g.game.app_name.clone()) else { return };
        let before = hud.shown;
        hud.shown = shown;
        let hud = hud.clone();
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: MangoHud change skipped");
            return;
        }
        let record = crate::overlay::mangohud_record(&self.paths);
        let files = hud.files.clone();
        self.with_service(cx, move |s, _| s.hud_edited(&app_name, files));
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_overlay::mangohud::set_shown(&hud, shown, &record) }).await;
            if let Err(e) = res {
                log::warn!("overlay: MangoHud HUD {shown}: {e}");
                this.update(cx, |this, cx| {
                    if let Some(h) = &mut this.data.watched_hud {
                        h.shown = before;
                    }
                    this.error = Some(tr!("Could not show or hide MangoHud: {err}", err = e).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Applies a new FPS limit (`0` = none) or shows / hides MangoHud's HUD
    /// in the running game.
    fn set_live(&mut self, change: LiveChange, cx: &mut Context<Self>) {
        let Some(game) = &mut self.game else { return };
        let Some(now) = &mut game.fps else { return };
        let hud_changed = change.hud != now.hud;
        (now.limit, now.hud) = (change.limit, change.hud);
        let live = now.live.clone();
        let (app_name, pid) = (game.game.app_name.clone(), game.game.pid);
        self.with_service(cx, move |s, _| s.live_changed(&app_name, change));
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: FPS limit / HUD change skipped");
            return;
        }
        let LiveChange { limit, hud } = change;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match &live {
                        LiveFps::MangoHud { .. } => heronite_launch::live_fps::set_mangohud(&live, limit, hud.unwrap_or(true)),
                        LiveFps::Gamescope { .. } => match pid {
                            Some(pid) => heronite_overlay::gamescope::set_fps_limit(pid, limit),
                            None => Err("the game's process is not known yet".into()),
                        },
                    }
                })
                .await;
            if let Err(e) = res {
                log::warn!("overlay: FPS limit {limit}, HUD {hud:?}: {e}");
                this.update(cx, |this, cx| {
                    let error = if hud_changed {
                        tr!("Could not show or hide MangoHud: {err}", err = e)
                    } else {
                        tr!("Could not change the FPS limit: {err}", err = e)
                    };
                    this.error = Some(error.into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Gives the focus back to the game's window and closes the panel.
    fn back_to_game(&mut self, cx: &mut Context<Self>) {
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: back to the game skipped");
            return;
        }
        let Some(pid) = self.game.as_ref().and_then(|g| g.game.pid) else { return };
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_overlay::game_window::focus(pid) }).await;
            this.update(cx, |this, cx| match res {
                Ok(()) => this.close(cx),
                Err(e) => {
                    log::warn!("overlay: back to the game: {e}");
                    this.error = Some(tr!("Could not go back to the game: {err}", err = e).into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn set_status(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(status) = self.data.statuses.get(index).cloned() else { return };
        let Some(game) = &self.game else { return };
        let app_name = game.game.app_name.clone();
        let extras = SetStatusExtras { runner: game.info.as_ref().map(|i| i.runner.clone()), title: Some(game.title()), playnite_id: None };
        self.data.status_id = Some(status.id.clone());
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: status change skipped");
            return;
        }
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { status::set_game_status(&paths, &app_name, &status.id, &extras) }).await;
            this.update(cx, |this, cx| match res {
                Ok(_) => this.with_service(cx, |s, cx| s.library_changed(cx)),
                Err(e) => {
                    this.error = Some(tr!("Could not change status: {err}", err = format!("{e:#}")).into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn disconnect(&mut self, address: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { heronite_overlay::controller::disconnect(&address) }).await;
            if let Err(e) = res {
                this.update(cx, |this, cx| {
                    this.error = Some(tr!("Could not turn it off: {err}", err = e).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

/// The limits offered (`0` = none), with `current` among them.
fn fps_steps(current: u32) -> Vec<u32> {
    let mut steps = vec![0, 30, 40, 45, 60, 72, 90, 120, 144];
    if !steps.contains(&current) {
        steps.push(current);
        steps.sort_unstable();
    }
    steps
}

/// Home's "Show MangoHud" row. `limits`: MangoHud (or DXVK) still limits the
/// game while it is hidden, as the launch set it up.
fn hud_row(shown: bool, limits: bool) -> Row {
    let (ic, line) = match (shown, limits) {
        (true, _) => (Icon::Visibility, tr!("Shown over the game")),
        (false, true) => (Icon::VisibilityOff, tr!("Hidden. The FPS limit still applies.")),
        (false, false) => (Icon::VisibilityOff, tr!("Hidden until you show it again or the game ends")),
    };
    Row::toggle(MANGOHUD, tr!("Show MangoHud"), shown).icon(ic).desc(line)
}
