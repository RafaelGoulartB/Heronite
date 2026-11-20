//! Cinema: the focused game's key art full bleed, its logo (or its name in
//! spaced capitals) large left of centre with a line of facts and its genres
//! centred under it, its achievements on the right, and one row of large
//! covers along the bottom. ← → move along the row (LT / RT a page): while
//! the focus travels the game's block steps aside so the art shows whole,
//! and it comes back once the focus rests (the logo, then the rest, as
//! `motion::reveal` brings them in). A plays (the theme's `nav.aButton`), X
//! opens the game's page, Y its actions in a sheet, View the Filters panel.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::TabSpec;
use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::super::hero::{logo_element, played_time};
use super::super::sources::{by_last_played, by_title, keys};
use crate::collection::data::played_ago;
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome};
use crate::console::motion::{self, Axis, Glide, Goal};
use crate::console::pointer::{self, Clickable};
use crate::console::{hint, look, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

/// Space between covers, and room above them for the focused one's lift and ring.
const GAP: f32 = 18.;
const RING_ROOM: f32 = 22.;
/// Room under the row for the hint bar that floats over the bottom.
const HINTS_ROOM: f32 = 70.;
/// Between the game's block and the row.
const BLOCK_GAP: f32 = 46.;
/// The logo's box, and the name's size without one.
const LOGO: (f32, f32) = (600., 170.);
const NAME: f32 = 46.;
/// Heights of the facts and genres lines.
const FACTS_H: f32 = 44.;
const GENRES_H: f32 = 24.;
/// The achievements block's width.
const ACH_W: f32 = 250.;
/// How long the focus must rest before the game's block comes back.
const SETTLE: Duration = Duration::from_millis(400);
/// How far a returning logo slides in, design px.
const LOGO_SLIDE: f32 = -30.;

/// The focus of a Cinema tab.
pub(crate) struct Nav {
    index: usize,
    row: Glide,
    /// The focus moved; the game's block stays away until then.
    moving_until: Option<Instant>,
    /// Counts the times the block came back (a new value replays its entrance).
    returns: u64,
    hidden: bool,
    wake: Option<Task<()>>,
    /// The game's actions (Y), open in a sheet.
    options: Option<ListNav>,
}

impl Default for Nav {
    fn default() -> Self {
        Self { index: 0, row: Glide::new(Axis::X), moving_until: None, returns: 0, hidden: false, wake: None, options: None }
    }
}

/// The left edge of the row and of the game's block: a share of the width,
/// within reason.
fn side(vw: f32) -> f32 {
    (vw * 0.07).clamp(46., 150.)
}

/// The covers' height in a body `body_h` tall: about a third of the screen.
fn cover_h(body_h: f32) -> f32 {
    (body_h * 0.36).clamp(170., 290.)
}

/// One fact: a small label over its value, centred.
fn fact(label: &str, value: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(u(3.))
        .child(div().text_size(u(10.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(label.to_uppercase()))
        .child(value.h(u(22.)).flex().items_center().gap(u(7.)).text_size(u(16.)).font_weight(theme::W_BOLD).text_color(kit::strong()))
}

/// A thin rule between two facts.
fn rule() -> Div {
    div().flex_none().w(px(1.)).h(u(30.)).mx(u(20.)).bg(kit::ink(0.24))
}

/// The game's name in spaced capitals (no logo).
fn spaced_name(title: &str, width: f32) -> Div {
    let size = name_size(title.chars().count(), width);
    div()
        .font_family(kit::heading_font())
        .font_weight(theme::W_BOLD)
        .text_size(u(size))
        .line_height(u(size * 1.2))
        .text_color(kit::strong())
        .text_center()
        .line_clamp(2)
        .child(spaced_words(title))
}

/// `title` in capitals, its letters spaced with a narrow space that never
/// breaks, so a line breaks between words only.
fn spaced_words(title: &str) -> String {
    title
        .split_whitespace()
        .map(|w| w.to_uppercase().chars().map(String::from).collect::<Vec<_>>().join("\u{202f}"))
        .collect::<Vec<_>>()
        .join("\u{2002}")
}

/// The size of a name of `chars` characters in spaced capitals, so it takes
/// at most two lines of `width` design px (a spaced capital, with its
/// space, is about as wide as the size; a line breaks between words, so
/// the longer line gets a little more).
fn name_size(chars: usize, width: f32) -> f32 {
    let per_line = (chars as f32 * 0.6).max(8.);
    (width / (per_line * 1.05)).clamp(26., NAME)
}

/// A soft shade with no edges behind the game's block, so it reads over
/// bright art without darkening the rest (a blurred shadow of the
/// background colour spreading from a line with no height).
fn block_shade(w: f32, h: f32) -> Div {
    let mut c = theme::console_bg();
    c.a = if kit::light() { 0.55 } else { 0.5 };
    div().absolute().left(u(w * 0.15)).right(u(w * 0.15)).top(u(h / 2.)).h(px(0.)).shadow(vec![BoxShadow {
        color: c.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: u(90.),
        spread_radius: u(h * 0.38),
        inset: false,
    }])
}

/// The row's games: the most recently played first (never played ones by
/// title); "Not played yet" by title.
fn order(source: &str, games: &[std::sync::Arc<CollectionGame>]) -> Vec<SharedString> {
    let all: Vec<&CollectionGame> = games.iter().map(|g| g.as_ref()).collect();
    if source.starts_with("status:not-played") {
        keys(by_title(all))
    } else {
        keys(by_last_played(all))
    }
}

impl CollectionView {
    fn cinema_keys(&self, spec: &TabSpec) -> Vec<SharedString> {
        order(&spec.source, &self.console_games(&spec.source))
    }

    /// Playtime, last played, where it plays (the store's icon and name),
    /// when it came out and how long the main story takes, between thin rules.
    fn cinema_facts(&self, g: &CollectionGame) -> Div {
        let dash = || "\u{2014}".to_string();
        let played = played_time(g).unwrap_or_else(dash);
        let last = g.last_played.as_deref().and_then(played_ago).unwrap_or_else(|| tr!("Never").to_string());
        let icon = super::super::super::store_badges::game_icons(&self.with_copies(&g.key)).first().copied();
        let platform = div().children(icon.map(|i| crate::ui::icon(i, u(17.), theme::text_primary()))).child(g.source_label.to_string());
        let mut row = div()
            .flex()
            .items_center()
            .child(fact(tr!("Playtime"), div().child(played)))
            .child(rule())
            .child(fact(tr!("Last played"), div().child(last)))
            .child(rule())
            .child(fact(tr!("Platform"), platform));
        if let Some(year) = release_year(g) {
            row = row.child(rule()).child(fact(tr!("Released"), div().child(year.to_string())));
        }
        let beat = self.wiki.hltb(&g.title, &g.game.title).map(|h| h.main_story).filter(|h| *h > 0.);
        if let Some(hours) = beat {
            row = row.child(rule()).child(fact(tr!("Time to beat"), div().child(heronite_data::hltb::format_hours(hours))));
        }
        row
    }

    /// The game's achievements, when it has a list: a trophy, how many of
    /// how many, the share on the right and a thin bar of it.
    fn cinema_achievements(&self, g: &CollectionGame, cx: &mut Context<Self>) -> Option<Div> {
        if !crate::services::achievements::enabled() {
            return None;
        }
        let svc = crate::services::achievements::get(cx);
        let svc = svc.read(cx);
        let set = svc.entry(&crate::services::achievements::key_of(&g.runner, &g.app_name))?.primary()?;
        let (got, all) = (set.unlocked(), set.total());
        let share = got as f32 / all.max(1) as f32;
        let gold: Hsla = theme::console_focus().into();
        Some(
            div()
                .w(u(ACH_W))
                .flex()
                .items_center()
                .gap(u(14.))
                .child(
                    div()
                        .flex_none()
                        .size(u(40.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gold.opacity(0.14))
                        .child(crate::ui::icon(Icon::Trophy, u(20.), gold)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(u(4.))
                        .child(
                            div()
                                .text_size(u(10.))
                                .font_weight(theme::W_BOLD)
                                .text_color(theme::text_muted())
                                .child(tr!("Achievements").to_uppercase()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(u(4.))
                                        .child(
                                            div()
                                                .text_size(u(17.))
                                                .font_weight(theme::W_BOLD)
                                                .text_color(kit::strong())
                                                .child(got.to_string()),
                                        )
                                        .child(div().text_size(u(13.)).text_color(theme::text_muted()).child(format!("/ {all}"))),
                                )
                                .child(div().text_size(u(13.)).text_color(theme::text_muted()).child(format!("{:.0}%", share * 100.))),
                        )
                        .child(
                            div()
                                .h(u(3.))
                                .w_full()
                                .rounded_full()
                                .bg(kit::ink(0.16))
                                .child(div().h_full().rounded_full().bg(gold).w(relative(share.max(0.01)))),
                        ),
                ),
        )
    }

    /// The game's block: the logo (or the name), the facts and the genres,
    /// centred on each other; it comes in as `id` says.
    fn cinema_block(&mut self, g: &CollectionGame, id: &str, slide: f32, width: f32, window: &Window) -> Div {
        let key = SharedString::from(g.key.clone());
        let logo_w = LOGO.0.min(width);
        let name = match self.console_logo(&key, logo_w, LOGO.1, true, window) {
            Some(src) => logo_element(src, logo_w, LOGO.1, window),
            None => spaced_name(&g.title, width).max_w(u(width)).into_any_element(),
        };
        let genres = div()
            .h(u(GENRES_H))
            .flex()
            .items_center()
            .justify_center()
            .gap(u(18.))
            .text_size(u(14.))
            .text_color(theme::text_muted())
            .children(g.details.genres.iter().take(5).map(|genre| div().whitespace_nowrap().child(genre.clone())));
        let rest = div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(u(14.))
            .child(div().h(u(FACTS_H)).flex().items_center().child(self.cinema_facts(g)))
            .child(genres);
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(u(14.))
            .child(motion::reveal(
                SharedString::from(format!("{id}-logo")),
                div().relative().flex().justify_center().child(name),
                motion::NAME_HOLD,
                motion::NAME_FADE,
                slide,
                0.,
            ))
            .child(motion::reveal(SharedString::from(format!("{id}-rest")), rest, motion::REST_HOLD, motion::REST_FADE, 0., 0.))
    }

    pub(super) fn render_cinema(&mut self, tab: usize, spec: &TabSpec, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let all = self.cinema_keys(spec);
        if all.is_empty() {
            return super::no_games();
        }
        let n = all.len();
        let nav = &mut self.tab_nav(tab).cinema;
        nav.index = nav.index.min(n - 1);
        let moving = nav.moving_until.is_some_and(|t| Instant::now() < t);
        // The focus came to rest: the block comes back, its entrance anew.
        if nav.hidden && !moving {
            nav.hidden = false;
            nav.returns += 1;
        }
        let (index, glide, returns, options) = (nav.index, nav.row.clone(), nav.returns, nav.options.is_some());
        let focused = all[index].clone();
        self.console_focus_game(Some(focused.clone()), cx);
        let game = self.lib.as_ref().and_then(|l| l.get(&focused)).cloned();

        let vw = super::body_width(window);
        let body_h = super::body_height(window, false);
        let side = side(vw);
        let th = cover_h(body_h);
        let tw = th / self.cover_aspect.max(0.5);
        let row_top = body_h - HINTS_ROOM - th - RING_ROOM;

        // The row: one scrolling line of covers, the ones out of sight empty boxes.
        let width = crate::ui::content_size(window).width;
        let seen = super::in_sight(glide.handle().offset().x, tw + GAP, width);
        let mut row = div()
            .id(("console-cinema-row", tab))
            .absolute()
            .left_0()
            .right_0()
            .top(u(row_top))
            .h(u(th + 2. * RING_ROOM))
            .flex()
            .items_center()
            .gap(u(GAP))
            .px(u(side))
            .overflow_x_scroll()
            .track_scroll(glide.handle());
        for (i, key) in all.iter().enumerate() {
            let visible = super::loads(&seen, i, index);
            let pads = pointer::focus_or_activate(pointer::along(index, i, Pad::Left, Pad::Right));
            row = row.child(self.console_tile(key, tw, th, i == index, visible, window).on_pads(pads));
        }
        row = row.child(div().flex_none().w(u(side))).child(glide.driver());

        // The game's block, and its achievements on the right, while the focus rests.
        let block_bottom = body_h - row_top + BLOCK_GAP - RING_ROOM;
        let mut over = div().absolute().inset_0();
        if let (Some(g), false) = (game.as_ref(), moving) {
            let id = format!("cinema-{tab}-{focused}-{returns}");
            let block_w = (vw * 0.5).clamp(520., 900.);
            let block = self.cinema_block(g, &id, LOGO_SLIDE, block_w, window);
            let shade_h = LOGO.1 + FACTS_H + GENRES_H + 70.;
            over = over
                .child(
                    div().absolute().left(u(side)).w(u(block_w)).bottom(u(block_bottom)).h(u(shade_h)).child(block_shade(block_w, shade_h)),
                )
                .child(div().absolute().left(u(side)).w(u(block_w)).bottom(u(block_bottom)).flex().justify_center().child(block));
            if let Some(ach) = self.cinema_achievements(g, cx).filter(|_| vw >= block_w + side * 2. + ACH_W + 40.) {
                let ach =
                    motion::reveal(SharedString::from(format!("{id}-ach")), ach.relative(), motion::REST_HOLD, motion::REST_FADE, 0., 0.);
                over = over.child(
                    div().absolute().right(u(side)).bottom(u(block_bottom + GENRES_H + 14.)).h(u(FACTS_H)).flex().items_center().child(ach),
                );
            }
        }

        // Y: the game's actions in a sheet over the screen.
        let sheet = options.then(|| self.cinema_options_sheet(&focused, cx)).flatten();
        div()
            .relative()
            .size_full()
            .child(over)
            .child(row)
            .children(sheet.map(|s| div().absolute().inset_0().occlude().child(s)))
            .into_any_element()
    }

    /// The sheet of the game's actions (Y).
    fn cinema_options_sheet(&mut self, key: &SharedString, cx: &mut Context<Self>) -> Option<AnyElement> {
        let g = self.lib.as_ref().and_then(|l| l.get(key)).cloned()?;
        let rows = self.game_rows(&g, cx);
        let (tab, _) = self.console_shown_tab();
        let list = self.tab_nav(tab).cinema.options.as_mut()?.render("console-cinema-options", &rows, true);
        let body = div()
            .flex()
            .flex_col()
            .size_full()
            .child(div().px(u(34.)).pt(u(30.)).pb(u(10.)).child(kit::title(tr!("Game options"), 26.)))
            .child(div().flex_1().min_h_0().px(u(12.)).pb(u(10.)).child(list));
        Some(super::super::game::side_sheet("console-cinema-options-panel", body))
    }

    pub(super) fn cinema_pad(&mut self, tab: usize, spec: &TabSpec, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let all = self.cinema_keys(spec);
        let n = all.len();
        if n == 0 {
            return false;
        }
        let i = self.tab_nav(tab).cinema.index.min(n - 1);
        let key = all[i].clone();
        // The actions' sheet takes the presses while it is open.
        if self.tab_nav(tab).cinema.options.is_some() {
            let Some(g) = self.lib.as_ref().and_then(|l| l.get(&key)).cloned() else { return true };
            let rows = self.game_rows(&g, cx);
            let Some(list) = self.tab_nav(tab).cinema.options.as_mut() else { return true };
            match list.on_pad(pad, &rows) {
                Outcome::Activate(id) | Outcome::Changed(id, _) => self.game_press_row(&key, &id, pad, window, cx),
                Outcome::Ignored if matches!(pad, Pad::B | Pad::Y) => self.tab_nav(tab).cinema.options = None,
                _ => {}
            }
            return true;
        }
        let wrap = look::theme().nav.wrap;
        let to = match pad {
            Pad::Left | Pad::Right => super::step(i, n, pad.dir(), wrap),
            Pad::LT | Pad::RT => {
                let page = ((super::body_width(window) / (cover_h(super::body_height(window, false)) / self.cover_aspect + GAP)).floor()
                    as isize
                    - 1)
                .max(1);
                super::step(i, n, if pad == Pad::LT { -page } else { page }, false)
            }
            Pad::A | Pad::X => {
                self.console_activate(key, pad, window, cx);
                return true;
            }
            Pad::Y => {
                self.tab_nav(tab).cinema.options = Some(ListNav::default());
                return true;
            }
            Pad::Select => {
                self.open_console_filters(window, cx);
                return true;
            }
            _ => return false,
        };
        if to != i {
            let tw = cover_h(super::body_height(window, false)) / self.cover_aspect.max(0.5);
            let nav = &mut self.tab_nav(tab).cinema;
            nav.index = to;
            nav.hidden = true;
            nav.moving_until = Some(Instant::now() + SETTLE);
            nav.row.go(Goal::Reveal { ix: to, margin: tw * 0.6 });
            // Drawn again once the focus rests, for the block to come back.
            nav.wake = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SETTLE + Duration::from_millis(20)).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            }));
        }
        true
    }

    /// Menu (with what it leads to) and the tabs on the left; the game's
    /// buttons on the right: A plays (or the page, as the theme's
    /// `nav.aButton` says), X the other, Y its actions, View the filters.
    pub(super) fn cinema_hints(&self, tab: usize, _spec: &TabSpec) -> Vec<Hint> {
        if self.console.tab_navs.get(tab).is_some_and(|n| n.cinema.options.is_some()) {
            return vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change")), hint(Btn::B, tr!("Close"))];
        }
        let theme = look::theme();
        let mut h = vec![hint(Btn::Start, tr!("Menu")).sub(tr!("Settings, downloads, power"))];
        if theme.tabs.len() > 1 {
            let next = &theme.tabs[(self.console.tab + 1) % theme.tabs.len()];
            h.push(hint(Btn::Bumpers, tr!("Tabs")).sub(tr!("Next: {tab}", tab = heronite_i18n::tr(&next.name))));
        }
        let cover = self.console_cover_hints();
        h.extend(cover.into_iter().filter(|c| matches!(c.btn, Btn::A | Btn::X)));
        h.push(hint(Btn::Y, tr!("Game options")));
        h.push(hint(Btn::Select, tr!("Filter")));
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn spaced_names_break_between_words_only() {
        assert_eq!(spaced_words("Ab cd"), "A\u{202f}B\u{2002}C\u{202f}D");
    }

    #[test]
    fn a_long_name_shrinks_to_two_lines() {
        assert_eq!(name_size(10, 600.), NAME);
        let size = name_size(30, 540.);
        assert!(size < NAME && 30. * 0.6 * size * 1.05 <= 540.5, "{size}");
        assert_eq!(name_size(200, 300.), 26.);
    }

    #[test]
    fn the_row_takes_about_a_third_and_the_side_follows_the_width() {
        assert_eq!(cover_h(724.), 724. * 0.36);
        assert_eq!(cover_h(300.), 170.);
        assert_eq!(side(1911.), 1911. * 0.07);
        assert_eq!(side(400.), 46.);
        assert_eq!(side(4000.), 150.);
    }
}
