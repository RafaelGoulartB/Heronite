//! "Session ended": when a game closes in console mode (the theme's
//! `launch.sessionCard`), a card in the top right corner once the console is
//! in front again, [`DELAY`] later, for [`HOLD`]: the game's name, the time
//! of the session and the total, its logo at the right over its art, faint.
//! It takes no input; a newer session replaces it. A game that ends while
//! the window is closed to the tray (minimize on launch) waits for the window
//! ([`remember`]).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_i18n::tr;
use heronite_images::Variant;
use heronite_launch::LaunchEvent;

use super::super::CollectionView;
use super::backdrop::backdrop_id;
use crate::console::kit::{self, u};
use crate::console::sound::{self, Sound};
use crate::console::{self, look, motion};
use crate::theme;

/// After the console comes to the front.
const DELAY: Duration = Duration::from_millis(1500);
/// How long it shows.
const HOLD: Duration = Duration::from_secs(8);
/// Its way out.
const OUT: Duration = Duration::from_millis(400);
const TICK: Duration = Duration::from_millis(250);
/// Shorter sessions are not recorded (`heronite_launch::record`): the game
/// never really started.
const MIN_SECONDS: u64 = heronite_launch::record::MIN_SESSION_SECONDS as u64;
/// The card, design px.
const CARD_W: f32 = 500.;
const CARD_H: f32 = 100.;
const CARD_R: f32 = 14.;

/// A session that ended.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ended {
    app_name: String,
    seconds: u64,
    /// The total afterwards, minutes, when the record says.
    total: Option<u64>,
}

impl Ended {
    /// From a launch event, for a session worth a card.
    fn of(ev: &LaunchEvent) -> Option<Self> {
        let LaunchEvent::Exited { app_name, elapsed, exit, stopped, record } = ev else { return None };
        if heronite_launch::failed_launch(*exit, *stopped, *elapsed).is_some() || elapsed.as_secs() < MIN_SECONDS {
            return None;
        }
        let total = record.as_ref().map(|r| r.total_played_minutes.max(0) as u64);
        Some(Self { app_name: app_name.clone(), seconds: elapsed.as_secs(), total })
    }
}

/// The time of a session: "<1m" under a minute, else as playtimes read.
fn session_text(seconds: u64) -> String {
    if seconds < 60 {
        tr!("<1m").to_string()
    } else {
        format_playtime_minutes(seconds / 60)
    }
}

/// The card's text.
#[derive(Clone)]
struct Card {
    key: SharedString,
    title: SharedString,
    session: String,
    total: String,
    id: u64,
}

#[derive(Default)]
pub(super) struct SessionUi {
    /// Waits for the console to be in front.
    pending: Option<Ended>,
    front_since: Option<Instant>,
    shown: Option<(Card, Instant)>,
    seq: u64,
    ticker: Option<Task<()>>,
    debug_done: bool,
    /// `HERONITE_DEBUG_SESSION_CARD`: stays.
    frozen: bool,
}

/// A session that ended while no window was open, for the next one.
static WAITING: Mutex<Option<Ended>> = Mutex::new(None);

/// A game ended with the window closed to the tray: the card waits for the
/// window to come back (in console mode).
pub(crate) fn remember(ev: &LaunchEvent) {
    if !console::active() || !look::theme().launch.session_card {
        return;
    }
    if let Some(e) = Ended::of(ev) {
        *WAITING.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
    }
}

impl CollectionView {
    /// A launch event while console mode is on.
    pub(super) fn session_card_event(&mut self, ev: &LaunchEvent) {
        if !look::theme().launch.session_card {
            return;
        }
        if let Some(e) = Ended::of(ev) {
            let ui = &mut self.console.session;
            ui.pending = Some(e);
            ui.front_since = None;
            ui.frozen = false;
        }
    }

    /// The card's text for a session (`None` when the game is gone).
    fn session_card_of(&mut self, e: &Ended) -> Option<Card> {
        let game = self.game_of_app(&e.app_name)?;
        let total = e.total.unwrap_or(game.playtime_minutes.max(e.seconds / 60));
        self.console.session.seq += 1;
        Some(Card {
            key: game.key.clone().into(),
            title: game.title.clone().into(),
            session: session_text(e.seconds),
            total: format_playtime_minutes(total),
            id: self.console.session.seq,
        })
    }

