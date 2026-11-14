//! The pane beside the list: what the selected item is. A game shows its
//! cover and its facts (store, installed or running, playtime, last played,
//! status); a page or an action its icon, what it does and its shortcut.
//! The game rows' small covers come from the same image store.

use std::collections::HashMap;
use std::rc::Rc;

use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{AppPaths, DataPaths};
use heronite_i18n::tr;
use heronite_images::{ImageRef, Variant};

use super::super::sources::{Entry, GameBits, GameCard, Kind};
use super::PaletteView;
use crate::icons::Icon;
use crate::images::ImageStore;
use crate::theme;
use crate::ui::controls::kbd;
use crate::ui::icon;

/// Width of the pane; the panel is this much wider when it shows.
pub(super) const PANE_W: f32 = 268.;
/// A game row's cover.
pub(super) const THUMB_W: f32 = 22.;
pub(super) const THUMB_H: f32 = 30.;
const COVER_W: f32 = 112.;
const COVER_H: f32 = 168.;

/// The listed games' covers, loaded at the sizes they are drawn.
pub(super) struct Covers {
    /// Each game's cover chain (`art::card_cover`), by key.
    chains: Rc<HashMap<SharedString, Vec<ImageRef>>>,
    data_paths: DataPaths,
    /// Made on the first cover asked for.
    store: Option<ImageStore>,
}

impl Covers {
    pub fn new(chains: Rc<HashMap<SharedString, Vec<ImageRef>>>, data_paths: DataPaths) -> Self {
        Self { chains, data_paths, store: None }
    }
}

