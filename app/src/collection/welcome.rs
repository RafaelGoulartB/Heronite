//! The Collection of a new setup (no games at all): rows of empty covers
//! fading out behind a card that offers where games come from, instead of an
//! empty "No game selected" panel. While it shows, Steam is checked every few
//! seconds so its games come in as soon as Steam opens. Also the screen shown
//! when the library cannot be loaded.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{n_, tr};
use heronite_steam::LibraryAccess;

use super::CollectionView;
use crate::icons::Icon;
use crate::pages::Page;
use crate::theme;
use crate::ui::controls::{button, kbd, kicker, ButtonKind};
use crate::ui::icon;

#[derive(Clone, Copy)]
enum Source {
    /// Epic Games, GOG and Amazon Games (their pages switch with tabs).
    Stores,
    Steam,
    Emulators,
    AddGame,
}

const SOURCES: [(Source, Icon, &str); 4] = [
    (Source::Stores, Icon::Storefront, n_!("Stores")),
    (Source::Steam, Icon::CloudSync, "Steam"),
    (Source::Emulators, Icon::SportsEsports, n_!("Emulators")),
    (Source::AddGame, Icon::Add, n_!("Add a game")),
];

/// The store chips of the Stores tile (brands: not translated).
const STORES: [(Page, &str); 3] = [(Page::Epic, "Epic"), (Page::Gog, "GOG"), (Page::Amazon, "Amazon")];

/// How often Steam is checked while the welcome shows.
const STEAM_POLL: Duration = Duration::from_secs(4);

/// Empty covers: 2/3 like the real ones, a little smaller on each lower row
/// so the shelves seem to recede.
const SHELF_W: f32 = 132.;
const SHELF_GAP: f32 = 22.;
const SHELF_SHRINK: f32 = 0.06;

#[derive(Default)]
pub(super) struct Welcome {
    /// The Steam check of an empty Collection (stops once games come in).
    watch: Option<(Task<()>, Subscription)>,
    /// The first load failed: shown instead of the Collection.
    pub load_error: Option<String>,
}

impl CollectionView {
    /// Checks Steam every few seconds while the Collection is empty and
    /// reads its library as soon as it can (the service alone waits ten
    /// minutes between reads). Off with `HERONITE_DISABLE_IDLE_JOBS`.
    pub(super) fn start_welcome_watch(&mut self, cx: &mut Context<Self>) {
        if self.welcome.watch.is_some() || crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") {
            return;
        }
        let steam = crate::services::steam_library(cx);
        let sub = cx.observe(&steam, |_, _, cx| cx.notify());
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STEAM_POLL).await;
                let empty = this.update(cx, |this, cx| {
                    let empty = this.lib.as_ref().is_some_and(|lib| lib.games.is_empty());
                    if empty {
                        // Read once it turns ready; afterwards only watch the state.
                        let ready = steam.read(cx).access == Some(LibraryAccess::Ready);
                        steam.update(cx, |s, cx| if ready { s.check(cx) } else { s.sync_if_ready(cx) });
                    }
                    empty
                });
                if !matches!(empty, Ok(true)) {
                    break;
                }
            }
            this.update(cx, |this, _| this.welcome.watch = None).ok();
        });
        self.welcome.watch = Some((task, sub));
    }

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

    /// The note under Steam: what is missing to read the library, if anything.
    fn steam_note(cx: &App) -> (&'static str, Rgba) {
        let auto = (tr!("Automatic while Steam is open"), theme::success());
        let Some(svc) = crate::services::steam_library_started(cx) else { return auto };
        let s = svc.read(cx);
        if s.syncing {
            return (tr!("Reading your Steam library\u{2026}"), theme::accent());
        }
        match s.access {
            Some(LibraryAccess::Closed) => (tr!("Open Steam to bring in your games"), theme::accent()),
            Some(LibraryAccess::NeedsRestart) => (tr!("Restart Steam once to bring in your games"), theme::danger_text()),
            Some(LibraryAccess::NotInstalled) => (tr!("Steam was not found on this PC"), theme::text_dim()),
            _ => auto,
        }
    }

    fn store_chips(&self, cx: &mut Context<Self>) -> Div {
        div().flex().flex_wrap().justify_center().gap(px(6.)).children(STORES.iter().map(|&(page, name)| {
            div()
                .id(SharedString::from(format!("welcome-store-{name}")))
                .px(px(9.))
                .py(px(2.))
                .rounded_full()
                .bg(theme::white_a(0.06))
                .border_1()
                .border_color(theme::white_a(0.1))
                .text_size(px(12.))
                .text_color(theme::text_muted())
                .cursor_pointer()
                .hover(|s| s.bg(theme::accent_tint()).border_color(theme::accent_tint_border()).text_color(theme::text_primary()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.navigate(page, window, cx);
                }))
                .child(name)
        }))
    }

    fn welcome_card(&self, cx: &mut Context<Self>) -> Div {
        let tiles: Vec<_> = SOURCES
            .iter()
            .map(|&(source, ic, title)| {
                let note = match source {
                    Source::Stores => self.store_chips(cx),
                    Source::Steam => {
                        let (text, color) = Self::steam_note(cx);
                        div().text_center().text_size(px(12.)).text_color(color).child(text)
                    }
                    Source::Emulators => {
                        div().text_center().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Find emulators and ROMs"))
                    }
                    Source::AddGame => {
                        div().text_center().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("A program, web game or ROM"))
                    }
                };
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
                    .when(matches!(source, Source::Steam), |d| d.border_dashed())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        match source {
                            Source::Stores => {
                                let page = this.pages.stores_page();
                                this.navigate(page, window, cx);
                            }
                            // Its page says what Steam needs, and reads the library now.
                            Source::Steam => this.navigate(Page::Steam, window, cx),
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
                    .child(note)
            })
            .collect();
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
        let palette = div()
            .id("welcome-palette")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .hover(|s| s.text_color(theme::text_primary()))
            .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx)))
            .child(kbd("Ctrl K"))
            .child(tr!("to search anything"));
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
                    .child(tr!("Let's fill your shelves")),
            )
            .child(
                div()
                    .mt(px(8.))
                    .text_size(px(16.))
                    .text_color(theme::text_muted())
                    .child(tr!("Pick where your games come from. You can add more sources later.")),
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
                    .child(palette),
            )
    }

    /// The first load failed (a damaged file in the data folder, a disk
    /// error): what happened and a way to try again, instead of an empty list.
    pub(super) fn render_load_error(&self, err: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(32.))
            .child(
                div()
                    .w(px(560.))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .px(px(40.))
                    .py(px(36.))
                    .rounded(px(24.))
                    .bg(theme::rgb_a(14, 18, 24, 0.96))
                    .border_1()
                    .border_color(theme::white_a(0.1))
                    .child(icon(Icon::Warning, px(36.), theme::danger_text()))
                    .child(
                        div()
                            .font_family(theme::FONT_HEADING)
                            .text_size(px(24.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .child(tr!("Your library could not be loaded")),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(14.))
                            .text_color(theme::text_muted())
                            .child(tr!("A file in the data folder could not be read. Your games and their files were not touched.")),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .max_w_full()
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .bg(theme::white_a(0.05))
                            .font_family(theme::FONT_MONO)
                            .text_size(px(12.))
                            .text_color(theme::text_dim())
                            .child(err),
                    )
                    .child(div().mt(px(8.)).child(button("load-retry", ButtonKind::Primary, tr!("Try again")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.reload(cx);
                        },
                    )))),
            )
            .into_any_element()
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
