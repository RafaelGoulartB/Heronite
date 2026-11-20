//! The layout of a store Heronite reads in two ways (Steam, Xbox): a strip
//! with what the library holds, then two cards side by side.
//!
//! - Method 1, **local**: what is on this PC, with no login (the Steam
//!   client signed in already, the Xbox app's packages).
//! - Method 2, **online**, optional: the store account, after a sign-in.
//!
//! Each card says which kind it is (its eyebrow and first pill), what it
//! brings and what it needs (its facts), what is wrong or what to do now
//! (its notice) and the buttons for it. Narrow windows stack the cards.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The colour of a pill or a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Accent,
    Warning,
}

impl Tone {
    fn color(self) -> Rgba {
        match self {
            Tone::Neutral => theme::text_muted(),
            Tone::Good => theme::success(),
            Tone::Accent => theme::accent(),
            Tone::Warning => theme::warning(),
        }
    }
}

/// One line of what a method brings (`good`) or needs.
pub struct Fact {
    good: bool,
    text: SharedString,
}

/// Something the method brings.
pub fn brings(text: impl Into<SharedString>) -> Fact {
    Fact { good: true, text: text.into() }
}

/// Something the method needs, or leaves out.
pub fn needs(text: impl Into<SharedString>) -> Fact {
    Fact { good: false, text: text.into() }
}

/// What a method card shows.
pub struct Method {
    /// [`local_eyebrow`] or [`online_eyebrow`].
    pub eyebrow: &'static str,
    pub icon: Icon,
    pub title: SharedString,
    /// What the method asks of the user ("No login needed").
    pub badge: (SharedString, Tone),
    /// Where it stands now ("Port in use", "Signed in as …").
    pub status: Option<(SharedString, Tone)>,
    pub facts: Vec<Fact>,
    /// What is wrong, or what to do now.
    pub notice: Option<(String, Tone)>,
    /// More content after the notice (a sign-in form).
    pub body: Vec<AnyElement>,
    /// Buttons, at the bottom of the card.
    pub actions: Vec<AnyElement>,
    /// The outcome of the last thing asked here (`true`: an error).
    pub message: Option<(String, bool)>,
    /// A second way to do the same, under the buttons (the sign-in through
    /// the browser).
    pub footer: Vec<AnyElement>,
}

pub fn local_eyebrow() -> &'static str {
    tr!("Method 1 \u{b7} Local")
}

pub fn online_eyebrow() -> &'static str {
    tr!("Method 2 \u{b7} Online \u{b7} Optional")
}

/// The first pill of a local method.
pub fn no_login() -> (SharedString, Tone) {
    (tr!("No login needed").into(), Tone::Good)
}

/// A pill: `color` text on a tint of it.
pub fn pill(text: impl Into<SharedString>, tone: Tone) -> Div {
    let c: Hsla = tone.color().into();
    div()
        .flex_none()
        .px(px(10.))
        .py(px(3.))
        .rounded_full()
        .bg(Hsla { a: 0.16, ..c })
        .text_size(px(12.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::readable(tone.color()))
        .whitespace_nowrap()
        .child(text.into())
}

/// A tinted box with a short text: what is wrong, or what to do.
pub fn notice(text: impl Into<SharedString>, tone: Tone) -> Div {
    let c: Hsla = tone.color().into();
    div()
        .px(px(14.))
        .py(px(12.))
        .rounded(px(12.))
        .bg(Hsla { a: 0.08, ..c })
        .border_1()
        .border_color(Hsla { a: 0.35, ..c })
        .text_size(px(13.))
        .line_height(px(19.))
        .text_color(theme::text_primary())
        .child(text.into())
}

fn fact_row(f: Fact) -> Div {
    let mark = if f.good {
        icon(Icon::Check, px(16.), theme::success()).into_any_element()
    } else {
        div()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(5.)).rounded_full().bg(theme::text_muted()))
            .into_any_element()
    };
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .text_size(px(13.))
        .line_height(px(19.))
        .text_color(theme::text_muted())
        .child(div().flex_none().mt(px(1.5)).child(mark))
        .child(div().flex_1().min_w_0().child(f.text))
}

/// A method's card.
pub fn method_card(m: Method) -> Div {
    let header = div()
        .flex()
        .items_start()
        .gap(px(14.))
        .child(
            div().flex_none().size(px(48.)).rounded(px(14.)).bg(theme::accent_tint()).flex().items_center().justify_center().child(icon(
                m.icon,
                px(26.),
                theme::accent(),
            )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .child(div().text_size(px(11.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(m.eyebrow.to_uppercase()))
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .text_size(px(20.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .child(m.title),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(pill(m.badge.0, m.badge.1))
                        .children(m.status.map(|(text, tone)| pill(text, tone))),
                ),
        );
    let message = m.message.map(|(text, error)| {
        div()
            .text_size(px(13.))
            .line_height(px(19.))
            .text_color(if error { theme::danger_text() } else { theme::text_primary() })
            .child(text)
    });
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .p(px(22.))
        .rounded(px(18.))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
        .child(header)
        .child(div().flex().flex_col().gap(px(8.)).children(m.facts.into_iter().map(fact_row)))
        .children(m.notice.map(|(text, tone)| notice(text, tone)))
        .children(m.body)
        // The buttons sit at the bottom, so those of both cards line up.
        .child(div().flex_1())
        .when(!m.actions.is_empty(), |d| d.child(div().flex().flex_wrap().gap(px(10.)).children(m.actions)))
        .children(message)
        .children(m.footer)
}

/// The strip above the cards: what the library holds, the outcome of the
/// last read asked for, and a read button.
pub fn library_strip(summary: impl Into<SharedString>, message: Option<(String, bool)>, action: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(18.))
        .py(px(12.))
        .rounded(px(14.))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
        .child(icon(Icon::Collections, px(22.), theme::accent()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("Your library")))
                .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(summary.into()))
                .children(message.map(|(text, error)| {
                    div().text_size(px(13.)).text_color(if error { theme::danger_text() } else { theme::text_primary() }).child(text)
                })),
        )
        .child(div().flex_none().child(action))
}

/// The two cards: side by side and as tall as each other when `wide`,
/// stacked otherwise.
pub fn methods(wide: bool, local: Div, online: Div) -> Div {
    let card = |d: Div| d.min_w_0().when(wide, |d| d.flex_basis(px(0.)).flex_grow(1.));
    div()
        .flex()
        .w_full()
        .gap(px(24.))
        .when(wide, |d| d.items_stretch())
        .when(!wide, |d| d.flex_col())
        .child(card(local))
        .child(card(online))
}

/// A muted note under the cards.
pub fn footnote(text: impl Into<SharedString>) -> Div {
    div().mx(px(6.)).text_size(px(12.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.into())
}

/// The page: the strip, the cards and what follows them.
pub fn page(strip: Div, cards: Div, rest: impl IntoIterator<Item = AnyElement>) -> Div {
    div().flex().flex_col().gap(px(20.)).w_full().child(strip).child(cards).children(rest)
}
