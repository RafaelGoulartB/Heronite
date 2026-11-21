//! The screen that covers console mode while a game starts, so the desktop
//! never shows between Play and the game (the theme's `launch`): black, then
//! the game's art, still, under a shade, its logo (or name) rising into
//! place (`motion`), and under it a quiet line with the launch's step. It
//! comes up for any launch while console mode is in front (Play, X, the quick
//! launcher, the tray) and leaves, fading to black and out, once the game
//! runs and its window is in front: our window lost the focus, or the
//! desktop says the game has a window (Hyprland; elsewhere a short grace
//! after the game runs). It also leaves when the game ends, on B or a click,
//! and after [`CEILING`]; a launch that fails shows why until B. While it is
//! up it takes every press. Minimize on launch waits for it to leave
//! (`system::hide_after_cover`).

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{LaunchScreen, LaunchShade};
use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_images::Variant;
use heronite_launch::LaunchEvent;

use super::super::CollectionView;
use super::backdrop::{backdrop_id, backdrop_size};
use crate::console::kit::{self, u};
use crate::console::{self, hint, look, motion, Btn, Pad};
use crate::icons::Icon;
use crate::theme;

/// The longest the screen stays, whatever happens to the launch (past a
/// Steam cold start and a slow first start).
pub(super) const CEILING: Duration = Duration::from_secs(120);
/// The game runs and nothing can say where its window is: it is given this
/// long to come up before the screen leaves.
const GRACE: Duration = Duration::from_secs(8);
/// The game runs and the desktop can say whether it has a window: the
/// longest the screen waits for one.
const WINDOW_WAIT: Duration = Duration::from_secs(60);
/// A failed launch's message stays this long unless B comes first.
const ERROR_HOLD: Duration = Duration::from_secs(10);
/// Launch events within this long of console mode starting are the ones
/// the runtime replays for a window that just opened, not new launches.
const REPLAY_WINDOW: Duration = Duration::from_millis(1500);
/// The name stands in for the logo when none came by then (a logo that
/// comes later still replaces it, coming in on its own).
const NAME_AFTER: Duration = Duration::from_millis(2500);
/// How often the screen checks whether the game is in front.
const TICK: Duration = Duration::from_millis(250);
/// How often the desktop is asked for the game's window.
const PROBE_EVERY: Duration = Duration::from_millis(500);

/// The screen's own fade in; the logo starts a moment after it.
const FADE_IN_MS: u64 = 360;
const LOGO_DELAY_MS: u64 = 60;
const LOGO_FADE_MS: u64 = 750;
const LOGO_RISE_MS: u64 = 1000;
/// How far below its place the logo starts, design px.
const LOGO_RISE: f32 = 26.;
/// To black, then out.
const BLACKOUT_MS: u64 = 300;
const OUT_MS: u64 = 300;
/// The logo's box, design px (the logo is fitted in it, enlarged when
/// smaller).
const LOGO_W: f32 = 900.;
const LOGO_H: f32 = 340.;

/// What the desktop says about the game's window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Probe {
    /// Not asked yet.
    Unknown,
    /// It cannot say (only Hyprland can).
    Unsupported,
    Missing,
    Shown,
}

impl Probe {
    fn of(answer: Option<bool>) -> Self {
        match answer {
            None => Probe::Unsupported,
            Some(true) => Probe::Shown,
            Some(false) => Probe::Missing,
        }
    }
}

/// What decides when the screen leaves.
#[derive(Clone, Copy, Debug)]
struct Facts {
    shown_for: Duration,
    /// Since the game runs (`None`: still starting).
    running_for: Option<Duration>,
    /// Our window has the focus.
    active: bool,
    game_window: Probe,
    /// Since the launch failed.
    failed_for: Option<Duration>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    Stay,
    /// The game is in front (or should be by now).
    Ready,
    /// Waited long enough.
    GiveUp,
}

