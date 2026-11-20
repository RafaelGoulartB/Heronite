//! How the Achievements screen looks (see the module's docs).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_achievements::{Achievement, AchievementSet};
use heronite_data::console_theme::Glyphs;
use heronite_i18n::tr;

use super::super::super::CollectionView;
use super::listing::{self, Filter};
use super::{row_top, scroll_state, Shown, FADE_BOTTOM, FADE_TOP, HINTS_BAND, LIST_BOTTOM, PITCH, ROW_H};
use crate::console::kit::{self, u};
use crate::console::pointer::Clickable;
use crate::console::{look, motion, Btn, Pad};
use crate::icons::Icon;
use crate::theme;
use crate::ui::achievements::{long_day, percent_label, source_name};
use crate::ui::icon;

/// Space from the screen's left and right edges, and the widest the list gets.
const SIDE: f32 = 64.;
const MAX_W: f32 = 1480.;
/// Room for the focus ring around a row (the list clips what is outside it).
const RING: f32 = 6.;
const ROW_ICON: f32 = 50.;
const RECENT: usize = 12;
const RECENT_ICON: f32 = 46.;
/// The card of one achievement.
const CARD_W: f32 = 540.;
const CARD_ICON: f32 = 150.;

/// An achievement ready to draw.
struct Item<'a> {
    a: &'a Achievement,
    /// A secret kept hidden: the name and description are not shown.
    masked: bool,
    image: Option<ImageSource>,
}

impl Item<'_> {
    fn name(&self) -> SharedString {
        if self.masked {
            tr!("Secret achievement").into()
        } else {
            self.a.name.clone().into()
        }
    }

    fn description(&self) -> SharedString {
        if self.masked {
            tr!("Hidden until you unlock it.").into()
        } else {
            self.a.description.clone().into()
        }
    }
}

/// The day an achievement was unlocked, in the app's language.
fn day(a: &Achievement) -> Option<String> {
    a.unlocked_at.map(|t| long_day(t.with_timezone(&chrono::Local).date_naive()))
}

/// A row's opacity `t` (0..1) along a fade, eased so the fade looks even.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// A round check, the unlocked mark.
fn check(size: f32, colour: Hsla) -> Div {
    div().flex().flex_none().items_center().justify_center().size(u(size)).rounded_full().bg(colour).child(icon(
        Icon::Check,
        u(size * 0.72),
        theme::console_bg(),
    ))
}

/// An achievement's icon: dimmed with a small lock while locked (greyed when
/// the source has no locked icon), a lock alone for a hidden secret.
fn badge(item: &Item, size: f32) -> Div {
    let radius = u((size * 0.2).round());
    let unlocked = item.a.unlocked;
    let grey = !unlocked && item.a.icon_locked.is_empty();
    let mut d = div().relative().flex_none().size(u(size)).rounded(radius).bg(kit::ink(0.08));
    d = d.child(
        div()
            .size_full()
            .rounded(radius)
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .when(!unlocked, |d| d.opacity(0.55))
            .children(item.image.clone().map(|src| img(src).size_full().object_fit(ObjectFit::Cover).grayscale(grey)))
            .when(item.image.is_none(), |d| {
                d.child(icon(if item.masked { Icon::Lock } else { Icon::Trophy }, u(size * 0.42), theme::text_dim()))
            }),
    );
    if !unlocked && !(item.masked && item.image.is_none()) {
        let lock = (size * 0.36).clamp(16., 30.);
        d = d.child(
            div()
                .absolute()
                .right(u(-4.))
                .bottom(u(-4.))
                .size(u(lock))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::console_sidebar())
                .border_1()
                .border_color(kit::ink(0.16))
                .child(icon(Icon::Lock, u(lock * 0.58), kit::ink(0.75))),
        );
    }
    d
}

/// The rarity tier's trophy, in its colour (faint while none is unlocked).
fn tier_trophy(tier: theme::RarityTier, size: f32, lit: bool) -> Div {
    let colour: Hsla = kit::tint(theme::rarity_colour(tier)).into();
    let colour = if lit { colour } else { colour.opacity(0.35) };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(u(size))
        .rounded_full()
        .bg(colour.opacity(if lit { 0.16 } else { 0.07 }))
        .child(icon(Icon::Trophy, u(size * 0.58), colour))
}

