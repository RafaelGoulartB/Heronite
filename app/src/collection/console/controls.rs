//! Console mode behaviour around the controller: navigation sounds, the idle
//! screen, and what happens at start or when a controller connects. Nothing
//! here runs while console mode is off, except the controller watcher of
//! `autoConsole` (only when that option is on).

mod idle;
mod sound;

use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui_kit::*;
use heronite_data::console_theme::AutoConsole;
use heronite_i18n::tr;

use super::super::CollectionView;
use crate::console::input::{self, PadMsg};
use crate::console::{self, look, startup, themes, Pad};

/// How often the idle screen checks for quiet.
const CHECK_EVERY: Duration = Duration::from_secs(20);
/// How long a controller connecting waits for a press to enter console mode.
const ASK_FOR: Duration = Duration::from_secs(5);

pub(super) struct ControlsState {
    sounds: Option<sound::Player>,
    /// The last press, for the idle screen.
    last_press: Instant,
    /// Quiet time before the idle screen ([`idle::IDLE_AFTER`]).
    idle_after: Duration,
    idle: Option<idle::Idle>,
    idle_task: Option<Task<()>>,
    /// Looks for a controller while console mode is off (`autoConsole`).
    watch: Option<startup::Watcher>,
    /// A connected controller is being asked to press a button.
    asking: bool,
}

impl Default for ControlsState {
    fn default() -> Self {
        Self {
            sounds: None,
            last_press: Instant::now(),
            idle_after: idle::IDLE_AFTER,
            idle: None,
            idle_task: None,
            watch: None,
            asking: false,
        }
    }
}

impl CollectionView {
    /// At app start, before the first frame (console mode may be off): where
    /// the app opens (`openIn`) and, when `autoConsole` is on, the watcher.
    /// Reads only the values mirrored in ui-state.json.
    pub(super) fn console_startup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opens = startup::opens_in_console(self.ui.console_open_in, self.ui.console_mode);
        // `console_init` enters console mode from this flag.
        self.ui.console_mode = opens;
        if !opens && crate::env::value("HERONITE_DEBUG_CONSOLE").is_none() {
            self.watch_pads(self.ui.console_auto, true, window, cx);
        }
    }

    /// `openIn` / `autoConsole` changed while the app runs (Settings):
    /// the watcher follows. The Settings page calls it after
    /// `startup::set_auto_console`.
    pub(super) fn console_start_options_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = themes::prefs(&self.app_paths);
        self.ui.console_open_in = prefs.open_in;
        self.ui.console_auto = prefs.auto_console;
        self.console.controls.watch = None;
        if !console::active() {
            self.watch_pads(prefs.auto_console, false, window, cx);
        }
    }

    /// Console mode just ended (after its state was reset): back to watching
    /// for controllers when `autoConsole` asks for it. The controller that is
    /// connected now does not count.
    pub(super) fn console_controls_ended(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let auto = startup::auto_console(self.ui.console_auto);
        self.watch_pads(auto, false, window, cx);
    }

    /// Console mode just started, its theme installed.
    pub(super) fn console_controls_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.console_auto = themes::prefs(&self.app_paths).auto_console;
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
            let dir = self.app_paths.cache.join("console-sounds");
            st.sounds.get_or_insert_with(|| sound::Player::new(dir)).play(sound);
        }
        false
    }

    /// Drawn over everything when shown (the idle screen).
    pub(super) fn render_console_idle(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.render_idle(window, cx)
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
        let mode = themes::prefs(&self.app_paths).idle;
        self.start_idle(mode, cx);
    }

    /// Starts the controller watcher for `auto` (nothing for `Off`).
    fn watch_pads(&mut self, auto: AutoConsole, including_present: bool, window: &mut Window, cx: &mut Context<Self>) {
        if auto == AutoConsole::Off || console::active() || self.console.controls.watch.is_some() {
            return;
        }
        let Some((watcher, mut rx)) = startup::watch(including_present) else { return };
        self.console.controls.watch = Some(watcher);
        // Ends when the watcher is dropped (its thread closes the channel).
        cx.spawn_in(window, async move |this, cx| {
            while rx.next().await.is_some() {
                if this.update_in(cx, |this, window, cx| this.controller_found(auto, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn controller_found(&mut self, auto: AutoConsole, window: &mut Window, cx: &mut Context<Self>) {
        if console::active() {
            return;
        }
        match auto {
            AutoConsole::On => self.set_console(true, window, cx),
            AutoConsole::Ask => self.ask_for_console(window, cx),
            AutoConsole::Off => {}
        }
    }

    /// Tells that a controller is there and enters console mode on the next
    /// button it presses (console mode is off, so its thread listens only
    /// for [`ASK_FOR`]).
    fn ask_for_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.console.controls.asking {
            return;
        }
        let Some((thread, mut rx)) = input::start() else { return };
        self.console.controls.asking = true;
        self.show_toast(tr!("Controller connected — press any button to enter console mode"), false, cx);
        cx.spawn_in(window, async move |this, cx| {
            let _thread = thread;
            let mut limit = cx.background_executor().timer(ASK_FOR);
            loop {
                match futures::future::select(rx.next(), &mut limit).await {
                    futures::future::Either::Left((Some(PadMsg::Press(pad)), _)) if !matches!(pad, Pad::ScrollUp | Pad::ScrollDown) => {
                        this.update_in(cx, |this, window, cx| {
                            if window.is_window_active() {
                                this.toast = None;
                                this.set_console(true, window, cx);
                            }
                        })
                        .ok();
                        break;
                    }
                    futures::future::Either::Left((Some(_), _)) => {}
                    _ => break,
                }
            }
            this.update(cx, |this, _| this.console.controls.asking = false).ok();
        })
        .detach();
    }
}
