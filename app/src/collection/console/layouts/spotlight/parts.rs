//! Spotlight's buttons and cards. Each eases into and out of the focus (the
//! fill, the text, a little growth and the ring follow one spring), so the
//! focus glides from one to the next instead of jumping.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::super::super::super::CollectionView;
use super::super::super::hero::{clip, played_time};
use crate::collection::data::played_ago;
use crate::console::kit::{self, u};
use crate::console::motion;
use crate::icons::Icon;
use crate::theme;

/// A button's height and corners, design px.
pub(super) const BUTTON_H: f32 = 54.;
const BUTTON_R: f32 = 14.;
/// A card's size and corners, design px.
pub(super) const CARD_W: f32 = 270.;
pub(super) const CARD_H: f32 = 124.;
const CARD_R: f32 = 16.;
/// How far a focused button or card grows on each side, design px.
const GROW: f32 = 3.;

/// What a card shows: a kicker with its icon, a value, a note and, when it
/// is known, how far along (0..=1).
pub(super) struct Card {
    pub icon: Icon,
    pub title: SharedString,
    pub value: String,
    pub note: String,
    pub progress: Option<f32>,
}

/// The surface of a button or card at focus `k` (0..=1, more while it
/// springs past): a soft dark fill that becomes the focus colour, growing a
/// little, with a shadow.
fn surface(k: f32, radius: f32, solid: bool) -> Div {
    let t = k.clamp(0., 1.);
    let grow = u(GROW * k.max(0.));
    let idle = kit::shade(0.48);
    let fill: Hsla = if solid { motion::mix(idle, theme::console_focus(), t).into() } else { idle };
    div()
        .absolute()
        .top(-grow)
        .bottom(-grow)
        .left(-grow)
        .right(-grow)
        .rounded(u(radius))
        .bg(fill)
        .border_1()
        .border_color(kit::ink(0.12 * (1. - t)))
        .when(t > 0.01, |d| d.shadow(kit::shadow(0.45 * t, 6. + 6. * t, 16. + 14. * t)))
}

/// A button of the actions row: an icon and maybe a label. `wide` is the
/// console home's long Play button. Focused, it takes the focus colour.
pub(super) fn button(
    id: (&'static str, usize),
    focused: bool,
    ic: Icon,
    label: Option<SharedString>,
    tint: Option<Rgba>,
    wide: bool,
) -> AnyElement {
    let has_label = label.is_some();
    let base = div().relative().flex_none().h(u(BUTTON_H)).when(wide, |d| d.min_w(u(230.))).when(!has_label, |d| d.w(u(BUTTON_H)));
    motion::spring(id, base, motion::SOFT, focused, move |d, k| {
        let t = k.clamp(0., 1.);
        let idle: Rgba = tint.unwrap_or(theme::text_primary());
        let colour = motion::mix(idle, theme::console_on_focus(), t);
        let content = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(u(10.))
            .when(has_label, |d| d.px(u(if wide { 34. } else { 24. })))
            .text_size(u(if wide { 19. } else { 17. }))
            .font_weight(theme::W_BOLD)
            .text_color(colour)
            .child(crate::ui::icon(ic, u(if wide { 24. } else { 22. }), colour))
            .children(label);
        d.child(surface(k, BUTTON_R, true)).child(content)
    })
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
        let ring: Hsla = theme::console_focus().into();
        let gap = u(5.);
        d.child(surface(k, CARD_R, false)).child(content).when(t > 0.01, |d| {
            d.child(
                div()
                    .absolute()
                    .top(-gap)
                    .left(-gap)
                    .right(-gap)
                    .bottom(-gap)
                    .rounded(u(CARD_R) + gap)
                    .border(u(2.5))
                    .border_color(Hsla { a: t, ..ring }),
            )
        })
    })
}

impl CollectionView {
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
