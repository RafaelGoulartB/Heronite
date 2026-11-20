//! "Now playing": once a game started from console mode runs and is in
//! front, the launch screen stays as this screen, so coming back to console
//! mode while the game runs lands on it: the game's art, still, under the
//! shade, and a frosted card with its box cover, its logo (or name), what it
//! is (developer · year · genres), "Playing now" with the session's time
//! ticking and the total, and Back to game / Stop game / Back to console
//! (← → and A, or a click; B is Back to console). Stop asks first. Back to
//! console leaves the game running; Play on the game (its tile or page)
//! brings this screen back, as does coming back from the game. It goes when
//! the game ends (the "Session ended" card follows).
//!
//! `HERONITE_DEBUG_NOW_PLAYING=<app name or key>[:stop]` shows it for that
//! game (nothing runs), `:stop` with "Stop …?" open.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_images::Variant;
use heronite_launch::LaunchEvent;

use super::super::CollectionView;
use super::backdrop::{backdrop_id, backdrop_size};
use super::hero::logo_element;
use crate::console::kit::{self, u};
use crate::console::list::{simple_dialog, ListNav, Outcome, Row};
use crate::console::{hint, look, motion, Btn, Pad};
use crate::icons::Icon;
use crate::theme;

/// The card, design px.
const CARD_W: f32 = 940.;
const COVER_W: f32 = 200.;
const COVER_H: f32 = 300.;
const CARD_PAD: f32 = 26.;
const CARD_R: f32 = 22.;
const LOGO_W: f32 = 460.;
const LOGO_H: f32 = 130.;

/// When each game launched by this run started running (live events only,
/// read by the hub without a window): the session's time survives the window
/// closing to the tray and opening again.
static STARTED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Every launch event, from the hub.
pub(crate) fn note(ev: &LaunchEvent) {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    let map = started.get_or_insert_with(HashMap::new);
    match ev {
        LaunchEvent::Running { app_name, .. } => {
            map.entry(app_name.clone()).or_insert_with(Instant::now);
        }
        LaunchEvent::Exited { app_name, .. } | LaunchEvent::Error { app_name, .. } => {
            map.remove(app_name);
        }
        _ => {}
    }
}

/// When `app_name` started running, if this run saw it.
pub(super) fn started_at(app_name: &str) -> Option<Instant> {
    STARTED.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(app_name).copied()
}

/// The session's time as a clock: "4:07", "1:02:33".
fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// "Remedy Entertainment · 2019 · Action, Adventure".
fn about_line(g: &CollectionGame) -> String {
    let mut parts = Vec::new();
    if let Some(dev) = g.details.developers.first() {
        parts.push(dev.clone());
    }
    if let Some(year) = heronite_data::organize::release_year(g) {
        parts.push(year.to_string());
    }
    let genres: Vec<&str> = g.details.genres.iter().take(2).map(String::as_str).collect();
    if !genres.is_empty() {
        parts.push(genres.join(", "));
    }
    parts.join(" \u{b7} ")
}

/// The buttons, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Button {
    BackToGame,
    Stop,
    BackToConsole,
}

/// The buttons shown: Back to game only where the game's window can be
/// brought to the front.
fn buttons(can_focus: bool) -> Vec<Button> {
    let mut b = Vec::new();
    if can_focus {
        b.push(Button::BackToGame);
    }
    b.extend([Button::Stop, Button::BackToConsole]);
    b
}

/// What Now Playing keeps while it is up.
#[derive(Default)]
pub(super) struct NowPlaying {
    /// The focused button (index in [`buttons`]).
    pub focus: usize,
    /// "Back to console": hidden until the game is played again or the
    /// window comes back from the game.
    pub dismissed: bool,
    /// The window had the focus at the last look.
    pub was_active: bool,
    /// "Stop the game?" is open.
    pub confirm: Option<ListNav>,
    /// The seconds shown, so the clock repaints once a second.
    pub shown_secs: u64,
}