    /// Every [`TICK`] while a card waits or shows. `false`: nothing left.
    fn session_tick(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let ui = &mut self.console.session;
        if let Some((_, at)) = &ui.shown {
            if ui.frozen {
                return false;
            }
            let gone = at.elapsed() >= HOLD + OUT;
            if at.elapsed() >= HOLD {
                cx.notify();
            }
            if gone {
                ui.shown = None;
            }
        }
        if ui.pending.is_some() && self.lib.is_some() {
            // The launch screen first; a game in front does not count.
            // (`HERONITE_DEBUG_FAKE_SESSION` runs on a hidden workspace: as if in front.)
            let front = window.is_window_active() || crate::env::value("HERONITE_DEBUG_FAKE_SESSION").is_some();
            if front && self.console.launch.cover_is_none() {
                let since = *self.console.session.front_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= DELAY {
                    let ended = self.console.session.pending.take().expect("checked");
                    if let Some(card) = self.session_card_of(&ended) {
                        self.console.session.shown = Some((card, Instant::now()));
                        let theme = look::theme();
                        if theme.controls.sounds {
                            sound::play(&self.app_paths.cache, Sound::Confirm, sound::Settings::of(&theme.controls));
                        }
                        cx.notify();
                    }
                }
            } else {
                self.console.session.front_since = None;
            }
        }
        let ui = &self.console.session;
        ui.pending.is_some() || ui.shown.is_some()
    }