fn decide(f: Facts) -> Verdict {
    if let Some(failed) = f.failed_for {
        return if failed >= ERROR_HOLD { Verdict::GiveUp } else { Verdict::Stay };
    }
    if f.shown_for >= CEILING {
        return Verdict::GiveUp;
    }
    let Some(running) = f.running_for else { return Verdict::Stay };
    if !f.active || f.game_window == Probe::Shown {
        return Verdict::Ready;
    }
    let wait = if f.game_window == Probe::Unsupported { GRACE } else { WINDOW_WAIT };
    if running >= wait {
        Verdict::Ready
    } else {
        Verdict::Stay
    }
}

/// The name's size when there is no logo: large, a step smaller when long,
/// so it takes one or two lines.
fn name_size(chars: usize) -> f32 {
    match chars {
        0..=18 => 76.,
        19..=32 => 60.,
        _ => 46.,
    }
}

/// The darkening over the art.
fn shade_alpha(shade: LaunchShade) -> f32 {
    match shade {
        LaunchShade::None => 0.,
        LaunchShade::Soft => 0.35,
        LaunchShade::Dark => 0.6,
    }
}

/// Sine ease-out.
fn sine_out(t: f32) -> f32 {
    (t.clamp(0., 1.) * std::f32::consts::FRAC_PI_2).sin()
}

/// Quintic ease-out.
fn quint_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(5)
}

/// The logo's entrance `ms` after it began: its opacity and how far below
/// its place it is (0..=1 of the rise). `rise`: it may move.
fn logo_at(ms: f32, rise: bool) -> (f32, f32) {
    let t = ms.max(0.);
    let opacity = sine_out(t / LOGO_FADE_MS as f32);
    let below = if rise { 1. - quint_out(t / LOGO_RISE_MS as f32) } else { 0. };
    (opacity, below)
}

/// The way out `ms` after it began: the black over the screen, then the
/// screen's own opacity.
fn leave_at(ms: f32) -> (f32, f32) {
    let t = (ms / BLACKOUT_MS as f32).clamp(0., 1.);
    let black = t * t * (3. - 2. * t);
    let out = sine_out((ms - BLACKOUT_MS as f32) / OUT_MS as f32);
    (black, 1. - out)
}

/// The screen over console mode for one launch.
pub(super) struct Cover {
    key: SharedString,
    app_name: String,
    title: SharedString,
    since: Instant,
    /// Animation ids: a new cover replays its entrance.
    id: u64,
    pid: Option<u32>,
    running_at: Option<Instant>,
    failed: Option<(SharedString, Instant)>,
    probe: Probe,
    probed_at: Option<Instant>,
    leaving: Option<Instant>,
    /// The centre piece up (the logo: `true`, the name) and since when: a
    /// logo that comes after the name comes in on its own.
    piece: Option<(bool, Instant)>,
    /// `HERONITE_DEBUG_LAUNCH_SCREEN`: stays until B, nothing was launched.
    frozen: bool,
    /// The game runs and was in front: the screen is Now Playing.
    playing: Option<super::now_playing::NowPlaying>,
}

/// Console mode's launch screen.
pub(super) struct LaunchUi {
    /// When console mode started (replayed launch events come right after).
    born: Instant,
    cover: Option<Cover>,
    seq: u64,
    ticker: Option<Task<()>>,
    debug_done: bool,
    pub(super) debug_np_done: bool,
}

impl Default for LaunchUi {
    fn default() -> Self {
        Self { born: Instant::now(), cover: None, seq: 0, ticker: None, debug_done: false, debug_np_done: false }
    }
}

impl LaunchUi {
    pub(super) fn cover_is_none(&self) -> bool {
        self.cover.is_none()
    }

    /// Now Playing, when the screen is it: the game's app name and pid.
    pub(super) fn playing_mut(&mut self) -> Option<(&String, Option<u32>, &mut super::now_playing::NowPlaying)> {
        let c = self.cover.as_mut().filter(|c| c.leaving.is_none())?;
        let pid = c.pid;
        let np = c.playing.as_mut()?;
        Some((&c.app_name, pid, np))
    }

