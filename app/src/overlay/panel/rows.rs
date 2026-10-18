//! The rows of each tab and what they do.

use gpui_kit::*;
use heronite_data::status::{self, SetStatusExtras};
use heronite_i18n::tr;
use heronite_launch::live_fps::LiveFps;
use heronite_overlay::audio;

use super::{mixer, Confirm, OverlayPanel, Tab};
use crate::console::list::{Outcome, Row, Value};
use crate::icons::Icon;

impl OverlayPanel {
    pub(super) fn rows(&self) -> Vec<Row> {
        match self.tab() {
            Tab::Home => self.home_rows(),
            Tab::Shots => vec![Row::action("capture", tr!("Take screenshot")).icon(Icon::PhotoCamera)],
            Tab::Audio => self.audio_rows(),
            Tab::Controller => self.controller_rows(),
        }
    }

    fn home_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        match &self.data.audio {
            Some(Some(a)) => rows.extend(mixer::rows(a)),
            // Still reading it: the mixer's rows already hold their place.
            None => rows.extend(mixer::rows(&audio::AudioState::default())),
            Some(None) => {}
        }
        rows.push(Row::action("capture", tr!("Take screenshot")).icon(Icon::PhotoCamera));
        // Only when the game's limiter can change while it runs.
        if let Some((live, limit)) = &self.game.fps {
            let steps = fps_steps(*limit);
            let options = steps.iter().map(|n| if *n == 0 { tr!("Off").into() } else { SharedString::from(format!("{n} FPS")) }).collect();
            let index = steps.iter().position(|n| n == limit).unwrap_or(0);
            let desc = if live.is_gamescope() { tr!("Changes now (gamescope)") } else { tr!("Changes now (MangoHud)") };
            rows.push(Row::choice("fps", tr!("FPS limit"), options, index).icon(Icon::Tune).desc(desc).compact());
        }
        if let Some(id) = &self.data.status_id {
            let options: Vec<SharedString> = self.data.statuses.iter().map(|s| status::status_label(&s.name).to_string().into()).collect();
            let index = self.data.statuses.iter().position(|s| &s.id == id).unwrap_or(0);
            rows.push(Row::choice("status", tr!("Status"), options, index).icon(Icon::Flag));
        }
        rows.push(Row::action("launcher", tr!("Back to Heronite")).icon(Icon::Home).desc(tr!("The game keeps running")));
        rows.push(Row::action("quit", tr!("Quit game")).icon(Icon::PowerSettingsNew).danger());
        rows
    }

    fn audio_rows(&self) -> Vec<Row> {
        let a = match &self.data.audio {
            None => return mixer::rows(&audio::AudioState::default()),
            Some(None) => {
                return vec![Row::info("none", tr!("Volume control is not available"), "")
                    .desc(tr!("It needs pactl (PipeWire or PulseAudio)."))
                    .enabled(false)]
            }
            Some(Some(a)) => a,
        };
        let mut rows = mixer::rows(a);
        if !a.outputs.is_empty() {
            let options: Vec<SharedString> = a.outputs.iter().map(|o| mixer::short(&o.description).into()).collect();
            rows.push(Row::choice("output", tr!("Output"), options, a.default_output.unwrap_or(0)).icon(Icon::Headphones));
        }
        if let Some(muted) = a.mic_muted {
            rows.push(Row::toggle("mic", tr!("Microphone"), !muted).icon(if muted { Icon::MicOff } else { Icon::Mic }));
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
            let link = if c.bluetooth.is_some() { tr!("Bluetooth") } else { tr!("Wired or dongle") }.to_string();
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
            "launcher" => self.with_service(cx, |s, cx| s.show_launcher(cx)),
            "quit" => self.ask(Confirm::Quit),
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
        let pid = self.game.game.pid;
        match (id, value) {
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
            ("mic", Value::Toggle(on)) => {
                self.set_audio(|a| a.mic_muted = Some(!on));
                self.run_audio(move || audio::set_mic_muted(!on), cx);
            }
            ("status", Value::Choice { index, .. }) => self.set_status(index, cx),
            ("fps", Value::Choice { index, .. }) => {
                let Some((_, limit)) = &self.game.fps else { return };
                if let Some(next) = fps_steps(*limit).get(index).copied() {
                    self.set_fps_limit(next, cx);
                }
            }
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

    /// Applies a new FPS limit to the running game (`0` = none).
    fn set_fps_limit(&mut self, limit: u32, cx: &mut Context<Self>) {
        let Some((live, _)) = self.game.fps.clone() else { return };
        self.game.fps = Some((live.clone(), limit));
        let app_name = self.game.game.app_name.clone();
        self.with_service(cx, move |s, _| s.fps_limit_changed(&app_name, limit));
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: FPS limit change skipped");
            return;
        }
        let pid = self.game.game.pid;
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    match &live {
                        LiveFps::MangoHud { .. } => heronite_launch::live_fps::set_mangohud(&live, limit),
                        LiveFps::Gamescope { .. } => match pid {
                            Some(pid) => heronite_overlay::gamescope::set_fps_limit(pid, limit),
                            None => Err("the game's process is not known yet".into()),
                        },
                    }
                })
                .await;
            if let Err(e) = res {
                log::warn!("overlay: FPS limit {limit}: {e}");
                this.update(cx, |this, cx| {
                    this.error = Some(tr!("Could not change the FPS limit: {err}", err = e).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn set_status(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(status) = self.data.statuses.get(index).cloned() else { return };
        self.data.status_id = Some(status.id.clone());
        if crate::env::value("HERONITE_DEBUG_OVERLAY").is_some() {
            log::info!("overlay debug: status change skipped");
            return;
        }
        let paths = self.paths.clone();
        let app_name = self.game.game.app_name.clone();
        let extras = SetStatusExtras {
            runner: self.game.info.as_ref().map(|i| i.runner.clone()),
            title: Some(self.game.title()),
            playnite_id: None,
        };
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
