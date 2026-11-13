//! Entering console mode from anywhere: the app's, not the window's, so it
//! works with the window closed to the tray or behind another app.
//!
//! - A controller connecting does what "When a controller connects" says:
//!   nothing, enter console mode, or ask. Asking shows a notice in the
//!   bottom right corner of the screen (a layer-shell surface over every
//!   window, the overlay's; a desktop notification where there is none) for
//!   [`ASK_FOR`], saying which button to hold.
//! - Holding that button ([`hold::HOLD_FOR`]) enters console mode, asked or
//!   not ("Button to enter console mode"; with it off, Ask still takes the
//!   system's button). The notice shows while it is held, a ring filling.
//!
//! Nothing happens while a game runs (Guide is the in-game overlay's then)
//! or while console mode is in front. While the overlay listens without a
//! game, it owns its button (Guide; Back + Start on Windows): a tap opens
//! it, and it tells when that button is held instead
//! (`heronite_overlay::input::Event::ConsoleMode`, [`overlay_held`]). The
//! controllers are read without focus by `console::input`
//! ([`input::start_buttons`]), which also says when one connects.

mod hold;
mod notice;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui_kit::*;
use heronite_data::console_theme::{AutoConsole, ConsoleButton};
use heronite_data::{ui_state, AppPaths, DataPaths};
use heronite_i18n::tr;

use self::hold::{Hold, Step, HOLD_FOR};
use self::notice::{NoticeView, Shown};
use super::input::{self, PadButton, PadMsg, PadThread};
use crate::overlay::window;
use crate::overlay::OverlayService;
use crate::runtime::GameRuntime;

/// How long the "Ask" notice waits for the button.
const ASK_FOR: Duration = Duration::from_secs(8);
/// How long "Opening console mode…" stays.
const OPENING_FOR: Duration = Duration::from_millis(1200);
/// Connections this close to the last count as the same (Steam adds a
/// virtual controller for the one that connected).
const SAME_CONNECTION: Duration = Duration::from_secs(4);
/// The notice's size, in design px (the overlay's scale).
const NOTICE_SIZE: (f32, f32) = (360., 84.);
/// A Guide held through the overlay shows its ring after this: a tap (the
/// overlay) does not flash it.
const RING_AFTER: Duration = Duration::from_millis(250);

/// Whether the overlay should tell when its button is held ([`guide_held_wanted`]).
static GUIDE_HELD: AtomicBool = AtomicBool::new(false);

/// The overlay's own button (Guide) is the one to hold: while the overlay
/// listens without a game, it says when that button is held.
pub fn guide_held_wanted() -> bool {
    GUIDE_HELD.load(Ordering::Relaxed)
}

/// The overlay's Guide went down (`Some`, since then) or came up early.
pub fn overlay_hold(since: Option<Instant>, cx: &mut App) {
    if let Some(entity) = cx.try_global::<Global>().map(|g| g.0.clone()) {
        entity.update(cx, |s, cx| match since {
            Some(since) => s.overlay_started(since, cx),
            None => s.overlay_cancelled(cx),
        });
    }
}

/// The overlay's Guide was held long enough.
pub fn overlay_held(cx: &mut App) {
    if let Some(entity) = cx.try_global::<Global>().map(|g| g.0.clone()) {
        entity.update(cx, |s, cx| s.overlay_done(cx));
    }
}

struct Global(Entity<ConsoleSummon>);
impl gpui_kit::Global for Global {}

pub struct ConsoleSummon {
    app: AppPaths,
    runtime: Arc<GameRuntime>,
    auto: AutoConsole,
    button: ConsoleButton,
    /// Reads the controllers (and hears them connect) while either option
    /// is on. Dropping it stops the thread.
    reader: Option<(PadThread, Task<()>)>,
    /// The controllers there when the reader starts count as just connected
    /// (the app started with its window open).
    include_present: bool,
    hold: Hold,
    /// The overlay's Guide is held (it reads that button: [`overlay_hold`]).
    overlay_holding: bool,
    /// Bumped by every hold that starts or stops: an older timer does nothing.
    hold_gen: u64,
    asking: Option<Asking>,
    ask_gen: u64,
    notice: Option<WindowHandle<NoticeView>>,
    /// Bumped by every notice shown: an older timer leaves the new one alone.
    notice_gen: u64,
    /// No surface can float over the other windows (no layer-shell, X11):
    /// desktop notifications instead.
    no_surface: bool,
    last_found: Option<Instant>,
    /// `HERONITE_DEBUG_SUMMON=…:window`: the notice in a normal window.
    debug_window: bool,
}