    /// The screen as Now Playing for `game` (it runs since `running_at`).
    pub(super) fn now_playing_for(&mut self, game: &CollectionGame, pid: Option<u32>, running_at: Instant, frozen: bool) {
        self.seq += 1;
        self.cover = Some(Cover {
            key: game.key.clone().into(),
            app_name: game.app_name.clone(),
            title: game.title.clone().into(),
            since: Instant::now(),
            id: self.seq,
            pid,
            running_at: Some(running_at),
            failed: None,
            probe: Probe::Unsupported,
            probed_at: None,
            leaving: None,
            piece: None,
            frozen,
            playing: Some(super::now_playing::NowPlaying { was_active: true, ..Default::default() }),
        });
    }
}

/// What a tick asks the loop to do next.
enum Next {
    Stop,
    Wait,
    Probe(u32),
}

/// Whether console mode's launch screen would cover the window for a launch
/// now: the theme shows one and console mode is in front (minimize on
/// launch then waits for it, see `system::hide_after_cover`).
pub(crate) fn covers(cx: &mut App) -> bool {
    console::active() && look::theme().launch.screen != LaunchScreen::Off && crate::system::console_in_front(cx)
}

impl CollectionView {
    /// A launch event, while console mode is on (before the Collection's own
    /// handling: `playing` still holds a game that just ended).
    pub(in crate::collection) fn console_launch_event(&mut self, ev: &LaunchEvent, cx: &mut Context<Self>) {
        if !console::active() {
            return;
        }
        self.session_card_event(ev);
        match ev {
            LaunchEvent::Launching { app_name } => self.cover_launch(app_name, cx),
            LaunchEvent::Running { app_name, pid } => {
                let covering = self.console.launch.cover.as_mut().filter(|c| c.app_name == *app_name && c.leaving.is_none());
                match covering {
                    Some(c) => {
                        c.pid = Some(*pid);
                        c.running_at.get_or_insert_with(Instant::now);
                        // A made-up session's pid 1: nothing to ask the desktop.
                        if *pid <= 1 {
                            c.probe = Probe::Unsupported;
                        }
                    }
                    // The window opened again while the game runs (the
                    // runtime replays it): Now Playing.
                    None if self.console.launch.born.elapsed() < REPLAY_WINDOW => {
                        let at = super::now_playing::started_at(app_name);
                        if let (Some(at), Some(game)) = (at, self.game_of_app(app_name)) {
                            self.console.launch.now_playing_for(&game, Some(*pid), at, false);
                        }
                    }
                    // Nothing covers the start: minimize on launch need not wait.
                    None => cx.defer(crate::system::hide_after_cover),
                }
            }
            LaunchEvent::Exited { app_name, .. } => {
                if self.console.launch.cover.as_ref().is_some_and(|c| c.app_name == *app_name) {
                    self.leave_launch_screen(false, cx);
                }
            }
            LaunchEvent::Error { app_name, message } => {
                if let Some(c) = self.console.launch.cover.as_mut().filter(|c| c.app_name == *app_name && c.leaving.is_none()) {
                    c.failed = Some((message.clone().into(), Instant::now()));
                }
            }
            LaunchEvent::Preparing { .. } | LaunchEvent::Status { .. } => {}
        }
    }

    /// The library entry of a launch: the focused copy when it is the one,
    /// else the first with that app name.
    pub(super) fn game_of_app(&self, app_name: &str) -> Option<std::sync::Arc<CollectionGame>> {
        let lib = self.lib.as_ref()?;
        let focused = self.selected.as_ref().and_then(|k| lib.get(k)).filter(|g| g.app_name == app_name);
        focused.or_else(|| lib.games.iter().find(|g| g.app_name == app_name)).cloned()
    }

    /// A launch began: the screen comes up when the theme has one and console
    /// mode is in front.
    fn cover_launch(&mut self, app_name: &str, cx: &mut Context<Self>) {
        if look::theme().launch.screen == LaunchScreen::Off || self.console.launch.born.elapsed() < REPLAY_WINDOW {
            return;
        }
        // `HERONITE_DEBUG_FAKE_SESSION` runs on a hidden workspace: as if in front.
        if !crate::system::console_in_front(cx) && crate::env::value("HERONITE_DEBUG_FAKE_SESSION").is_none() {
            return;
        }
        let Some(game) = self.game_of_app(app_name) else { return };
        self.show_cover(&game, false);
    }

