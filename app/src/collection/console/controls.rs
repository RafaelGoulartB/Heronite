//! Console mode behaviour around the controller: navigation sounds and the
//! idle screen. Nothing here runs while console mode is off; what a
//! controller connecting does, and the button held to enter console mode,
//! belong to the app (`console::summon`), so they work with the window in
//! the tray too.

mod idle;
mod sound;

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::super::CollectionView;
use crate::console::{look, startup, themes, Pad};

/// How often the idle screen checks for quiet.
const CHECK_EVERY: Duration = Duration::from_secs(20);

pub(super) struct ControlsState {
    sounds: Option<sound::Player>,
    /// The last press, for the idle screen.
    last_press: Instant,
    /// Quiet time before the idle screen ([`idle::IDLE_AFTER`]).
    idle_after: Duration,
    idle: Option<idle::Idle>,
    idle_task: Option<Task<()>>,
}

impl Default for ControlsState {
    fn default() -> Self {
        Self { sounds: None, last_press: Instant::now(), idle_after: idle::IDLE_AFTER, idle: None, idle_task: None }
    }
}

impl CollectionView {
    /// At app start, before the first frame (console mode may be off): where
    /// the app opens (`openIn`). Reads only the values mirrored in
    /// ui-state.json. `reopened`: the window was closed in console mode in
    /// this run (it opens in it again).
    pub(super) fn console_startup(&mut self, reopened: bool) {
        // `console_init` enters console mode from this flag.
        self.ui.console_mode = reopened || startup::opens_in_console(self.ui.console_open_in, self.ui.console_mode);
    }

    /// `openIn` / `autoConsole` / the console button changed while the app
    /// runs (Settings). `console::summon` heard of it already.
    pub(in crate::collection) fn console_start_options_changed(&mut self) {
        let prefs = themes::prefs(&self.app_paths);
        self.ui.console_open_in = prefs.open_in;
        self.ui.console_auto = prefs.auto_console;
    }

    /// Console mode just started, its theme installed.
    pub(super) fn console_controls_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.console.controls.last_press = Instant::now();
        // `HERONITE_DEBUG_IDLE_SECS=<n>`: the idle screen after `n` seconds of quiet (checked every second).
        let debug_secs = crate::env::parse::<u64>("HERONITE_DEBUG_IDLE_SECS");
        if let Some(n) = debug_secs {
            self.console.controls.idle_after = Duration::from_secs(n);
        }
        let check = if debug_secs.is_some() { Duration::from_secs(1) } else { CHECK_EVERY };
        self.console.controls.idle_task = Some(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(check).await;
            if this.update_in(cx, |this, window, cx| this.idle_tick(window, cx)).is_err() {
                break;
            }
        }));
    }

    /// Every press, before it is routed. `true` when the press only woke the
    /// idle screen: the caller drops it.
    pub(super) fn console_controls_press(&mut self, pad: Pad, cx: &mut Context<Self>) -> bool {
        let st = &mut self.console.controls;
        st.last_press = Instant::now();
        if st.idle.take().is_some() {
            cx.notify();
            return true;
        }
        if !look::theme().controls.sounds {
            st.sounds = None;
        } else if let Some(sound) = sound::for_pad(pad) {
            st.sounds.get_or_insert_with(|| sound::Player::new(self.app_paths.cache.join("console-sounds"))).play(sound);
        }
        false
    }

    /// Every [`CHECK_EVERY`] while console mode is on: shows the idle screen
    /// after [`idle::IDLE_AFTER`] of quiet (a game in front does not count).
    fn idle_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let st = &mut self.console.controls;
        if st.idle.is_some() {
            return;
        }
        if !window.is_window_active() {
            st.last_press = Instant::now();
            return;
        }
        if st.last_press.elapsed() < st.idle_after {
            return;
        }
        // Not set: look again after another quiet spell.
        st.last_press = Instant::now();
        let mode = themes::read(&self.app_paths, |p| p.idle);
        self.start_idle(mode, cx);
    }
}