/// "12 / 30": the unlocked count bright, the total dimmer.
fn count_text(got: usize, all: usize, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .text_size(u(size))
        .font_weight(theme::W_BOLD)
        .child(div().text_color(theme::text_primary()).child(got.to_string()))
        .child(div().text_color(theme::text_dim()).child(format!("\u{2009}/\u{2009}{all}")))
}

/// "17 of 50 unlocked · last on Oct 7, 2026" (the count alone before the first unlock).
fn count_line(set: &AchievementSet) -> String {
    let (unlocked, total) = (set.unlocked(), set.total());
    match set.last_unlocked_at() {
        Some(t) => {
            let date = long_day(t.with_timezone(&chrono::Local).date_naive());
            tr!("{unlocked} of {total} unlocked · last on {date}", unlocked, total, date)
        }
        None => tr!("{unlocked} of {total} unlocked", unlocked, total),
    }
}

/// Where the achievements come from, and the points when the source has them.
fn source_line(set: &AchievementSet) -> String {
    let mut parts = vec![source_name(set.source).to_string()];
    if let Some((got, all)) = set.points() {
        parts.push(tr!("{got}/{all} points", got, all));
    }
    parts.join(" \u{b7} ")
}

/// The tier, the points and how many players have it ("Rare · 25 points · 9.8%").
fn rarity_line(a: &Achievement, size: f32, players: bool) -> Option<Div> {
    if a.rarity.is_none() && a.points.is_none() {
        return None;
    }
    let mut d = div().flex().flex_none().items_center().gap(u(6.)).text_size(u(size)).font_weight(theme::W_BOLD);
    if let Some(r) = a.rarity {
        let tier = theme::rarity_tier(r);
        d = d
            .child(icon(Icon::Trophy, u(size + 2.), kit::tint(theme::rarity_colour(tier))))
            .child(div().text_color(kit::tint(theme::rarity_colour(tier))).child(listing::tier_label(tier)));
    }
    let mut rest = Vec::new();
    if let Some(p) = a.points {
        rest.push(tr!("{n} points", n = p));
    }
    if let Some(r) = a.rarity {
        rest.push(if players { tr!("{percent} of players", percent = percent_label(r)) } else { percent_label(r) });
    }
    for (i, text) in rest.into_iter().enumerate() {
        let lead = if i == 0 && a.rarity.is_none() { String::new() } else { "\u{b7} ".to_string() };
        d = d.child(div().text_color(theme::text_muted()).child(format!("{lead}{text}")));
    }
    Some(d)
}

/// Unlocked on a day, the progress of a counted one, or "Not unlocked".
fn status_line(a: &Achievement, size: f32, long: bool) -> Div {
    let d = div().flex().flex_none().items_center().gap(u(6.)).text_size(u(size)).font_weight(theme::W_BOLD);
    if a.unlocked {
        let label = match (day(a), long) {
            (Some(date), true) => tr!("Unlocked {date}", date),
            (Some(date), false) => date,
            (None, _) => tr!("Unlocked").to_string(),
        };
        let mut d = d.text_color(theme::text_primary()).child(check(size + 1., kit::ink(0.82))).child(label);
        if a.hardcore {
            d = d.child(div().text_color(theme::text_dim()).child(format!("\u{b7} {}", tr!("Hardcore"))));
        }
        return d;
    }
    if let Some(p) = a.progress.filter(|p| p.max > 1) {
        return d
            .text_color(theme::text_muted())
            .child(
                div()
                    .w(u(if long { 120. } else { 70. }))
                    .h(u(4.))
                    .rounded_full()
                    .bg(kit::ink(0.14))
                    .child(div().h_full().rounded_full().bg(theme::console_focus()).w(relative(p.fraction().max(0.03)))),
            )
            .child(format!("{}/{}", p.current.min(p.max), p.max));
    }
    d.text_color(kit::ink(0.45)).child(tr!("Not unlocked"))
}