    fn ensure_session_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = WAITING.lock().unwrap_or_else(|e| e.into_inner()).take() {
            self.console.session.pending = Some(e);
        }
        let ui = &mut self.console.session;
        if ui.ticker.is_some() || (ui.pending.is_none() && ui.shown.is_none()) {
            return;
        }
        ui.ticker = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                match this.update_in(cx, |this, window, cx| this.session_tick(window, cx)) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => return,
                }
            }
            this.update(cx, |this, _| this.console.session.ticker = None).ok();
        }));
    }

    /// `HERONITE_DEBUG_SESSION_CARD=<app name or key>`: the card for that
    /// game at once (a made-up 47 minute session), staying.
    fn debug_session_card(&mut self) {
        if self.console.session.debug_done || self.lib.is_none() {
            return;
        }
        self.console.session.debug_done = true;
        let Some(name) = crate::env::value("HERONITE_DEBUG_SESSION_CARD") else { return };
        let game = self.lib.as_ref().and_then(|l| l.get(&name).cloned()).or_else(|| self.game_of_app(&name));
        let Some(game) = game else {
            log::warn!("HERONITE_DEBUG_SESSION_CARD: no game {name}");
            return;
        };
        let ended = Ended { app_name: game.app_name.clone(), seconds: 47 * 60, total: None };
        if let Some(card) = self.session_card_of(&ended) {
            self.console.session.shown = Some((card, Instant::now()));
            self.console.session.frozen = true;
        }
    }

    /// The faint art behind the card: the game's wide art at the card's size.
    fn session_card_art(&mut self, key: &SharedString, window: &Window) -> Option<std::sync::Arc<RenderImage>> {
        let px = kit::scale() * window.scale_factor();
        let size = ((CARD_W * px).round() as u32, (CARD_H * px).round() as u32);
        let chain = self.stage_chains.get(key).filter(|c| !c.is_empty())?.clone();
        let id = backdrop_id(Some(key), "card", size, 0.);
        self.images.get_or_request(&id, || chain, size.0, size.1, Variant::Stage { blur_px: 0., saturation: 1. }, 600)
    }

    /// The card, for the top right corner (`None` when none shows).
    pub(super) fn render_session_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.debug_session_card();
        self.ensure_session_ticker(window, cx);
        let (card, at) = self.console.session.shown.clone()?;
        let leaving = !self.console.session.frozen && at.elapsed() >= HOLD;
        let art = self.session_card_art(&card.key, window);
        let logo = self.console_logo(&card.key, 150., 64., true, window);
        let panel = theme::console_sidebar();
        let mut clear = panel;
        clear.a = 0.;
        let figure = |value: String, label: SharedString| {
            div()
                .flex()
                .items_baseline()
                .gap(u(6.))
                .flex_none()
                .child(div().text_size(u(17.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(value))
                .child(div().text_size(u(13.)).text_color(theme::text_muted()).child(label))
        };
        let words = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(div().text_size(u(13.)).text_color(theme::text_muted()).child(tr!("Session ended")))
            .child(
                div()
                    .mt(u(1.))
                    .mb(u(6.))
                    .text_size(u(19.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(card.title.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(u(10.))
                    .child(figure(card.session.clone(), tr!("this session").into()))
                    .child(div().text_size(u(15.)).text_color(theme::text_dim()).child("·"))
                    .child(figure(card.total.clone(), tr!("total").into())),
            );
        let body = div()
            .id(SharedString::from(format!("console-session-card-{}", card.id)))
            .relative()
            .w(u(CARD_W))
            .max_w(relative(1.))
            .h(u(CARD_H))
            .rounded(u(CARD_R))
            .overflow_hidden()
            .bg(Rgba { a: 0.94, ..panel })
            .border_1()
            .border_color(kit::ink(0.12))
            .shadow(kit::shadow(0.45, 8., 28.))
            // Faint on the right, gone under the words on the left.
            .children(art.map(|i| {
                div()
                    .absolute()
                    .inset_0()
                    .child(img(ImageSource::Render(i)).size_full().rounded(u(CARD_R)).object_fit(ObjectFit::Cover).opacity(0.42))
                    .child(
                        // Clipping is square: the layers round their own corners.
                        div().absolute().inset_0().rounded(u(CARD_R)).bg(linear_gradient(
                            90.,
                            linear_color_stop(Rgba { a: 0.94, ..panel }, 0.15),
                            linear_color_stop(clear, 0.85),
                        )),
                    )
            }))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .gap(u(14.))
                    .pl(u(18.))
                    .pr(u(16.))
                    .child(words)
                    .children(logo.map(|src| img(src).flex_none().max_w(u(150.)).max_h(u(64.)).object_fit(ObjectFit::Contain))),
            );
        let id = card.id;
        let shown = motion::enter(SharedString::from(format!("session-card-in-{id}")), body, 320, 40., 0.);
        let out =
            motion::spring(SharedString::from(format!("session-card-out-{id}")), div().child(shown), motion::SOFT, !leaving, |d, k| {
                d.opacity(k.clamp(0., 1.))
            });
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{session_text, Ended};
    use heronite_launch::record::RecordOutcome;
    use heronite_launch::LaunchEvent;
    use std::time::Duration;

    fn exited(secs: u64, record: Option<RecordOutcome>) -> LaunchEvent {
        LaunchEvent::Exited { app_name: "a".into(), elapsed: Duration::from_secs(secs), exit: None, stopped: false, record }
    }

    #[test]
    fn short_sessions_read_as_under_a_minute() {
        assert_eq!(session_text(0), "<1m");
        assert_eq!(session_text(59), "<1m");
        assert_eq!(session_text(60), "1m");
        assert_eq!(session_text(2 * 3600 + 5 * 60 + 30), "2h 5m");
    }

    #[test]
    fn a_card_for_sessions_that_were_recorded() {
        assert_eq!(Ended::of(&exited(3, None)), None, "it never really started");
        let record = RecordOutcome { elapsed_seconds: 600, session_recorded: true, total_played_minutes: 6595 };
        let e = Ended::of(&exited(600, Some(record))).expect("a session");
        assert_eq!((e.seconds, e.total), (600, Some(6595)));
        assert_eq!(Ended::of(&exited(600, None)).and_then(|e| e.total), None, "the total then comes from the library");
        assert_eq!(Ended::of(&LaunchEvent::Launching { app_name: "a".into() }), None);
    }
}