    fn show_cover(&mut self, game: &CollectionGame, frozen: bool) {
        let ui = &mut self.console.launch;
        ui.seq += 1;
        ui.cover = Some(Cover {
            key: game.key.clone().into(),
            app_name: game.app_name.clone(),
            title: game.title.clone().into(),
            since: Instant::now(),
            id: ui.seq,
            pid: None,
            running_at: None,
            failed: None,
            probe: Probe::Unknown,
            probed_at: None,
            leaving: None,
            piece: None,
            frozen,
            playing: None,
        });
    }

    /// The screen fades to black and out. `ready`: the game is in front, so
    /// minimize on launch may close the window now.
    fn leave_launch_screen(&mut self, ready: bool, cx: &mut Context<Self>) {
        let Some(c) = self.console.launch.cover.as_mut() else { return };
        if c.leaving.is_some() {
            return;
        }
        let running = c.running_at.is_some() && c.failed.is_none() && c.playing.is_none();
        // Now Playing put away (Back to console): it is not on screen, so it
        // goes at once instead of coming back to fade out over the console.
        let hidden = c.playing.as_ref().is_some_and(|np| np.dismissed);
        if motion::on() && !hidden {
            c.leaving = Some(Instant::now());
        } else {
            self.console.launch.cover = None;
        }
        if ready || running {
            cx.defer(crate::system::hide_after_cover);
        }
        cx.notify();
    }

    /// A press while the screen is up: B (or a click) puts it away, the rest
    /// go nowhere. `false` when it is not up.
    pub(super) fn launch_screen_pad(&mut self, pad: Pad, cx: &mut Context<Self>) -> bool {
        let Some(c) = self.console.launch.cover.as_ref() else { return false };
        if c.playing.is_some() && c.leaving.is_none() {
            return self.now_playing_pad(pad, cx);
        }
        if pad == Pad::B {
            self.leave_launch_screen(false, cx);
        }
        true
    }

    /// One look at the launch: leaves when it is time, asks for the game's
    /// window now and then.
    fn launch_tick(&mut self, window: &Window, cx: &mut Context<Self>) -> Next {
        let active = window.is_window_active();
        let Some(c) = self.console.launch.cover.as_mut() else { return Next::Stop };
        if let Some(t) = c.leaving {
            if t.elapsed() >= Duration::from_millis(BLACKOUT_MS + OUT_MS) {
                self.console.launch.cover = None;
                cx.notify();
                return Next::Stop;
            }
            return Next::Wait;
        }
        if let Some(np) = c.playing.as_mut() {
            // Back from the game: Now Playing again.
            if active && !np.was_active {
                np.dismissed = false;
                np.focus = 0;
                cx.notify();
            }
            np.was_active = active;
            let secs = super::now_playing::started_at(&c.app_name).map_or(0, |t| t.elapsed().as_secs());
            if active && !np.dismissed && secs != np.shown_secs {
                cx.notify();
            }
            return Next::Wait;
        }
        if c.frozen {
            cx.notify();
            return Next::Wait;
        }
        let facts = Facts {
            shown_for: c.since.elapsed(),
            running_for: c.running_at.map(|t| t.elapsed()),
            active,
            game_window: c.probe,
            failed_for: c.failed.as_ref().map(|(_, t)| t.elapsed()),
        };
        // Repaints the step as it changes.
        cx.notify();
        match decide(facts) {
            Verdict::Stay => match c.pid {
                // pid 1 is a made-up session's: every window would be "its".
                Some(pid) if pid > 1 && c.probe != Probe::Unsupported && c.probed_at.is_none_or(|t| t.elapsed() >= PROBE_EVERY) => {
                    c.probed_at = Some(Instant::now());
                    Next::Probe(pid)
                }
                _ => Next::Wait,
            },
            Verdict::Ready => {
                // The game's window is up behind ours: it comes to the front.
                if active && c.probe == Probe::Shown {
                    if let Some(pid) = c.pid.filter(|p| *p > 1) {
                        cx.background_spawn(async move {
                            if let Err(e) = heronite_overlay::game_window::focus(pid) {
                                log::debug!("launch screen: focusing the game: {e}");
                            }
                        })
                        .detach();
                    }
                }
                c.playing = Some(super::now_playing::NowPlaying { was_active: active, ..Default::default() });
                // Minimize on launch: the game is in front now.
                cx.defer(crate::system::hide_after_cover);
                cx.notify();
                Next::Wait
            }
            Verdict::GiveUp => {
                self.leave_launch_screen(false, cx);
                Next::Wait
            }
        }
    }