impl PaletteView {
    /// The cover of the game `key` at `w`×`h` logical px; `None` while it
    /// loads, or when the game has none.
    pub(super) fn cover(&mut self, key: &str, w: f32, h: f32, scale: f32, priority: i32, cx: &mut Context<Self>) -> Option<ImageSource> {
        let chain = self.covers.chains.get(key)?.clone();
        if self.covers.store.is_none() {
            let paths = &self.covers.data_paths;
            let (store, mut rx) = ImageStore::new(paths.images_cache_dir(), &AppPaths::for_data(paths));
            cx.spawn(async move |this, cx| {
                while let Some(first) = rx.next().await {
                    let mut batch = vec![first];
                    while let Ok(more) = rx.try_recv() {
                        batch.push(more);
                    }
                    let alive = this
                        .update(cx, |this, cx| {
                            if let Some(store) = &mut this.covers.store {
                                for item in batch {
                                    store.on_loaded(item);
                                }
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            })
            .detach();
            self.covers.store = Some(store);
        }
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("pal:{key}:{pw}x{ph}").into();
        let store = self.covers.store.as_mut()?;
        store.get_or_request(&id, || chain, pw, ph, Variant::Cover, priority).map(ImageSource::Render)
    }

    /// Frees the GPU memory of covers the store let go (from `render`).
    pub(super) fn drop_evicted_covers(&mut self, window: &mut Window) {
        if let Some(store) = &mut self.covers.store {
            store.drop_evicted(window);
        }
    }

    /// The pane for the selected item (empty when nothing is listed).
    pub(super) fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let pane = div()
            .flex_none()
            .w(px(PANE_W))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(20.))
            .py(px(20.))
            .overflow_hidden()
            .border_l_1()
            .border_color(theme::ink_a(0.08))
            .bg(theme::ink_a(0.025));
        let Some(e) = self.selected_entry().cloned() else { return pane };
        match e.game.as_ref().and_then(|b| b.card.clone().map(|c| (b.clone(), c))) {
            Some((bits, card)) => self.game_preview(pane, &bits, &card, window, cx),
            None => self.item_preview(pane, &e),
        }
    }

    fn game_preview(&mut self, pane: Div, bits: &GameBits, card: &GameCard, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let cover = self.cover(&bits.key, COVER_W, COVER_H, window.scale_factor(), 1000, cx);
        let (state, state_color) = if bits.playing {
            (tr!("Playing now"), theme::accent())
        } else if bits.installed {
            (tr!("Installed"), theme::success())
        } else {
            (tr!("Not installed"), theme::text_dim())
        };
        let last = card.last_played.clone().unwrap_or_else(|| tr!("Never").into());
        pane.child(
            div()
                .flex_none()
                .relative()
                .w(px(COVER_W))
                .h(px(COVER_H))
                .rounded(px(8.))
                .overflow_hidden()
                .bg(theme::ink_a(0.06))
                .shadow_md()
                .flex()
                .items_center()
                .justify_center()
                .when(cover.is_none(), |d| d.child(icon(Icon::SportsEsports, px(32.), theme::text_dim())))
                .children(cover.map(|src| img(src).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
                .when(bits.favourite, |d| {
                    d.child(div().absolute().top(px(6.)).right(px(6.)).p(px(4.)).rounded_full().bg(theme::black_a(0.55)).child(icon(
                        Icon::Favorite,
                        px(12.),
                        theme::accent(),
                    )))
                }),
        )
        .child(
            div()
                .mt(px(14.))
                .w_full()
                .max_h(px(42.))
                .overflow_hidden()
                .text_center()
                .text_size(px(15.))
                .line_height(px(21.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_primary())
                .child(card.title.clone()),
        )
        .child(
            div()
                .mt(px(8.))
                .flex()
                .flex_wrap()
                .justify_center()
                .gap(px(6.))
                .child(chip(card.source.clone(), theme::text_muted().into()))
                .child(chip(state.into(), state_color.into())),
        )
        .child(
            div()
                .mt(px(16.))
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(fact(tr!("Played"), div().child(card.playtime.clone())))
                .child(fact(tr!("Last played"), div().child(last)))
                .children(card.status.as_ref().map(|(name, slug)| {
                    fact(
                        tr!("Status"),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().size(px(7.)).rounded_full().bg(theme::status_color(slug)))
                            .child(name.clone()),
                    )
                })),
        )
    }

    fn item_preview(&self, pane: Div, e: &Entry) -> Div {
        let kind = match e.kind {
            Kind::Page if e.secondary => tr!("Settings"),
            Kind::Page => tr!("Page"),
            Kind::Action => tr!("Action"),
            Kind::Game => tr!("Game"),
        };
        let off = e.disabled.is_some();
        pane.justify_center()
            .child(
                div()
                    .flex_none()
                    .size(px(64.))
                    .rounded(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if off { theme::ink_a(0.06) } else { theme::accent_tint() })
                    .child(icon(e.icon, px(30.), if off { theme::text_dim() } else { theme::accent() })),
            )
            .child(div().mt(px(14.)).text_size(px(11.)).text_color(theme::text_dim()).child(kind.to_uppercase()))
            .child(
                div()
                    .mt(px(4.))
                    .w_full()
                    .text_center()
                    .text_size(px(15.))
                    .line_height(px(21.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_primary())
                    .child(e.title.clone()),
            )
            .when(!e.subtitle.is_empty(), |d| {
                d.child(
                    div()
                        .mt(px(6.))
                        .w_full()
                        .text_center()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(theme::text_muted())
                        .child(e.subtitle.clone()),
                )
            })
            .children(
                e.disabled
                    .clone()
                    .map(|why| div().mt(px(10.)).w_full().text_center().text_size(px(12.)).text_color(theme::warning()).child(why)),
            )
            .children(e.shortcut.map(|s| {
                div()
                    .mt(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(theme::text_dim())
                    .child(tr!("Shortcut"))
                    .child(kbd(s))
            }))
    }
}

fn chip(text: SharedString, color: Hsla) -> Div {
    div().px(px(8.)).py(px(2.)).rounded_full().bg(theme::ink_a(0.07)).text_size(px(11.5)).text_color(color).child(text)
}

/// A line of facts: the label on the left, the value on the right.
fn fact(label: &'static str, value: Div) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .text_size(px(12.5))
        .child(div().text_color(theme::text_dim()).child(label))
        .child(value.min_w_0().truncate().text_color(theme::text_primary()))
}
