//! Spotlight's small parts: the pill buttons, the facts under them (small
//! labels over values), the genre chips, the cards and the focus ring the
//! page's tiles share. Each eases into and out of the focus (the fill, the
//! text, a little growth and the ring follow one spring), so the focus
//! glides from one to the next instead of jumping.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::super::super::super::store_badges;
use super::super::super::super::CollectionView;
use super::super::super::hero::{clip, played_time};
use crate::collection::data::played_ago;
use crate::console::kit::{self, u};
use crate::console::{look, motion};
use crate::icons::Icon;
use crate::theme;

/// A pill's height, design px.
pub(super) const BUTTON_H: f32 = 42.;
/// The facts under the buttons and the genre chips, design px.
pub(super) const FACTS_H: f32 = 40.;
pub(super) const CHIPS_H: f32 = 22.;
/// A card's size and corners, design px.
pub(super) const CARD_W: f32 = 270.;
pub(super) const CARD_H: f32 = 124.;
const CARD_R: f32 = 16.;
/// How far a focused button or card grows on each side, design px.
const GROW: f32 = 2.;
/// The focus ring of the page's tiles: how far out and how thick, design px.
pub(super) const RING_GAP: f32 = 4.;
const RING: f32 = 2.5;
/// A section's title and the room under it, design px.
pub(super) const TITLE: f32 = 15.;
pub(super) const TITLE_H: f32 = 30.;

/// What a card shows: a kicker with its icon, a value, a note and, when it
/// is known, how far along (0..=1).
pub(super) struct Card {
    pub icon: Icon,
    pub title: SharedString,
    pub value: String,
    pub note: String,
    pub progress: Option<f32>,
}

/// A soft glass fill: what the idle pills, chips and panels sit on.
pub(super) fn glass(a: f32) -> Hsla {
    kit::shade(a)
}