    /// Keeps the tick going while the screen is up (started from render, which
    /// has the window).
    fn ensure_launch_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = &mut self.console.launch;
        if ui.cover.is_none() || ui.ticker.is_some() {
            return;
        }
        ui.ticker = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let Ok(next) = this.update_in(cx, |this, window, cx| this.launch_tick(window, cx)) else { return };
                match next {
                    Next::Stop => break,
                    Next::Wait => {}
                    Next::Probe(pid) => {
                        let answer = cx.background_spawn(async move { heronite_overlay::game_window::shown(pid) }).await;
                        let set = this.update(cx, |this, _| {
                            if let Some(c) = this.console.launch.cover.as_mut().filter(|c| c.pid == Some(pid)) {
                                c.probe = Probe::of(answer);
                            }
                        });
                        if set.is_err() {
                            return;
                        }
                    }
                }
            }
            this.update(cx, |this, _| this.console.launch.ticker = None).ok();
        }));
    }

    /// `HERONITE_DEBUG_LAUNCH_SCREEN=<app name or key>[:error|:<step>]`: the
    /// screen for that game as if it were starting (nothing is launched; it
    /// stays until B), with a made-up failure or step (`_` for spaces).
    fn debug_launch_screen(&mut self) {
        if self.console.launch.debug_done || self.lib.is_none() {
            return;
        }
        self.console.launch.debug_done = true;
        let Some(spec) = crate::env::value("HERONITE_DEBUG_LAUNCH_SCREEN") else { return };
        if look::theme().launch.screen == LaunchScreen::Off {
            return;
        }
        let (name, extra) = spec.split_once(':').map_or((spec.as_str(), None), |(n, e)| (n, Some(e)));
        let game = self.lib.as_ref().and_then(|l| l.get(name).cloned()).or_else(|| self.game_of_app(name));
        let Some(game) = game else {
            log::warn!("HERONITE_DEBUG_LAUNCH_SCREEN: no game {name}");
            return;
        };
        self.show_cover(&game, true);
        match extra {
            Some("error") => {
                let message = tr!("Could not launch {title}: {error}", title = game.title, error = "wine: could not load kernel32.dll");
                if let Some(c) = self.console.launch.cover.as_mut() {
                    c.failed = Some((message.into(), Instant::now()));
                }
            }
            Some(step) => {
                self.preparing.insert(game.app_name.clone(), super::super::launch_progress::LaunchStep::starting());
                self.set_launch_step(game.app_name.clone(), step.replace('_', " "), true);
            }
            None => {}
        }
    }

    /// The art behind the logo: the game's wide art, else its cover blurred.
    fn launch_art(&mut self, key: &SharedString, window: &Window) -> Option<std::sync::Arc<RenderImage>> {
        let size = backdrop_size(window);
        let art_id = backdrop_id(Some(key), "art", size, 0.);
        let has_art = self.stage_chains.get(key).is_some_and(|c| !c.is_empty()) && !self.images.is_failed(&art_id);
        let (id, variant, chain) = if has_art {
            (art_id, Variant::Stage { blur_px: 0., saturation: 1. }, self.stage_chains.get(key).cloned().unwrap_or_default())
        } else {
            let id = backdrop_id(Some(key), "cover", size, 30.);
            (id, Variant::Stage { blur_px: 30., saturation: 1.2 }, self.cover_chains.get(key).cloned().unwrap_or_default())
        };
        self.images.get_or_request(&id, || chain, size.0, size.1, variant, 1000)
    }

    /// The top right corner (the "Session ended" card, the activity) and the
    /// launch screen, over everything else. `top_bar`: the screen shows it.
    pub(super) fn render_launch_layers(&mut self, top_bar: bool, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let card = self.render_session_card(window, cx);
        let corner = self.render_activity_corner(top_bar, card, window, cx);
        let screen = self.render_launch_screen(window, cx);
        corner.into_iter().chain(screen).collect()
    }

    /// The launch screen, over everything (`None` when it is not up).
    fn render_launch_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.debug_launch_screen();
        self.debug_now_playing();
        self.ensure_launch_ticker(window, cx);
        let opts = look::theme().launch.clone();
        let c = self.console.launch.cover.as_ref()?;
        // Now Playing (`None`: still starting); put away with Back to console.
        let playing = c.playing.as_ref().map(|np| np.dismissed);
        if playing == Some(true) && c.leaving.is_none() {
            return None;
        }
        let playing = playing.is_some();
        let (key, id, since, leaving) = (c.key.clone(), c.id, c.since, c.leaving);
        let title = c.title.clone();
        let failed = c.failed.as_ref().map(|(m, _)| m.clone());
        let step = self.preparing.get(&c.app_name).map(|s| s.label()).unwrap_or_else(|| tr!("Starting...").into());
        let art = (opts.screen == LaunchScreen::ArtLogo || playing).then(|| self.launch_art(&key, window)).flatten();
        let logo = self.console_logo(&key, LOGO_W, LOGO_H, true, window);

        let mut layer = div().absolute().inset_0().bg(black());
        if let Some(img_) = art {
            let picture = img(ImageSource::Render(img_)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover);
            // Still: a slow pan or zoom of a full-screen picture moves it by
            // less than a pixel a frame, and pixel snapping makes that judder.
            let scaled = picture;
            layer =
                layer.child(motion::fade_in(SharedString::from(format!("launch-art-{id}")), div().absolute().inset_0().child(scaled), 300));
            let shade = shade_alpha(opts.shade);
            if shade > 0. {
                layer = layer.child(div().absolute().inset_0().bg(theme::black_a(shade)));
            }
        }
        // The step reads over any art.
        layer = layer.child(div().absolute().left_0().right_0().bottom_0().h(relative(0.34)).bg(linear_gradient(
            0.,
            linear_color_stop(theme::black_a(0.62), 0.),
            linear_color_stop(theme::black_a(0.), 1.),
        )));

        // The logo, or the name when there is none (after a moment, so a
        // logo still loading does not flash the name first).
        let logo_ready = logo.is_some();
        let centre: Option<AnyElement> = match logo {
            Some(src) => Some(img(src).w(u(LOGO_W)).max_w(relative(0.86)).h(u(LOGO_H)).object_fit(ObjectFit::Contain).into_any_element()),
            // A box of its own width, so it wraps between words only (in a
            // box of no width it broke "Contr / ol"), large, smaller when long.
            None if since.elapsed() >= NAME_AFTER || !motion::on() => {
                let size = name_size(title.chars().count());
                Some(
                    div()
                        .w(u(1100.))
                        .max_w(relative(0.8))
                        .text_center()
                        .font_family(kit::heading_font())
                        .font_weight(theme::W_BOLD)
                        .text_size(u(size))
                        .line_height(u(size * 1.15))
                        .line_clamp(2)
                        .text_color(white())
                        .child(title.clone())
                        .into_any_element(),
                )
            }
            None => None,
        };
        // Its entrance starts after the screen's fade, or when it comes.
        let is_logo = logo_ready;
        let entrance = centre.is_some().then(|| {
            let c = self.console.launch.cover.as_mut().expect("drawn while up");
            let at = match c.piece {
                Some((logo, at)) if logo == is_logo => at,
                _ => {
                    let now = Instant::now();
                    c.piece = Some((is_logo, now));
                    now
                }
            };
            at.max(since + Duration::from_millis(FADE_IN_MS + LOGO_DELAY_MS))
        });
        let centre = centre.zip(entrance).map(|(el, start)| {
            let holder = div().relative().w_full().flex().items_center().justify_center().child(el);
            if !motion::on() {
                return holder.into_any_element();
            }
            let rise = !motion::reduced() && opts.motion;
            let total = FADE_IN_MS + LOGO_DELAY_MS + LOGO_RISE_MS.max(LOGO_FADE_MS);
            holder
                .with_animation(
                    SharedString::from(format!("launch-logo-{id}-{is_logo}")),
                    Animation::new(Duration::from_millis(total)),
                    move |d, _| {
                        let ms = Instant::now().saturating_duration_since(start).as_millis() as f32;
                        let (opacity, below) = logo_at(ms, rise);
                        d.opacity(opacity).top(u(LOGO_RISE * below))
                    },
                )
                .into_any_element()
        });
        if playing {
            layer = layer.children(self.render_now_playing(&key, id, window));
        } else {
            layer = layer.child(div().absolute().inset_0().flex().items_center().justify_center().children(centre));
            layer = layer.child(status_line(id, step, failed));
        }

        // In, then on leaving to black and out (worked out here, so the
        // tree, and every animation in it, stays as it was).
        let (black_a, shown_a) = leaving.map_or((0., 1.), |t| leave_at(t.elapsed().as_millis() as f32));
        if leaving.is_some() {
            window.request_animation_frame();
        }
        let el = div()
            .absolute()
            .inset_0()
            .opacity(shown_a)
            .child(motion::fade_in(SharedString::from(format!("launch-in-{id}")), layer, FADE_IN_MS))
            .child(div().absolute().inset_0().bg(theme::black_a(black_a)));
        // Nothing under it takes a click; a click puts it away.
        // Now Playing: a click on the art does nothing, the right button is B.
        let dismiss = self.console_click(move |this, _, cx| {
            if !playing {
                this.leave_launch_screen(false, cx);
            }
        });
        let dismiss_right = self.console_click(|this, _, cx| {
            if !this.now_playing_pad(Pad::B, cx) {
                this.leave_launch_screen(false, cx);
            }
        });
        Some(
            div()
                .id("console-launch-screen")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, dismiss)
                .on_mouse_down(MouseButton::Right, dismiss_right)
                .child(el)
                .into_any_element(),
        )
    }
}

