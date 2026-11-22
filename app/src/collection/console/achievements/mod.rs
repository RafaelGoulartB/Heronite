//! The console Achievements screen: one game's achievements full screen over
//! its art (the top bar hidden, like a game page). The logo and the latest
//! unlocks on the left, the progress on the right (a medallion in gold at
//! 100%), the filters All / Unlocked / Locked switched with LB / RB, and a
//! one-column list (icon, name, description; the rarity tier and the unlock
//! day on the right) that glides to the focused row. Y switches the order,
//! A opens the focused achievement as a card over a dim (↑ ↓ step it through
//! the list), B closes the card, then the screen. The wheel (and the right
//! stick) scroll the list; the next arrow brings the focus back into view.
//! Every console theme reaches it from the game page's "Achievements" action.

mod listing;
mod view;

use std::collections::HashSet;

use gpui_kit::*;
use heronite_achievements::{AchievementSet, Problem, Source};
use heronite_i18n::tr;

use super::super::CollectionView;
use crate::console::motion::{Axis, Glide, Goal};
use crate::console::{hint, Btn, Hint, Pad};
use crate::services::achievements::{self as service, GameEntry};
use listing::{Filter, Order};

/// The rows' place in the list, design px: each row and the space under it.
const ROW_H: f32 = 74.;
const ROW_GAP: f32 = 10.;
const PITCH: f32 = ROW_H + ROW_GAP;
/// Room above the first row (the focus ring) and below the last one (the
/// floating hint bar and the fade above it).
const LIST_TOP: f32 = 8.;
const LIST_BOTTOM: f32 = 120.;
/// The list fades out over this much at its top once scrolled, and at its
/// bottom above the hint bar's band.
const FADE_TOP: f32 = 46.;
const FADE_BOTTOM: f32 = 70.;
const HINTS_BAND: f32 = 56.;
/// The wheel (or the right stick) scrolls this many rows a notch.
const WHEEL_ROWS: f32 = 2.;

/// The screen's state while it is open.
pub(super) struct AchNav {
    /// The game (its Collection key).
    pub key: SharedString,
    filter: Filter,
    order: Order,
    /// The focused row, and the achievement it shows (kept when the list
    /// changes under it: a refresh, a new unlock).
    focus: usize,
    focus_id: Option<String>,
    /// The focused achievement's card is open.
    detail: bool,
    /// `HERONITE_DEBUG_CONSOLE_ACH=<key>:detail`: open the card once the list is in.
    detail_soon: bool,
    /// Secret achievements shown anyway (X on the card).
    revealed: HashSet<String>,
    list: Glide,
    /// The wheel moved the list since the last arrow: the focus may be out of view.
    wheeled: bool,
    /// The service was asked for the game.
    asked: bool,
    /// The focused achievement is a secret the card hides (X reveals it).
    secret: bool,
    /// Bumped each time the screen opens (its entrance plays again).
    opened: u64,
}