struct Asking {
    name: String,
    until: Instant,
}

/// Starts it (once). `include_present`: the controllers connected now count
/// as just connected (the window opens at this start, not in console mode).
pub fn init(cx: &mut App, data_paths: &DataPaths, runtime: Arc<GameRuntime>, include_present: bool) {
    if cx.has_global::<Global>() {
        return;
    }
    let app = AppPaths::for_data(data_paths);
    // The mirror of console.json, read at every start anyway.
    let ui = ui_state::UiState::load(&app);
    let debug = crate::env::value("HERONITE_DEBUG_SUMMON");
    let entity = cx.new(|_| ConsoleSummon {
        app,
        runtime,
        auto: ui.console_auto,
        button: ui.console_button,
        reader: None,
        include_present,
        hold: Hold::default(),
        overlay_holding: false,
        hold_gen: 0,
        asking: None,
        ask_gen: 0,
        notice: None,
        notice_gen: 0,
        no_surface: !window::supported(),
        last_found: None,
        debug_window: debug.as_deref().is_some_and(|d| d.ends_with(":window")),
    });
    cx.set_global(Global(entity.clone()));
    entity.update(cx, |s, cx| {
        s.sync(cx);
        if let Some(d) = debug {
            s.debug(&d, cx);
        }
    });
}

/// "When a controller connects" or the console button changed (Settings).
pub fn options_changed(cx: &mut App) {
    let Some(entity) = cx.try_global::<Global>().map(|g| g.0.clone()) else { return };
    entity.update(cx, |s, cx| {
        let prefs = super::themes::prefs(&s.app);
        (s.auto, s.button) = (prefs.auto_console, prefs.console_button);
        s.include_present = false;
        s.sync(cx);
    });
}