/// Under the logo, near the bottom: a thin line with a light sweeping along
/// it and the launch's step; or, when the launch failed, why and B.
fn status_line(id: u64, step: SharedString, failed: Option<SharedString>) -> Div {
    let column = div().absolute().left_0().right_0().bottom(u(92.)).px(u(40.)).flex().flex_col().items_center().gap(u(16.));
    if let Some(message) = failed {
        let back = kit::hint_item(&hint(Btn::B, tr!("Back")), look::glyphs() == heronite_data::console_theme::Glyphs::Keyboard, 15., 8.)
            .text_color(theme::white_a(0.72));
        return column
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(12.))
                    .w(u(820.))
                    .max_w(relative(1.))
                    .justify_center()
                    .child(crate::ui::icon(Icon::Warning, u(24.), rgb(0xffb74d)))
                    .child(
                        div().min_w_0().text_size(u(19.)).line_height(u(26.)).line_clamp(3).text_color(theme::white_a(0.92)).child(message),
                    ),
            )
            .child(back);
    }
    const TRACK: f32 = 180.;
    const SPARK: f32 = 64.;
    let spark = div().absolute().top_0().bottom_0().w(u(SPARK)).rounded_full().bg(theme::white_a(0.7));
    let moving = motion::on();
    let spark: AnyElement = if moving {
        spark
            .with_animation(
                SharedString::from(format!("launch-sweep-{id}")),
                Animation::new(Duration::from_millis(1600)).repeat().with_easing(ease_in_out),
                |d, t| {
                    // Across and back, fading at the ends.
                    let p = if t < 0.5 { t * 2. } else { 2. - t * 2. };
                    d.left(u((TRACK - SPARK) * p)).opacity(0.35 + 0.65 * (1. - (2. * p - 1.).abs()))
                },
            )
            .into_any_element()
    } else {
        spark.left(u((TRACK - SPARK) / 2.)).into_any_element()
    };
    let track = div().relative().w(u(TRACK)).h(u(3.)).rounded_full().bg(theme::white_a(0.14)).overflow_hidden().child(spark);
    column.child(track).child(div().text_size(u(17.)).text_color(theme::white_a(0.72)).child(step))
}