impl AchNav {
    fn new(key: SharedString) -> Self {
        Self {
            key,
            filter: Filter::All,
            order: Order::Default,
            focus: 0,
            focus_id: None,
            detail: false,
            detail_soon: false,
            revealed: HashSet::new(),
            list: Glide::new(Axis::Y),
            wheeled: false,
            asked: false,
            secret: false,
            opened: OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

static OPENED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What the screen shows for the game now.
enum Shown {
    /// The service has not read the game yet.
    Loading,
    Set(GameEntry),
    /// A ROM, and no RetroAchievements account yet.
    NeedsAccount,
    /// A ROM whose file no RetroAchievements set knows.
    Unmatched,
    /// No source has achievements for it (or they are turned off).
    Nothing,
}

impl Shown {
    fn of(entry: Option<GameEntry>) -> Shown {
        let Some(entry) = entry else { return Shown::Loading };
        if entry.primary().is_some() {
            return Shown::Set(entry);
        }
        match entry.problem() {
            Some((Source::RetroAchievements, Problem::NeedsAccount)) => Shown::NeedsAccount,
            Some((Source::RetroAchievements, Problem::Unmatched { .. })) => Shown::Unmatched,
            _ if !entry.loaded || (entry.refreshing && entry.order.iter().all(|s| entry.game.checked_at(*s).is_none())) => Shown::Loading,
            _ => Shown::Nothing,
        }
    }
}

/// The list's scroll offset and height in design px (from the last layout;
/// the height is 0 before the first one).
fn scroll_state(list: &Glide) -> (f32, f32) {
    let s = crate::console::kit::scale();
    let offset = -f32::from(list.handle().offset().y) / s;
    let view = f32::from(list.handle().bounds().size.height) / s;
    (offset.max(0.), view)
}

/// Where row `i` starts in the list, design px.
fn row_top(i: usize) -> f32 {
    LIST_TOP + i as f32 * PITCH
}

/// The offset (design px) that shows row `i` clear of the fades, moving the
/// list as little as possible.
fn reveal_offset(i: usize, offset: f32, view: f32) -> f32 {
    let top = row_top(i) - FADE_TOP - 6.;
    let bottom = row_top(i) + ROW_H + HINTS_BAND + FADE_BOTTOM;
    if view <= 0. || top < offset {
        top.max(0.)
    } else if bottom > offset + view {
        bottom - view
    } else {
        offset
    }
}

/// The row nearest to `focus` that shows whole in the list (after the wheel).
fn nearest_shown(focus: usize, n: usize, offset: f32, view: f32) -> usize {
    if n == 0 || view <= 0. {
        return focus;
    }
    let first = ((offset + FADE_TOP - LIST_TOP) / PITCH).ceil().max(0.) as usize;
    let last = ((offset + view - HINTS_BAND - FADE_BOTTOM - LIST_TOP - ROW_H) / PITCH).floor().max(0.) as usize;
    let (first, last) = (first.min(n - 1), last.min(n - 1).max(first.min(n - 1)));
    focus.clamp(first, last)
}

impl CollectionView {
    /// Opens the Achievements screen of the game `key` over the current screen
    /// (B goes back to it).
    pub(in crate::collection) fn open_console_achievements(&mut self, key: SharedString, cx: &mut Context<Self>) {
        self.console.achievements = Some(AchNav::new(key));
        cx.notify();
    }

    pub(super) fn close_console_achievements(&mut self) {
        self.console.achievements = None;
    }

    /// `HERONITE_DEBUG_CONSOLE_ACH=<game key or runner_appName>[:detail]`:
    /// opens the screen at start (the card too with `:detail`).
    pub(super) fn debug_console_achievements(&mut self, cx: &mut Context<Self>) {
        let Some(value) = crate::env::value("HERONITE_DEBUG_CONSOLE_ACH") else { return };
        let (key, detail) = match value.strip_suffix(":detail") {
            Some(k) => (k.to_string(), true),
            None => (value, false),
        };
        self.open_console_achievements(key.into(), cx);
        if let Some(nav) = self.console.achievements.as_mut() {
            nav.detail_soon = detail;
        }
    }

    /// The game of the open screen.
    fn ach_game(&self) -> Option<std::sync::Arc<heronite_data::CollectionGame>> {
        let key = &self.console.achievements.as_ref()?.key;
        let lib = self.lib.as_ref()?;
        lib.get(key).cloned().or_else(|| lib.games.iter().find(|g| g.app_name == **key).cloned())
    }

    /// What the service has for the open screen's game (asking it once).
    fn ach_shown(&mut self, cx: &mut Context<Self>) -> (Shown, bool) {
        let Some(game) = self.ach_game() else { return (Shown::Loading, true) };
        let svc = service::get(cx);
        if let Some(nav) = self.console.achievements.as_mut().filter(|n| !n.asked) {
            nav.asked = true;
            svc.update(cx, |s, cx| s.want(&game, cx));
        }
        if !service::enabled() {
            return (Shown::Nothing, true);
        }
        let svc = svc.read(cx);
        (Shown::of(svc.entry(&service::key_of(&game.runner, &game.app_name)).cloned()), svc.prefs().hide_secrets)
    }

    /// Keeps the focus on its achievement when the list changed, inside it.
    fn ach_settle(nav: &mut AchNav, set: &AchievementSet, ids: &[usize]) {
        if let Some(i) = nav.focus_id.as_ref().and_then(|id| ids.iter().position(|&ix| set.achievements[ix].id == *id)) {
            nav.focus = i;
        } else {
            nav.focus = nav.focus.min(ids.len().saturating_sub(1));
            if ids.is_empty() {
                nav.detail = false;
            }
        }
        nav.focus_id = ids.get(nav.focus).map(|&ix| set.achievements[ix].id.clone());
        if nav.detail_soon && !ids.is_empty() {
            nav.detail_soon = false;
            nav.detail = true;
        }
    }

    /// Moves the focus to row `i` of `n` (the list follows unless a hover moved it).
    fn ach_focus_row(&mut self, i: usize, reveal: bool) -> bool {
        let Some(nav) = self.console.achievements.as_mut() else { return false };
        nav.wheeled = false;
        if i == nav.focus {
            return false;
        }
        nav.focus = i;
        nav.focus_id = None;
        if reveal {
            let (offset, view) = scroll_state(&nav.list);
            nav.list.go(Goal::Offset(reveal_offset(i, offset, view) * crate::console::kit::scale()));
        }
        true
    }

    /// A press while the screen is open; `false` lets the shell use it.
    pub(super) fn achievements_pad(&mut self, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let (shown, _) = self.ach_shown(cx);
        let Some(nav) = self.console.achievements.as_mut() else { return false };
        let Shown::Set(entry) = shown else {
            if pad == Pad::B {
                self.close_console_achievements();
            }
            return true;
        };
        let Some(set) = entry.primary() else { return true };
        let ids = listing::listed(set, nav.filter, nav.order);
        Self::ach_settle(nav, set, &ids);
        let n = ids.len();
        let s = crate::console::kit::scale();

        if nav.detail {
            match pad {
                Pad::Up | Pad::Left | Pad::LB if nav.focus > 0 => {
                    let i = nav.focus - 1;
                    self.ach_focus_row(i, true);
                }
                Pad::Down | Pad::Right | Pad::RB if nav.focus + 1 < n => {
                    let i = nav.focus + 1;
                    self.ach_focus_row(i, true);
                }
                Pad::X => {
                    if let Some(a) = ids.get(nav.focus).map(|&ix| &set.achievements[ix]).filter(|a| a.is_secret()) {
                        if !nav.revealed.insert(a.id.clone()) {
                            nav.revealed.remove(&a.id);
                        }
                    }
                }
                Pad::A | Pad::B => nav.detail = false,
                _ => {}
            }
            return true;
        }

        match pad {
            Pad::Up | Pad::Down | Pad::LT | Pad::RT if n > 0 => {
                let (offset, view) = scroll_state(&nav.list);
                let to = if nav.wheeled {
                    nearest_shown(nav.focus, n, offset, view)
                } else {
                    let page = ((view - HINTS_BAND - FADE_BOTTOM - FADE_TOP) / PITCH).floor().max(1.) as isize;
                    let step = match pad {
                        Pad::LT => -page,
                        Pad::RT => page,
                        p => p.dir(),
                    };
                    (nav.focus as isize + step).clamp(0, n as isize - 1) as usize
                };
                nav.wheeled = false;
                self.ach_focus_row(to, true);
            }
            Pad::ScrollUp | Pad::ScrollDown => {
                let step = WHEEL_ROWS * PITCH * s;
                nav.list.by(if pad == Pad::ScrollDown { step } else { -step });
                nav.wheeled = true;
            }
            Pad::LB | Pad::RB => {
                nav.filter = nav.filter.step(pad.dir());
                Self::ach_restart(nav);
            }
            Pad::Y => {
                nav.order = nav.order.next();
                Self::ach_restart(nav);
            }
            Pad::A if n > 0 => {
                nav.detail = true;
                // Only a row in view opens (the wheel may have left the focus out of it).
                if nav.wheeled {
                    let (offset, view) = scroll_state(&nav.list);
                    let to = nearest_shown(nav.focus, n, offset, view);
                    self.ach_focus_row(to, false);
                }
            }
            Pad::B => self.close_console_achievements(),
            Pad::Start => return false,
            _ => {}
        }
        true
    }

    /// A new filter or order starts at its top, at once.
    fn ach_restart(nav: &mut AchNav) {
        nav.focus = 0;
        nav.focus_id = None;
        nav.detail = false;
        nav.wheeled = false;
        nav.list.reset();
    }

    /// A click on a pill: that filter.
    fn ach_set_filter(&mut self, filter: Filter) {
        if let Some(nav) = self.console.achievements.as_mut().filter(|n| n.filter != filter) {
            nav.filter = filter;
            Self::ach_restart(nav);
        }
    }

    pub(super) fn achievements_hints(&self) -> Vec<Hint> {
        let Some(nav) = &self.console.achievements else { return Vec::new() };
        if nav.detail {
            let mut h = Vec::new();
            if nav.secret {
                let shown = nav.focus_id.as_ref().is_some_and(|id| nav.revealed.contains(id));
                h.push(hint(Btn::X, if shown { tr!("Hide") } else { tr!("Reveal") }));
            }
            h.push(hint(Btn::B, tr!("Close")));
            return h;
        }
        vec![
            hint(Btn::A, tr!("Open")),
            hint(Btn::Y, nav.order.label()),
            hint(Btn::Bumpers, tr!("Filter")),
            hint(Btn::B, tr!("Back")),
            hint(Btn::Start, tr!("Menu")),
        ]
    }

    /// The presses of a click on row `i`: the focus there, or A when it has it.
    fn ach_row_click(&self, i: usize) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        self.cover_click(move |this| this.ach_focus_row(i, true))
    }

    fn ach_row_hover(&self, i: usize) -> impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static {
        self.cover_hover(move |this| this.ach_focus_row(i, false))
    }

    fn ach_pill_click(&self, filter: Filter) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        self.console_click(move |this, _, cx| {
            this.ach_set_filter(filter);
            crate::console::motion::press();
            cx.notify();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn reveal_moves_the_list_only_as_far_as_needed() {
        // A row in view stays put.
        assert_eq!(reveal_offset(2, 0., 600.), 0.);
        // Below the view: the list rises until the row clears the hint band.
        let off = reveal_offset(10, 0., 600.);
        assert!(off > 0. && row_top(10) + ROW_H + HINTS_BAND + FADE_BOTTOM <= off + 600. + 0.01);
        // Above it: the row comes back under the top fade.
        assert_eq!(reveal_offset(1, 500., 600.), (row_top(1) - FADE_TOP - 6.).max(0.));
    }

    #[test]
    fn after_the_wheel_the_focus_comes_back_into_view() {
        assert_eq!(nearest_shown(0, 50, 10. * PITCH, 600.), ((10. * PITCH + FADE_TOP - LIST_TOP) / PITCH).ceil() as usize);
        assert_eq!(nearest_shown(12, 50, 10. * PITCH, 600.), 12);
        assert!(nearest_shown(49, 50, 0., 600.) < 10);
        assert_eq!(nearest_shown(3, 0, 0., 600.), 3);
    }
}
