//! The Collection of a new setup (no games at all): rows of empty covers
//! fading out behind a card that offers where games come from, instead of an
//! empty "No game selected" panel.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{n_, tr};

use super::CollectionView;
use crate::icons::Icon;
use crate::pages::Page;
use crate::theme;
use crate::ui::controls::{kbd, kicker};
use crate::ui::icon;

#[derive(Clone, Copy)]
enum Source {
    /// Epic Games, GOG and Amazon Games (their pages switch with tabs).
    Stores,
    Steam,
    Emulators,
    AddGame,
}

const SOURCES: [(Source, Icon, &str, &str); 4] = [
    (Source::Stores, Icon::Storefront, n_!("Stores"), "Epic \u{b7} GOG \u{b7} Amazon \u{b7} Steam"),
    (Source::Steam, Icon::CloudSync, "Steam", n_!("Automatic")),
    (Source::Emulators, Icon::SportsEsports, n_!("Emulators"), n_!("Detect & scan ROMs")),
    (Source::AddGame, Icon::Add, n_!("Add a game"), n_!("Program, browser or ROM")),
];

/// Empty covers: 2/3 like the real ones, a little smaller on each lower row
/// so the shelves seem to recede.
const SHELF_W: f32 = 132.;
const SHELF_GAP: f32 = 22.;
const SHELF_SHRINK: f32 = 0.06;

impl CollectionView {
    pub(super) fn render_welcome(&self, vw: f32, vh: f32, cx: &mut Context<Self>) -> AnyElement {
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(shelves(vw, vh))
            .child(vignette(vh))
            .child(div().absolute().inset_0().flex().items_center().justify_center().p(px(32.)).child(self.welcome_card(cx)))
            .into_any_element()
    }

    fn welcome_card(&self, cx: &mut Context<Self>) -> Div {
        let tiles = SOURCES.iter().map(|&(source, ic, title, note)| {
            let auto = matches!(source, Source::Steam);
            div()
                .id(SharedString::from(format!("welcome-{title}")))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .pt(px(22.))
                .pb(px(18.))
                .px(px(10.))
                .rounded(px(18.))
                .bg(theme::card_fill())
                .border_1()
                .border_color(theme::card_border())
                .when(auto, |d| d.border_dashed())
                .cursor_pointer()
                .hover(|s| s.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    match source {
                        Source::Stores => {
                            let page = this.pages.stores_page();
                            this.navigate(page, window, cx);
                        }
                        // Steam games are found on their own; a click looks again now.
                        Source::Steam => this.refresh_all(cx),
                        Source::Emulators => this.navigate(Page::Emulators, window, cx),
                        Source::AddGame => {
                            this.open_add_game(window, cx);
                        }
                    }
                    cx.notify();
                }))
                .child(icon(ic, px(32.), theme::accent()))
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(px(15.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_primary())
                        .child(heronite_i18n::tr(title)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(if auto { theme::success() } else { theme::text_dim() })
                        .child(heronite_i18n::tr(note)),
                )
        });
        let proton = div()
            .id("welcome-proton")
            .text_color(theme::accent())
            .cursor_pointer()
            .hover(|s| s.underline())
            .on_click(cx.listener(|this, _, window, cx| {
                this.navigate(Page::WineManager, window, cx);
                cx.notify();
            }))
            .child(tr!("Download Proton"));
        div()
            .w(px(760.))
            .max_w_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(46.))
            .py(px(42.))
            .rounded(px(28.))
            .bg(theme::rgb_a(14, 18, 24, 0.96))
            .border_1()
            .border_color(theme::white_a(0.1))
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.55),
                offset: point(px(0.), px(30.)),
                blur_radius: px(90.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(kicker(tr!("Welcome to Heronite")))
            .child(
                div()
                    .mt(px(12.))
                    .font_family(theme::FONT_HEADING)
                    .text_size(px(38.))
                    .line_height(px(46.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(tr!("Your shelves are empty \u{2014} for now")),
            )
            .child(
                div()
                    .mt(px(8.))
                    .text_size(px(16.))
                    .text_color(theme::text_muted())
                    .child(tr!("Choose where your games come from. You can add more sources at any time.")),
            )
            .child(div().mt(px(30.)).w_full().flex().gap(px(12.)).children(tiles))
            .child(
                div()
                    .mt(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(13.5))
                    .text_color(theme::text_muted())
                    .child(tr!("Playing Windows games?"))
                    .child(proton)
                    .child("\u{00b7}")
                    .child(kbd("Ctrl K"))
                    .child(tr!("finds anything")),
            )
    }
}

/// Rows of empty covers filling the area, each a bit smaller than the one
/// above; they fade in row by row once (no animation keeps running).
fn shelves(vw: f32, vh: f32) -> Div {
    let mut rows = Vec::new();
    let (mut y, mut r) = (24.0_f32, 0_usize);
    while y < vh {
        let w = SHELF_W * (1. - SHELF_SHRINK * r as f32).max(0.6);
        let (h, gap) = (w * 1.5, SHELF_GAP * w / SHELF_W);
        let count = (vw / (w + gap)).ceil() as usize + 1;
        let row = div().flex().justify_center().gap(px(gap)).children((0..count).map(move |_| {
            div().flex_none().w(px(w)).h(px(h)).rounded(px(10.)).bg(theme::white_a(0.05)).border_1().border_color(theme::white_a(0.09))
        }));
        let row = if theme::animations() {
            let delay = r as f32 * 0.12;
            row.with_animation(
                ElementId::Name(format!("welcome-shelf-{r}").into()),
                Animation::new(Duration::from_millis(1400)).with_easing(ease_in_out),
                move |el, t| el.opacity(((t - delay) / (1. - delay).max(0.2)).clamp(0., 1.)),
            )
            .into_any_element()
        } else {
            row.into_any_element()
        };
        rows.push(row);
        y += h + gap;
        r += 1;
    }
    div().absolute().inset_0().flex().flex_col().items_center().gap(px(SHELF_GAP)).pt(px(24.)).children(rows)
}

/// Darkens the edges so the shelves fade into the page around the card.
fn vignette(vh: f32) -> Div {
    let bg = theme::page_bg();
    let edge = |angle: f32| linear_gradient(angle, linear_color_stop(bg, 0.), linear_color_stop(theme::rgb_a(7, 9, 12, 0.), 1.));
    let band = (vh * 0.22).max(120.);
    div()
        .absolute()
        .inset_0()
        .child(div().absolute().top_0().left_0().right_0().h(px(band)).bg(edge(180.)))
        .child(div().absolute().bottom_0().left_0().right_0().h(px(band)).bg(edge(0.)))
        .child(div().absolute().top_0().bottom_0().left_0().w(relative(0.16)).bg(edge(90.)))
        .child(div().absolute().top_0().bottom_0().right_0().w(relative(0.16)).bg(edge(270.)))
}