#[cfg(test)]
mod tests {
    use super::{decide, leave_at, logo_at, Facts, Probe, Verdict, CEILING, ERROR_HOLD, GRACE, WINDOW_WAIT};
    use std::time::Duration;

    fn starting() -> Facts {
        Facts { shown_for: Duration::from_secs(3), running_for: None, active: true, game_window: Probe::Unknown, failed_for: None }
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn stays_while_the_game_starts_even_without_the_focus() {
        assert_eq!(decide(starting()), Verdict::Stay);
        // Steam's own window may take the focus before the game runs.
        assert_eq!(decide(Facts { active: false, ..starting() }), Verdict::Stay);
        assert_eq!(decide(Facts { shown_for: CEILING, ..starting() }), Verdict::GiveUp);
    }

    #[test]
    fn leaves_once_the_game_is_in_front() {
        let running = Facts { running_for: Some(secs(1)), ..starting() };
        assert_eq!(decide(running), Verdict::Stay);
        assert_eq!(decide(Facts { active: false, ..running }), Verdict::Ready, "the game took the focus");
        assert_eq!(decide(Facts { game_window: Probe::Shown, ..running }), Verdict::Ready, "its window is up");
    }

    #[test]
    fn waits_for_the_window_as_long_as_the_desktop_can_tell() {
        let running = |s, probe| Facts { running_for: Some(secs(s)), game_window: probe, ..starting() };
        assert_eq!(decide(running(GRACE.as_secs() - 1, Probe::Unsupported)), Verdict::Stay);
        assert_eq!(decide(running(GRACE.as_secs(), Probe::Unsupported)), Verdict::Ready);
        assert_eq!(decide(running(GRACE.as_secs(), Probe::Missing)), Verdict::Stay, "the desktop says it has no window yet");
        assert_eq!(decide(running(WINDOW_WAIT.as_secs(), Probe::Missing)), Verdict::Ready);
        assert_eq!(decide(running(WINDOW_WAIT.as_secs(), Probe::Unknown)), Verdict::Ready);
    }

    #[test]
    fn a_failed_launch_stays_until_read() {
        let failed = |s| Facts { failed_for: Some(secs(s)), ..starting() };
        assert_eq!(decide(failed(1)), Verdict::Stay);
        // Even past the ceiling: the message came late.
        assert_eq!(decide(Facts { shown_for: CEILING, ..failed(1) }), Verdict::Stay);
        assert_eq!(decide(failed(ERROR_HOLD.as_secs())), Verdict::GiveUp);
    }

    #[test]
    fn the_logo_fades_in_while_rising() {
        assert_eq!(logo_at(0., true), (0., 1.), "hidden and below its place as it begins");
        let (o, below) = logo_at(380., true);
        assert!(o > 0.5 && o < 1. && below > 0. && below < 0.5, "{o} {below}");
        assert_eq!(logo_at(3000., true), (1., 0.));
        assert_eq!(logo_at(800., false).1, 0., "reduced motion: it only fades");
    }

    #[test]
    fn leaves_through_black() {
        assert_eq!(leave_at(0.), (0., 1.));
        let (black, shown) = leave_at(300.);
        assert!((black - 1.).abs() < 1e-4 && (shown - 1.).abs() < 1e-4, "black first, the screen still there");
        assert_eq!(leave_at(600.).1, 0.);
        assert_eq!(leave_at(10_000.), (1., 0.));
    }
}