/// One row of the list. `alpha`: its fade near the list's edges.
fn row(item: &Item, i: usize, focused: bool, alpha: f32) -> Div {
    let a = item.a;
    let text = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(u(3.))
        .child(
            div()
                .text_size(u(16.))
                .font_weight(theme::W_BOLD)
                .text_color(if a.unlocked { theme::text_primary().into() } else { kit::ink(0.82) })
                .truncate()
                .child(item.name()),
        )
        .child(div().text_size(u(13.)).line_height(u(17.)).text_color(theme::text_muted()).line_clamp(2).child(item.description()));
    let right = div()
        .flex()
        .flex_none()
        .flex_col()
        .items_end()
        .gap(u(6.))
        .pl(u(16.))
        .children(rarity_line(a, 12.5, false))
        .child(status_line(a, 12.5, false));
    let content = div()
        .relative()
        .size_full()
        .flex()
        .items_center()
        .gap(u(18.))
        .px(u(14.))
        .rounded(u(14.))
        .border_1()
        .child(badge(item, ROW_ICON))
        .child(text)
        .child(right);
    let base = div().absolute().left(u(RING)).right(u(RING)).top(u(row_top(i))).h(u(ROW_H)).opacity(alpha);
    // The surface lightens and the ring of the focus colour comes out a little way.
    base.child(motion::spring(("ach-row", i), content, motion::SOFT, focused, |d, k| {
        let t = k.clamp(0., 1.);
        let ring: Hsla = theme::console_focus().into();
        let gap = u(4.);
        d.bg(kit::ink(0.05 + 0.06 * t)).border_color(kit::ink(0.13 + 0.1 * t)).when(t > 0.01, |d| {
            d.child(
                div()
                    .absolute()
                    .top(-gap)
                    .left(-gap)
                    .right(-gap)
                    .bottom(-gap)
                    .rounded(u(14.) + gap)
                    .border(u(2.5))
                    .border_color(Hsla { a: t, ..ring }),
            )
        })
    }))
}

/// A filter pill; the current one in the focus colour.
fn pill(label: String, current: bool) -> Div {
    let d = div().flex_none().px(u(16.)).py(u(6.)).rounded_full().text_size(u(13.5)).font_weight(theme::W_BOLD).cursor_pointer();
    if current {
        d.bg(theme::console_focus()).text_color(theme::console_on_focus()).child(label)
    } else {
        d.bg(kit::ink(0.12)).text_color(kit::ink(0.86)).hover(|s| s.bg(kit::ink(0.18))).child(label)
    }
}

/// A centred message where the list would be.
fn message(ic: Icon, title: impl Into<SharedString>, body: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(u(10.))
        .pt(u(80.))
        .child(icon(ic, u(40.), theme::text_dim()))
        .child(kit::title(title, 22.))
        .children(body.map(|b| kit::body_text(b).max_w(u(560.)).text_center()))
}