/// The buttons of `combo`, as the notice names them.
fn label(combo: &[PadButton]) -> String {
    combo
        .iter()
        .map(|b| match b {
            PadButton::Guide => "Guide",
            PadButton::Select => "Back",
            PadButton::Start => "Start",
            PadButton::LB => "LB",
            PadButton::RB => "RB",
            _ => "?",
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// The overlay reads these while it listens (Guide; Back + Start stands for
/// it on Windows).
fn overlay_owns(combo: &[PadButton]) -> bool {
    combo.contains(&PadButton::Guide) || (cfg!(windows) && combo == hold::combo(ConsoleButton::BackStart))
}

impl ConsoleSummon {
    fn wanted(&self) -> bool {
        self.auto != AutoConsole::Off || self.button.resolved() != ConsoleButton::Off
    }

    /// The button set, or the system's when "Ask" has none to ask for.
    fn setting(&self) -> ConsoleButton {
        match self.button.resolved() {
            ConsoleButton::Off if self.auto == AutoConsole::Ask => ConsoleButton::Auto,
            b => b,
        }
    }

    /// The reader runs while either option is on.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let guide = self.wanted() && overlay_owns(hold::combo(self.setting()));
        if GUIDE_HELD.swap(guide, Ordering::Relaxed) != guide {
            cx.defer(OverlayService::summon_changed);
        }
        if !self.wanted() {
            self.reader = None;
            self.hold.clear();
            self.asking = None;
            self.close_notice(cx);
            return;
        }
        if self.reader.is_some() {
            return;
        }
        let Some((thread, mut rx)) = input::start_buttons() else { return };
        let task = cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if this.update(cx, |s, cx| s.on_msg(msg, cx)).is_err() {
                    break;
                }
            }
        });
        self.reader = Some((thread, task));
    }

    fn on_msg(&mut self, msg: PadMsg, cx: &mut Context<Self>) {
        match msg {
            PadMsg::Connected(name, plugged) => {
                if plugged || std::mem::take(&mut self.include_present) {
                    self.found(name, cx);
                }
            }
            PadMsg::Button(b, down) => {
                log::trace!("console mode: {b:?} {}", if down { "down" } else { "up" });
                self.button(b, down, cx)
            }
            PadMsg::Disconnected { .. } => {
                // Its buttons never come up.
                if self.hold.since().is_some() {
                    self.hold.clear();
                    self.hold_gen += 1;
                    self.after_hold(cx);
                } else {
                    self.hold.clear();
                }
            }
            PadMsg::Press(_) => {}
        }
    }

    /// Why nothing may happen now, if so.
    fn blocked(&self, cx: &mut Context<Self>) -> Option<&'static str> {
        if !self.runtime.running_apps().is_empty() {
            return Some("a game runs");
        }
        if crate::system::console_in_front(cx) {
            return Some("console mode is in front");
        }
        None
    }

    /// The buttons to hold now (none with the setting off, unless asked).
    fn combo(&self) -> &'static [PadButton] {
        match self.button.resolved() {
            ConsoleButton::Off if self.asking.is_none() => &[],
            _ => hold::combo(self.setting()),
        }
    }

    /// [`combo`](Self::combo) as this reader sees it: none while the
    /// overlay reads them (it grabs the controller: the release never
    /// comes here), which tells instead ([`overlay_hold`]).
    fn read_combo(&self, cx: &App) -> &'static [PadButton] {
        let combo = self.combo();
        if overlay_owns(combo) && OverlayService::listening(cx) {
            &[]
        } else {
            combo
        }
    }

    /// A controller connected.
    fn found(&mut self, name: String, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.last_found.is_some_and(|t| now.duration_since(t) < SAME_CONNECTION) {
            return;
        }
        self.last_found = Some(now);
        if let Some(why) = self.blocked(cx) {
            log::info!("console mode: {name} connected; nothing done, {why}");
            return;
        }
        match self.auto {
            AutoConsole::Off => {}
            AutoConsole::On => {
                log::info!("console mode: {name} connected, entering it");
                crate::system::summon_console(cx);
            }
            AutoConsole::Ask => self.ask(name, cx),
        }
    }

    /// Asks to hold the button for [`ASK_FOR`].
    fn ask(&mut self, name: String, cx: &mut Context<Self>) {
        self.ask_gen += 1;
        let gen = self.ask_gen;
        self.asking = Some(Asking { name: name.clone(), until: Instant::now() + ASK_FOR });
        let button = label(self.combo());
        log::info!("console mode: {name} connected, asking to hold {button}");
        if self.no_surface {
            crate::system::notify::send(
                tr!("Controller connected: {name}", name = name.clone()),
                tr!("Hold {button} to enter console mode", button = button.clone()),
            );
        }
        self.show_asking(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ASK_FOR).await;
            this.update(cx, |s, cx| s.ask_ended(gen, cx)).ok();
        })
        .detach();
    }

    fn show_asking(&mut self, cx: &mut Context<Self>) {
        let Some(a) = &self.asking else { return };
        let shown = Shown::Asking { name: a.name.clone(), button: label(self.combo()), until: a.until, total: ASK_FOR };
        self.show(shown, cx);
    }

    fn ask_ended(&mut self, gen: u64, cx: &mut Context<Self>) {
        // A newer question, or the button held: they decide.
        if gen != self.ask_gen || self.hold.since().is_some() || self.overlay_holding {
            return;
        }
        self.asking = None;
        self.close_notice(cx);
    }

    fn button(&mut self, b: PadButton, down: bool, cx: &mut Context<Self>) {
        let combo = self.read_combo(cx);
        self.hold.set_combo(combo);
        match self.hold.button(b, down, Instant::now()) {
            Some(Step::Started(since)) => {
                if let Some(why) = self.blocked(cx) {
                    log::debug!("console mode: button held; nothing done, {why}");
                    return;
                }
                self.hold_gen += 1;
                let gen = self.hold_gen;
                let name = self.asking.as_ref().map(|a| a.name.clone());
                self.show(Shown::Holding { since, total: HOLD_FOR, name }, cx);
                self.hold_timer(gen, HOLD_FOR, cx);
            }
            Some(Step::Cancelled) => {
                self.hold_gen += 1;
                self.after_hold(cx);
            }
            Some(Step::Done) | None => {}
        }
    }

    fn hold_timer(&self, gen: u64, wait: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |s, cx| s.hold_tick(gen, cx)).ok();
        })
        .detach();
    }

    fn hold_tick(&mut self, gen: u64, cx: &mut Context<Self>) {
        if gen != self.hold_gen {
            return;
        }
        match self.hold.tick(Instant::now()) {
            Some(Step::Done) => self.done(cx),
            // The timer woke a moment early.
            _ if self.hold.since().is_some() => self.hold_timer(gen, Duration::from_millis(30), cx),
            _ => {}
        }
    }

    /// The overlay's Guide went down (see [`overlay_hold`]).
    fn overlay_started(&mut self, since: Instant, cx: &mut Context<Self>) {
        if self.combo().is_empty() || self.blocked(cx).is_some() {
            return;
        }
        self.overlay_holding = true;
        self.hold_gen += 1;
        let gen = self.hold_gen;
        let name = self.asking.as_ref().map(|a| a.name.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RING_AFTER).await;
            this.update(cx, |s, cx| {
                if s.hold_gen == gen && s.overlay_holding {
                    s.show(Shown::Holding { since, total: HOLD_FOR, name }, cx);
                }
            })
            .ok();
            // The overlay always says how it ended; should it not, the ring goes.
            cx.background_executor().timer(HOLD_FOR).await;
            this.update(cx, |s, cx| {
                if s.hold_gen == gen && s.overlay_holding {
                    s.overlay_holding = false;
                    s.after_hold(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn overlay_cancelled(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.overlay_holding) {
            self.hold_gen += 1;
            self.after_hold(cx);
        }
    }

    fn overlay_done(&mut self, cx: &mut Context<Self>) {
        self.overlay_holding = false;
        self.hold_gen += 1;
        if self.combo().is_empty() {
            // Held while nothing asked for it ("Ask" with the button off).
            return;
        }
        self.done(cx);
    }

    /// The button came up early: back to the question, or nothing.
    fn after_hold(&mut self, cx: &mut Context<Self>) {
        match &self.asking {
            Some(a) if a.until > Instant::now() => self.show_asking(cx),
            _ => {
                self.asking = None;
                self.close_notice(cx);
            }
        }
    }

    /// Held long enough: console mode.
    fn done(&mut self, cx: &mut Context<Self>) {
        self.asking = None;
        if let Some(why) = self.blocked(cx) {
            log::info!("console mode: button held; nothing done, {why}");
            self.close_notice(cx);
            return;
        }
        log::info!("console mode: button held, entering it");
        self.show(Shown::Opening, cx);
        let gen = self.notice_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OPENING_FOR).await;
            this.update(cx, |s, cx| {
                if s.notice_gen == gen {
                    s.close_notice(cx);
                }
            })
            .ok();
        })
        .detach();
        crate::system::summon_console(cx);
    }

    /// Shows `shown` in the notice, opening it when needed.
    fn show(&mut self, shown: Shown, cx: &mut Context<Self>) {
        self.notice_gen += 1;
        if self.no_surface && !self.debug_window {
            return;
        }
        if let Some(handle) = self.notice {
            let s = shown.clone();
            if handle.update(cx, |v, _, cx| v.set(s, cx)).is_ok() {
                return;
            }
            self.notice = None;
        }
        let display = window::Display::current(cx);
        let scale = display.scale();
        match window::open_corner_notice(cx, display, !self.debug_window, NOTICE_SIZE, |_, cx| cx.new(|_| NoticeView::new(shown, scale))) {
            Ok(handle) => self.notice = Some(handle),
            Err(e) => {
                log::warn!("console mode: cannot show the notice: {e:#}");
                if window::is_layer_shell_missing(&e) {
                    // GNOME: desktop notifications from now on.
                    self.no_surface = true;
                    if let Some(a) = &self.asking {
                        let button = label(self.combo());
                        crate::system::notify::send(
                            tr!("Controller connected: {name}", name = a.name.clone()),
                            tr!("Hold {button} to enter console mode", button),
                        );
                    }
                }
            }
        }
    }

    fn close_notice(&mut self, cx: &mut Context<Self>) {
        self.notice_gen += 1;
        if let Some(handle) = self.notice.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    /// `HERONITE_DEBUG_SUMMON=ask|hold|opening[:window]`: the notice in that
    /// state for a made-up controller (nothing is entered).
    fn debug(&mut self, mode: &str, cx: &mut Context<Self>) {
        let name = "Xbox Wireless Controller".to_string();
        let button = label(self.combo());
        let now = Instant::now();
        let shown = match mode.trim_end_matches(":window") {
            "ask" => Shown::Asking { name, button, until: now + ASK_FOR * 100, total: ASK_FOR * 160 },
            "hold" => Shown::Holding { since: now, total: HOLD_FOR * 1000, name: Some(name) },
            "hold-anytime" => Shown::Holding { since: now, total: HOLD_FOR * 1000, name: None },
            "opening" => Shown::Opening,
            other => {
                log::warn!("HERONITE_DEBUG_SUMMON: unknown {other:?}");
                return;
            }
        };
        self.show(shown, cx);
        if let Some(handle) = self.notice {
            // Frozen part of the way, for captures.
            let _ = handle.update(cx, |v, _, _| v.freeze(0.6));
        }
    }
}