fn confirm_rows() -> Vec<Row> {
    vec![Row::action("no", tr!("Cancel")).icon(Icon::Close), Row::action("yes", tr!("Stop game")).icon(Icon::Stop)]
}

impl CollectionView {
    /// A press on Now Playing. `false`: it is not up (dismissed).
    pub(super) fn now_playing_pad(&mut self, pad: Pad, cx: &mut Context<Self>) -> bool {
        let Some((app_name, pid, np)) = self.console.launch.playing_mut() else { return false };
        if np.dismissed {
            return false;
        }
        if let Some(nav) = np.confirm.as_mut() {
            match nav.on_pad(pad, &confirm_rows()) {
                Outcome::Activate(id) => {
                    np.confirm = None;
                    if id.as_ref() == "yes" {
                        let app = app_name.clone();
                        cx.defer(move |cx| crate::system::stop_running(cx, &app));
                    }
                }
                Outcome::Ignored if pad == Pad::B => np.confirm = None,
                _ => {}
            }
            cx.notify();
            return true;
        }
        let list = buttons(pid.is_some_and(|p| p > 1) && heronite_overlay::game_window::supported());
        np.focus = np.focus.min(list.len() - 1);
        match pad {
            Pad::Left => np.focus = np.focus.saturating_sub(1),
            Pad::Right => np.focus = (np.focus + 1).min(list.len() - 1),
            Pad::B => np.dismissed = true,
            Pad::A => {
                let button = list[np.focus];
                self.now_playing_press(button, cx);
            }
            _ => {}
        }
        cx.notify();
        true
    }

    fn now_playing_press(&mut self, button: Button, cx: &mut Context<Self>) {
        let Some((_, pid, np)) = self.console.launch.playing_mut() else { return };
        match button {
            Button::BackToGame => {
                if let Some(pid) = pid.filter(|p| *p > 1) {
                    cx.background_spawn(async move {
                        if let Err(e) = heronite_overlay::game_window::focus(pid) {
                            log::debug!("now playing: focusing the game: {e}");
                        }
                    })
                    .detach();
                }
            }
            Button::Stop => np.confirm = Some(ListNav::default()),
            Button::BackToConsole => np.dismissed = true,
        }
        cx.notify();
    }

    /// Play on a game that runs, in console mode: its Now Playing (instead of
    /// stopping it). `false` when the game is not known.
    pub(in crate::collection) fn show_now_playing(&mut self, game: &CollectionGame) -> bool {
        if let Some((app, _, np)) = self.console.launch.playing_mut() {
            if *app == game.app_name {
                np.dismissed = false;
                np.focus = 0;
                return true;
            }
        }
        let pid = None;
        self.console.launch.now_playing_for(game, pid, started_at(&game.app_name).unwrap_or_else(Instant::now), false);
        true
    }

    /// The blurred art the card is frosted with (small: it is blurred).
    fn now_playing_glass(&mut self, key: &SharedString, window: &Window) -> Option<Arc<RenderImage>> {
        let (w, h) = backdrop_size(window);
        let size = ((w / 6).max(1), (h / 6).max(1));
        let chain = self.stage_chains.get(key).filter(|c| !c.is_empty())?.clone();
        let id = backdrop_id(Some(key), "glass", size, 8.);
        self.images.get_or_request(&id, || chain, size.0, size.1, Variant::Stage { blur_px: 8., saturation: 1.2 }, 900)
    }