/// A pill button: an icon and maybe a label (`wide`: the long Play). Idle,
/// a glass capsule with a thin edge; focused, it fills with the focus colour,
/// grows a touch and glows.
pub(super) fn button(
    id: (&'static str, usize),
    focused: bool,
    ic: Icon,
    label: Option<SharedString>,
    tint: Option<Rgba>,
    wide: bool,
) -> AnyElement {
    let has_label = label.is_some();
    let base = div()
        .relative()
        .flex_none()
        .h(u(BUTTON_H))
        .when(wide, |d| d.min_w(u(190.)))
        .when(has_label && !wide, |d| d.min_w(u(130.)))
        .when(!has_label, |d| d.w(u(BUTTON_H)));
    motion::spring(id, base, motion::SOFT, focused, move |d, k| {
        let t = k.clamp(0., 1.);
        let grow = u(GROW * k.max(0.));
        let idle: Rgba = tint.unwrap_or(theme::text_primary());
        let colour = motion::mix(idle, theme::console_on_focus(), t);
        let fill: Hsla = motion::mix(glass(0.42), theme::console_focus(), t).into();
        let focus: Hsla = theme::console_focus().into();
        let surface = div()
            .absolute()
            .top(-grow)
            .bottom(-grow)
            .left(-grow)
            .right(-grow)
            .rounded_full()
            .bg(fill)
            .border_1()
            .border_color(kit::ink(0.16 * (1. - t)))
            .when(t > 0.01, |d| {
                d.shadow(vec![BoxShadow {
                    color: Hsla { a: 0.38 * t, ..focus },
                    offset: point(px(0.), px(0.)),
                    blur_radius: u(18.),
                    spread_radius: px(0.),
                    inset: false,
                }])
            });
        let content = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(u(8.))
            .when(has_label, |d| d.px(u(if wide { 30. } else { 24. })))
            .text_size(u(16.))
            .font_weight(theme::W_BOLD)
            .text_color(colour)
            .child(crate::ui::icon(ic, u(if has_label { 19. } else { 18. }), colour))
            .children(label);
        d.child(surface).child(content)
    })
}

/// The ring a little way out of a focused tile of the page (`radius`: the
/// tile's corners), faded by `t`.
pub(super) fn ring(radius: f32, t: f32) -> Div {
    let colour: Hsla = theme::console_focus().into();
    let gap = u(RING_GAP);
    div()
        .absolute()
        .top(-gap)
        .left(-gap)
        .right(-gap)
        .bottom(-gap)
        .rounded(u(radius + RING_GAP))
        .border(u(RING))
        .border_color(Hsla { a: t, ..colour })
        .shadow(vec![BoxShadow {
            color: Hsla { a: 0.25 * t, ..colour },
            offset: point(px(0.), px(0.)),
            blur_radius: u(16.),
            spread_radius: px(0.),
            inset: false,
        }])
}

/// A section's title ("About this game"). On a page without glass
/// (Cinema's), over a thin champagne rule.
pub(super) fn section_title(text: &str) -> Div {
    if !look::theme().layouts.showcase_glass {
        let gold: Hsla = theme::console_focus().into();
        return div()
            .h(u(TITLE_H))
            .flex()
            .flex_col()
            .justify_center()
            .gap(u(6.))
            .child(
                div().text_size(u(TITLE)).font_weight(theme::W_BOLD).text_color(kit::strong()).whitespace_nowrap().child(text.to_string()),
            )
            .child(div().w(u(48.)).h(px(1.)).bg(gold.opacity(0.6)));
    }
    div().h(u(TITLE_H)).text_size(u(TITLE)).font_weight(theme::W_BOLD).text_color(kit::strong()).whitespace_nowrap().child(text.to_string())
}

/// One fact: a small label over its value.
fn fact(label: &str, value: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(3.))
        .child(div().text_size(u(10.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(label.to_uppercase()))
        .child(value.h(u(22.)).flex().items_center().text_size(u(16.)).font_weight(theme::W_BOLD).text_color(kit::strong()))
}

/// A thin bar between two facts.
fn divider() -> Div {
    div().flex_none().w(px(1.)).h(u(30.)).mx(u(20.)).bg(kit::ink(0.22))
}

/// The genres as soft chips, at most four.
pub(super) fn genre_chips(g: &CollectionGame) -> Div {
    div().flex().gap(u(6.)).h(u(CHIPS_H)).children(g.details.genres.iter().take(4).map(|genre| {
        div()
            .flex()
            .items_center()
            .px(u(10.))
            .rounded(u(11.))
            .bg(kit::ink(0.13))
            .text_size(u(11.5))
            .text_color(theme::text_primary())
            .whitespace_nowrap()
            .child(genre.clone())
    }))
}

impl CollectionView {
    /// Playtime, last played, where it plays (the store's logo and name) and
    /// the year it came out, each a small label over its value.
    pub(super) fn spotlight_facts(&self, g: &CollectionGame) -> Div {
        let dash = || "\u{2014}".to_string();
        let played = played_time(g).unwrap_or_else(dash);
        let last = g.last_played.as_deref().and_then(played_ago).unwrap_or_else(|| tr!("Never").to_string());
        let icon = store_badges::game_icons(&self.with_copies(&g.key)).first().copied();
        let platform =
            div().gap(u(7.)).children(icon.map(|i| crate::ui::icon(i, u(17.), theme::text_primary()))).child(g.source_label.to_string());
        let mut row = div()
            .flex()
            .items_center()
            .h(u(FACTS_H))
            .child(fact(tr!("Playtime"), div().child(played)))
            .child(divider())
            .child(fact(tr!("Last played"), div().child(last)))
            .child(divider())
            .child(fact(tr!("Platform"), platform));
        if let Some(year) = release_year(g) {
            row = row.child(divider()).child(fact(tr!("Released"), div().child(year.to_string())));
        }
        row
    }

    /// Playtime, How Long To Beat (how far along the main story) and the
    /// journal; the last one only once the journal of the focused game is read.
    pub(super) fn spotlight_cards(&self, g: &CollectionGame) -> Vec<Card> {
        let last = match g.last_played.as_deref().and_then(played_ago) {
            Some(when) => tr!("Last played: {when}", when = when),
            None => tr!("Never played").to_string(),
        };
        let played = played_time(g).unwrap_or_else(|| "\u{2014}".to_string());
        let mut cards = vec![Card { icon: Icon::AccessTime, title: tr!("Playtime").into(), value: played, note: last, progress: None }];
        let main = self.wiki.hltb(&g.title, &g.game.title).map(|h| h.main_story).filter(|h| *h > 0.);
        let hltb = tr!("How Long To Beat");
        cards.push(match main {
            Some(hours) => {
                let done = (g.playtime_minutes > 0).then(|| (g.playtime_minutes as f64 / 60. / hours) as f32);
                let note = match done {
                    Some(d) => {
                        let percent = (d * 100.).round().min(99.) as u32;
                        tr!("Main story \u{b7} you are at {percent}%", percent = percent)
                    }
                    None => tr!("Main story").to_string(),
                };
                Card { icon: Icon::Flag, title: hltb.into(), value: heronite_data::hltb::format_hours(hours), note, progress: done }
            }
            None => Card { icon: Icon::Flag, title: hltb.into(), value: "\u{2014}".into(), note: tr!("No data").into(), progress: None },
        });
        let key = SharedString::from(g.key.clone());
        if let Some(j) = self.journal.data.as_ref().filter(|_| self.journal.is_for(&key)) {
            let n = j.entries.len();
            let note = j.entries.last().map(|e| clip(&e.text, 60)).filter(|t| !t.is_empty());
            cards.push(Card {
                icon: Icon::Notes,
                title: tr!("Journal").into(),
                value: if n == 0 { tr!("No notes yet").to_string() } else { trn!(n, "{n} note", "{n} notes") },
                note: note.map_or(String::new(), |t| format!("\u{201c}{t}\u{201d}")),
                progress: None,
            });
        }
        cards
    }
}

/// A card; focused, a ring of the focus colour a little way out.
pub(super) fn card(i: usize, focused: bool, c: &Card) -> AnyElement {
    let progress = c.progress.map(|p| {
        div()
            .absolute()
            .left(u(18.))
            .right(u(18.))
            .bottom(u(14.))
            .h(u(4.))
            .rounded_full()
            .bg(kit::ink(0.14))
            .child(div().h_full().w(relative(p.clamp(0.02, 1.))).rounded_full().bg(theme::console_focus()))
    });
    let content = div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .px(u(18.))
        .pt(u(14.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(u(8.))
                .text_size(u(13.))
                .text_color(theme::text_dim())
                .child(crate::ui::icon(c.icon, u(16.), theme::text_dim()))
                .child(c.title.clone()),
        )
        .child(kit::title(c.value.clone(), 24.).mt(u(6.)).truncate())
        .child(div().mt(u(2.)).text_size(u(13.5)).text_color(theme::text_muted()).truncate().child(c.note.clone()))
        .children(progress);
    let base = div().relative().flex_none().w(u(CARD_W)).h(u(CARD_H));
    motion::spring(("spot-card", i), base, motion::SOFT, focused, move |d, k| {
        let t = k.clamp(0., 1.);
        let surface = div().absolute().inset_0().rounded(u(CARD_R)).bg(glass(0.48)).border_1().border_color(kit::ink(0.12 * (1. - t)));
        d.child(surface).child(content).when(t > 0.01, |d| d.child(ring(CARD_R, t)))
    })
}