impl CollectionView {
    /// The screen, full screen over the game's art.
    pub(in crate::collection::console) fn render_console_achievements(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(game) = self.ach_game() else {
            let body = if self.lib.is_some() {
                kit::empty(Icon::Warning, tr!("This game is gone"), tr!("Press B to go back."))
            } else {
                message(Icon::Trophy, tr!("Reading the achievements…"), None)
            };
            return div().size_full().flex().justify_center().child(body).into_any_element();
        };
        let key = SharedString::from(game.key.clone());
        self.console_focus_game(Some(key.clone()), cx);
        let (shown, hide_secrets) = self.ach_shown(cx);
        let Some(opened) = self.console.achievements.as_ref().map(|n| n.opened) else { return div().into_any_element() };

        // The width of the list's column (the header lines up with it).
        let s = kit::scale();
        let vw = f32::from(crate::ui::content_size(window).width) / s;
        let col_w = (vw - 2. * SIDE).min(MAX_W);

        let logo = match self.console_logo(&key, 400., 100., false, window) {
            Some(src) => super::super::hero::logo_element(src, 400., 100., window),
            None => kit::title(game.title.clone(), 34.).line_clamp(2).max_w(u(560.)).into_any_element(),
        };
        let mut left = div().flex().flex_col().gap(u(18.)).min_w_0().child(div().flex().child(logo));
        let mut right: Option<Div> = None;
        let mut body: AnyElement;
        let mut detail: Option<AnyElement> = None;

        match &shown {
            Shown::Set(entry) => {
                let set = entry.primary().expect("a set");
                // The list as filtered and ordered now, the focus kept on its achievement.
                let (filter, order) = {
                    let nav = self.console.achievements.as_ref().expect("open");
                    (nav.filter, nav.order)
                };
                let ids = listing::listed(set, filter, order);
                let nav = self.console.achievements.as_mut().expect("open");
                Self::ach_settle(nav, set, &ids);
                let (focus, open_detail) = (nav.focus, nav.detail);
                let revealed = nav.revealed.clone();
                let (offset, view) = scroll_state(&nav.list);
                let glide = nav.list.clone();
                let masked = |a: &Achievement| hide_secrets && a.is_secret() && !revealed.contains(&a.id);
                if let Some(nav) = self.console.achievements.as_mut() {
                    nav.secret = open_detail && ids.get(focus).is_some_and(|&ix| hide_secrets && set.achievements[ix].is_secret());
                }

                // Recently unlocked: look only.
                let recent = listing::recent_icons(set, RECENT);
                if !recent.is_empty() {
                    let icons: Vec<Div> = recent
                        .iter()
                        .map(|a| {
                            let image = self.achievement_icon(&a.icon, f32::from(u(RECENT_ICON)), window.scale_factor());
                            badge(&Item { a, masked: false, image }, RECENT_ICON)
                        })
                        .collect();
                    left = left.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(u(10.))
                            .child(
                                div()
                                    .text_size(u(14.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(kit::ink(0.9))
                                    .child(tr!("Recently unlocked")),
                            )
                            .child(div().flex().gap(u(10.)).children(icons)),
                    );
                }
                right = Some(progress(set));

                // The pills between LB and RB.
                let counts = listing::counts(set);
                let keys = look::glyphs() == Glyphs::Keyboard;
                let mut pills = div().flex().items_center().gap(u(10.)).child(kit::clickable_glyph(Btn::LB, keys).on_pads([Pad::LB]));
                for (f, n) in Filter::ALL.into_iter().zip(counts) {
                    pills = pills.child(pill(f.label(n), f == filter).on_mouse_down(MouseButton::Left, self.ach_pill_click(f)));
                }
                pills = pills.child(kit::clickable_glyph(Btn::RB, keys).on_pads([Pad::RB]));

                // Only the rows near the view are built; the rest is room.
                let n = ids.len();
                if view <= 0. {
                    // Laid out on this frame: the fades need its height.
                    window.request_animation_frame();
                }
                let span = if view > 0. { view } else { 1000. };
                let first = ((offset - span) / PITCH).floor().max(0.) as usize;
                let last = (((offset + 2. * span) / PITCH).ceil().max(0.) as usize).min(n);
                let mut rows = Vec::with_capacity(last.saturating_sub(first));
                for (i, &ix) in ids.iter().enumerate().take(last).skip(first) {
                    let a = &set.achievements[ix];
                    let y = row_top(i) - offset + ROW_H * 0.5;
                    let alpha = if view > 0. {
                        let top = if offset > 0.5 { ease(y / FADE_TOP) } else { 1. };
                        top.min(ease((view - HINTS_BAND - y) / FADE_BOTTOM))
                    } else {
                        1.
                    };
                    let masked = masked(a);
                    let image = if masked && a.icon_locked.is_empty() {
                        None
                    } else {
                        let url = if masked { a.icon_locked.as_str() } else { a.icon_now() };
                        self.achievement_icon(url, f32::from(u(ROW_ICON)), window.scale_factor())
                    };
                    let item = Item { a, masked, image };
                    let mut el = row(&item, i, i == focus, alpha);
                    if alpha > 0.35 {
                        el = el.on_mouse_down(MouseButton::Left, self.ach_row_click(i)).on_mouse_move(self.ach_row_hover(i));
                    }
                    rows.push(el);
                }
                let height = row_top(n) - (PITCH - ROW_H) + LIST_BOTTOM;
                let list = if n == 0 {
                    message(Icon::Trophy, tr!("No achievements match."), None).into_any_element()
                } else {
                    div()
                        .id("console-ach-list")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(glide.handle())
                        .child(div().relative().w_full().h(u(height)).children(rows))
                        .child(glide.driver())
                        .into_any_element()
                };
                body = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(div().pl(u(RING)).pt(u(26.)).pb(u(14.)).child(pills))
                    .child(div().flex_1().min_h_0().child(list))
                    .into_any_element();

                if open_detail {
                    if let Some(&ix) = ids.get(focus) {
                        let a = &set.achievements[ix];
                        let masked = masked(a);
                        let image = if masked && a.icon_locked.is_empty() {
                            None
                        } else {
                            let url = if masked { a.icon_locked.as_str() } else { a.icon_now() };
                            self.achievement_icon(url, f32::from(u(CARD_ICON)), window.scale_factor())
                        };
                        detail = Some(card(&Item { a, masked, image }, opened, focus));
                    }
                }
            }
            Shown::Loading => body = message(Icon::Trophy, tr!("Reading the achievements…"), None).into_any_element(),
            Shown::NeedsAccount => {
                body = message(
                    Icon::Trophy,
                    tr!("Connect RetroAchievements"),
                    Some(
                        tr!("Connect your RetroAchievements account in Settings → Achievements to see this game's achievements.")
                            .to_string(),
                    ),
                )
                .into_any_element()
            }
            Shown::Unmatched => {
                body = message(Icon::Trophy, tr!("This ROM doesn't match any RetroAchievements set."), None).into_any_element()
            }
            Shown::Nothing => body = message(Icon::Trophy, tr!("No achievements to show yet."), None).into_any_element(),
        }
        if !matches!(shown, Shown::Set(_)) {
            body = div().flex_1().min_h_0().flex().justify_center().child(body).into_any_element();
        }

        let header = div().flex().items_start().justify_between().gap(u(40.)).pl(u(RING)).child(left).children(right);
        let screen = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .pt(u(44.))
            .pl(u(SIDE - RING))
            .child(div().flex().flex_col().w(u(col_w + 2. * RING)).h_full().child(header).child(body));
        let shade = div().absolute().inset_0().bg(linear_gradient(
            90.,
            linear_color_stop(kit::veil(0.55), 0.),
            linear_color_stop(kit::veil(0.15), 1.),
        ));
        let id = SharedString::from(format!("console-ach-{opened}"));
        div().relative().size_full().child(shade).child(motion::enter(id, screen, 320, 0., 18.)).children(detail).into_any_element()
    }
}

/// The right of the header: at 100% a gold medallion with the count and the
/// tiers; else the tiers' trophies over the percentage and its bar.
fn progress(set: &AchievementSet) -> Div {
    let (unlocked, total) = (set.unlocked(), set.total());
    let pct = (unlocked * 100).checked_div(total).unwrap_or(0);
    let tiers = listing::tier_counts(set);
    let line = |text: String| div().text_size(u(13.)).text_color(theme::text_muted()).child(text);
    if set.is_complete() {
        let gold: Hsla = kit::tint(theme::achievement_gold()).into();
        let orb = div()
            .flex()
            .items_center()
            .justify_center()
            .size(u(96.))
            .rounded_full()
            .bg(gold.opacity(0.16))
            .border(u(2.))
            .border_color(gold.opacity(0.7))
            .shadow(vec![BoxShadow {
                color: gold.opacity(0.35),
                offset: point(px(0.), px(0.)),
                blur_radius: u(28.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(icon(Icon::Trophy, u(52.), gold));
        let small = tiers.map(|t| {
            div().flex().gap(u(18.)).mt(u(12.)).children(listing::TIERS.iter().zip(t).filter(|(_, (_, all))| *all > 0).map(
                |(tier, (got, all))| {
                    div().flex().items_center().gap(u(6.)).child(tier_trophy(*tier, 24., got > 0)).child(count_text(got, all, 13.))
                },
            ))
        });
        return div()
            .flex()
            .flex_none()
            .flex_col()
            .items_center()
            .child(orb)
            .child(kit::title(tr!("100% · Completed"), 28.).mt(u(12.)).text_color(gold))
            .child(line(count_line(set)).mt(u(4.)))
            .child(line(source_line(set)).text_color(theme::text_dim()))
            .children(small);
    }
    // As wide as the four trophies, so the bar does not jump when the rarity arrives.
    let board_w = 4. * 64. + 3. * 18.;
    let board = tiers.map(|t| {
        div().flex().gap(u(18.)).children(listing::TIERS.iter().zip(t).map(|(tier, (got, all))| {
            div()
                .w(u(64.))
                .flex()
                .flex_col()
                .items_center()
                .gap(u(4.))
                .child(tier_trophy(*tier, 44., got > 0))
                .child(count_text(got, all, 13.5))
                .child(div().text_size(u(11.)).text_color(theme::text_dim()).truncate().child(listing::tier_label(*tier)))
        }))
    });
    let fill = theme::console_focus();
    div().flex().flex_none().flex_col().w(u(board_w)).gap(u(16.)).children(board).child(
        div()
            .flex()
            .flex_col()
            .child(kit::title(format!("{pct}%"), 34.))
            .child(
                div()
                    .mt(u(6.))
                    .w_full()
                    .h(u(5.))
                    .rounded_full()
                    .bg(kit::ink(0.16))
                    .child(div().h_full().rounded_full().bg(fill).w(relative((pct as f32 / 100.).max(0.01)))),
            )
            .child(line(count_line(set)).mt(u(8.)))
            .child(line(source_line(set)).text_color(theme::text_dim())),
    )
}

/// One achievement in full over a dim (A on a row): the icon large with its
/// tier's trophy on the corner, the whole name and description, how many
/// players have it and when it was unlocked. A click on the dim closes it.
fn card(item: &Item, opened: u64, focus: usize) -> AnyElement {
    let a = item.a;
    let tier = a.rarity.map(theme::rarity_tier);
    let medal = div().relative().child(badge(item, CARD_ICON)).children(tier.map(|t| {
        div()
            .absolute()
            .right(u(-18.))
            .bottom(u(-14.))
            .rounded_full()
            .bg(theme::console_sidebar())
            .shadow(kit::shadow(0.5, 3., 10.))
            .child(tier_trophy(t, 46., a.unlocked))
    }));
    let panel = div()
        .relative()
        .w(u(CARD_W))
        .max_w(relative(0.92))
        .flex()
        .flex_col()
        .items_center()
        .px(u(44.))
        .pt(u(44.))
        .pb(u(36.))
        .rounded(u(24.))
        .bg(theme::console_sidebar())
        .border_1()
        .border_color(kit::ink(0.12))
        .shadow(kit::shadow(0.5, 10., 40.))
        .eats_clicks()
        .child(medal)
        .child(kit::title(item.name(), 25.).mt(u(26.)).text_center().line_clamp(3))
        .when(!item.description().is_empty(), |d| {
            d.child(
                div().mt(u(8.)).text_size(u(15.)).line_height(u(21.)).text_color(kit::ink(0.82)).text_center().child(item.description()),
            )
        })
        .child(div().mt(u(20.)).w(u(60.)).h(px(1.)).bg(kit::ink(0.15)))
        .child(
            div()
                .mt(u(18.))
                .flex()
                .flex_col()
                .items_center()
                .gap(u(9.))
                .children(rarity_line(a, 14., true))
                .child(status_line(a, 14., true)),
        );
    let id = SharedString::from(format!("console-ach-card-{opened}-{focus}"));
    div()
        .absolute()
        .inset_0()
        .occlude()
        .on_pads([Pad::B])
        .child(motion::fade_in("console-ach-dim", div().absolute().inset_0().bg(kit::dim(0.55)), 180))
        .child(div().absolute().inset_0().flex().items_center().justify_center().child(motion::enter_from(id, panel, 220, 0., 10., 0.4)))
        .into_any_element()
}