    /// The card and the hints, over the art (the launch screen draws the
    /// art and the shade). `id`: the screen's, for its animations.
    pub(super) fn render_now_playing(&mut self, key: &SharedString, id: u64, window: &mut Window) -> Vec<AnyElement> {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key).cloned()) else { return Vec::new() };
        let Some((_, pid, np)) = self.console.launch.playing_mut() else { return Vec::new() };
        let (focus, confirming) = (np.focus, np.confirm.is_some());
        let can_focus = pid.is_some_and(|p| p > 1) && heronite_overlay::game_window::supported();
        let since = started_at(&game.app_name);
        let secs = since.map_or(0, |t| t.elapsed().as_secs());
        np.shown_secs = secs;
        let cover = self.cover(key, COVER_W * kit::scale(), COVER_H * kit::scale(), window.scale_factor(), 900);
        let logo = self.console_logo(key, LOGO_W, LOGO_H, true, window);
        let glass = self.now_playing_glass(key, window);
        let total = format_playtime_minutes(game.playtime_minutes.max(secs / 60));

        let white = |a: f32| theme::white_a(a);
        let name_or_logo: AnyElement = match logo {
            Some(src) => div().h(u(LOGO_H)).flex().items_end().child(logo_element(src, LOGO_W, LOGO_H, window)).into_any_element(),
            None => div()
                .w(u(LOGO_W + 120.))
                .font_family(kit::heading_font())
                .font_weight(theme::W_BOLD)
                .text_size(u(super::hero::fitted_title(46., game.title.chars().count())))
                .line_height(u(54.))
                .text_color(white(1.))
                .line_clamp(2)
                .child(game.title.clone())
                .into_any_element(),
        };
        let about = about_line(&game);
        let playing = div()
            .flex()
            .items_center()
            .gap(u(10.))
            .child(div().size(u(10.)).rounded_full().bg(theme::console_on()))
            .child(div().text_size(u(17.)).font_weight(theme::W_BOLD).text_color(white(0.95)).child(tr!("Playing now")))
            .child(div().text_size(u(17.)).text_color(white(0.9)).child(clock(secs)))
            .child(div().text_size(u(15.)).text_color(white(0.45)).child("\u{b7}"))
            .child(div().text_size(u(15.)).text_color(white(0.62)).child(tr!("{total} total", total)));
        let list = buttons(can_focus);
        let row = div().flex().items_center().gap(u(12.)).children(list.iter().enumerate().map(|(i, b)| {
            let (ic, label) = match b {
                Button::BackToGame => (Icon::PlayArrow, tr!("Back to game")),
                Button::Stop => (Icon::Stop, tr!("Stop game")),
                Button::BackToConsole => (Icon::Tv, tr!("Back to console")),
            };
            let focused = i == focus.min(list.len() - 1) && !confirming;
            let b = *b;
            div()
                .id(SharedString::from(format!("now-playing-{i}")))
                .flex()
                .items_center()
                .gap(u(10.))
                .h(u(50.))
                .px(u(22.))
                .rounded_full()
                .cursor_pointer()
                .text_size(u(17.))
                .font_weight(theme::W_BOLD)
                .when(focused, |d| d.bg(white(0.96)).text_color(theme::black_a(0.88)).shadow(kit::shadow(0.35, 4., 14.)))
                .when(!focused, |d| d.bg(white(0.1)).border_1().border_color(white(0.14)).text_color(white(0.94)))
                .child(crate::ui::icon(ic, u(20.), if focused { theme::black_a(0.88) } else { white(0.94) }))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    self.console_click(move |this, _, cx| {
                        if let Some((_, _, np)) = this.console.launch.playing_mut() {
                            np.focus = i;
                        }
                        this.now_playing_press(b, cx);
                    }),
                )
        }));
        let column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .justify_between()
            .h(u(COVER_H))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(10.))
                    .child(name_or_logo)
                    .when(!about.is_empty(), |d| d.child(div().text_size(u(16.)).text_color(white(0.72)).truncate().child(about))),
            )
            .child(div().flex().flex_col().gap(u(18.)).child(playing).child(row));
        let cover_el = div()
            .flex_none()
            .w(u(COVER_W))
            .h(u(COVER_H))
            .rounded(u(12.))
            .overflow_hidden()
            .bg(white(0.08))
            .shadow(kit::shadow(0.5, 10., 30.))
            .children(cover.map(|src| img(src).size_full().rounded(u(12.)).object_fit(ObjectFit::Cover)));
        // Frosted: the art blurred, aligned with the window, clipped to the
        // card's rounded corners, under a dark wash and a thin light edge.
        let frost = glass.map(|img_| {
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let full = crate::ui::content_bounds(window);
                    window.paint_image(bounds, full, Corners::all(u(CARD_R)), img_.clone(), 0, false).ok();
                },
            )
            .absolute()
            .inset_0()
        });
        let card = div()
            .relative()
            .w(u(CARD_W))
            .max_w(relative(0.9))
            .rounded(u(CARD_R))
            .shadow(kit::shadow(0.5, 16., 48.))
            .children(frost)
            .child(div().absolute().inset_0().rounded(u(CARD_R)).bg(theme::black_a(0.42)).border_1().border_color(white(0.16)))
            .child(div().relative().flex().gap(u(28.)).p(u(CARD_PAD)).child(cover_el).child(column));
        let card = motion::enter(SharedString::from(format!("now-playing-in-{id}")), card, 420, 0., 24.);
        let keys = look::glyphs() == heronite_data::console_theme::Glyphs::Keyboard;
        let hints = div()
            .absolute()
            .right(u(46.))
            .bottom(u(26.))
            .flex()
            .gap(u(26.))
            .text_color(white(0.72))
            .child(kit::hint_item(&hint(Btn::A, tr!("Select")), keys, 14., 8.).text_color(white(0.72)))
            .child(kit::hint_item(&hint(Btn::B, tr!("Back to console")), keys, 14., 8.).text_color(white(0.72)));
        let mut out = vec![
            // Darker towards the card, so its edge and the text read over bright art.
            div()
                .absolute()
                .inset_0()
                .bg(linear_gradient(20., linear_color_stop(theme::black_a(0.7), 0.), linear_color_stop(theme::black_a(0.), 0.65)))
                .into_any_element(),
            div().absolute().left(u(96.)).right(u(96.)).bottom(u(110.)).flex().child(card).into_any_element(),
            hints.into_any_element(),
        ];
        if confirming {
            let rows = confirm_rows();
            let (_, _, np) = self.console.launch.playing_mut().expect("checked above");
            let nav = np.confirm.as_mut().expect("confirming");
            let message = tr!("Progress the game has not saved is lost. Heronite saves the playtime so far.");
            let dialog = simple_dialog(nav, "now-playing-stop", tr!("Stop {title}?", title = game.title), message, None, &rows);
            out.push(div().absolute().inset_0().child(dialog).into_any_element());
        }
        out
    }

    /// `HERONITE_DEBUG_NOW_PLAYING=<app name or key>`: Now Playing for that
    /// game, 47 minutes into a made-up session.
    pub(super) fn debug_now_playing(&mut self) {
        if self.console.launch.debug_np_done || self.lib.is_none() {
            return;
        }
        self.console.launch.debug_np_done = true;
        let Some(spec) = crate::env::value("HERONITE_DEBUG_NOW_PLAYING") else { return };
        let (name, stop) = match spec.strip_suffix(":stop") {
            Some(n) => (n.to_string(), true),
            None => (spec, false),
        };
        let game = self.lib.as_ref().and_then(|l| l.get(&name).cloned()).or_else(|| self.game_of_app(&name));
        let Some(game) = game else {
            log::warn!("HERONITE_DEBUG_NOW_PLAYING: no game {name}");
            return;
        };
        let at = Instant::now().checked_sub(Duration::from_secs(47 * 60 + 12)).unwrap_or_else(Instant::now);
        STARTED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(game.app_name.clone(), at);
        self.console.launch.now_playing_for(&game, None, at, true);
        if let (true, Some((_, _, np))) = (stop, self.console.launch.playing_mut()) {
            np.confirm = Some(ListNav::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{buttons, clock, Button};

    #[test]
    fn the_session_reads_as_a_clock() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(247), "4:07");
        assert_eq!(clock(3753), "1:02:33");
    }

    #[test]
    fn back_to_game_only_where_the_window_can_come_forward() {
        assert_eq!(buttons(true), [Button::BackToGame, Button::Stop, Button::BackToConsole]);
        assert_eq!(buttons(false), [Button::Stop, Button::BackToConsole]);
    }
}
