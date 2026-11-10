//! The right column: for the picked slot, the picture it shows now, then
//! the pictures SteamGridDB and the web found for it, each in the slot's
//! shape with its source's mark and size, a search field and "Load more".
//! A family that is not kept offers to keep it instead.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_data::source_prefs::ids;
use heronite_i18n::{tr, trn};
use heronite_images::ImageRef;
use heronite_metadata::WebImage;

use super::slots::{Look, Slot};
use super::{Busy, CoversDialog};
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::fields::search_box;
use crate::ui::icon;

/// Pictures listed before the first "Load more", and added by each.
pub(super) const PAGE: usize = 15;

/// Width of a found picture of each slot (its height follows the shape).
fn tile_w(slot: Slot) -> f32 {
    match slot {
        Slot::Cover(CoverFamily::Tall) => 104.,
        Slot::Cover(CoverFamily::Square) => 110.,
        Slot::Cover(CoverFamily::Wide) | Slot::Background => 186.,
    }
}

impl CoversDialog {
    /// Every picture found for the slot, SteamGridDB's first, with its source.
    pub(super) fn found(&self) -> Vec<(&'static str, &WebImage)> {
        let sgdb = self.results.steamgrid.iter().map(|i| (ids::STEAMGRIDDB, i));
        sgdb.chain(self.results.web.iter().map(|i| (ids::WEB_IMAGES, i))).collect()
    }

    fn sources_found(&self) -> usize {
        usize::from(!self.results.steamgrid.is_empty()) + usize::from(!self.results.web.is_empty())
    }

    /// The source's mark and a few words under a picture.
    fn tile_label(&self, source: Option<&str>, text: String) -> Div {
        let mark = source.map(|id| {
            let name = crate::dialogs::metadata_update::words::source_name(&self.service, id);
            labels::mark(id, &name, 16.)
        });
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .children(mark)
            .child(div().min_w_0().truncate().text_size(px(11.5)).text_color(theme::text_dim()).child(text))
    }

    fn tile_frame(id: SharedString, selected: bool) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(6.))
            .rounded(px(12.))
            .border_1()
            .border_color(if selected { theme::accent().into() } else { theme::ink_a(0.06) })
            .bg(if selected { theme::accent_alpha(0.12).into() } else { theme::ink_a(0.03) })
    }

    /// The picture the slot shows now, first in the grid.
    fn now_tile(&mut self, w: f32, h: f32, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let info = self.info(self.slot)?;
        let (chain, look, from) = (info.chain.clone(), info.look, info.from.clone());
        if chain.is_empty() {
            return None;
        }
        let text = match look {
            Look::Cropped(_) => tr!("Cropped \u{b7} now").to_string(),
            _ => from.map(|f| tr!("{from} \u{b7} now", from = f)).unwrap_or_else(|| tr!("Now").to_string()),
        };
        let pic = self.picture_box("now", &chain, w, h, cx);
        Some(Self::tile_frame("cv-now".into(), false).w(px(w + 12.)).child(pic).child(self.tile_label(None, text)))
    }

    fn found_tile(&mut self, i: usize, source: &'static str, image: WebImage, w: f32, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.selected == Some(i);
        let pic = self.picture_box("found", &[ImageRef::Url(image.thumb_url.clone())], w, h, cx);
        let size = if image.width > 0 && image.height > 0 { format!("{}\u{d7}{}", image.width, image.height) } else { String::new() };
        Self::tile_frame(SharedString::from(format!("cv-found-{i}")), selected)
            .w(px(w + 12.))
            .cursor_pointer()
            .when(!selected, |d| d.hover(|s| s.border_color(theme::ink_a(0.3))))
            .child(pic)
            .child(self.tile_label(Some(source), size))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if this.busy.is_some() {
                    return;
                }
                this.selected = Some(i);
                if matches!(ev, ClickEvent::Mouse(m) if m.up.click_count >= 2) {
                    this.use_selected(Some(i), cx);
                }
                cx.notify();
            }))
    }

    /// "Square pictures found · 9 from 2 sources".
    fn found_title(&self) -> Div {
        let title = match self.slot {
            Slot::Cover(CoverFamily::Tall) => tr!("Tall pictures found"),
            Slot::Cover(CoverFamily::Square) => tr!("Square pictures found"),
            Slot::Cover(CoverFamily::Wide) => tr!("Wide pictures found"),
            Slot::Background => tr!("Backgrounds found"),
        };
        let n = self.found().len();
        let count = (!self.loading && n > 0).then(|| {
            let sources = trn!(self.sources_found(), "{n} source", "{n} sources");
            tr!("{n} from {sources}", n, sources)
        });
        div()
            .flex()
            .items_baseline()
            .gap(px(8.))
            .min_w_0()
            .child(div().flex_none().text_size(px(12.)).text_color(theme::text_muted()).child(title))
            .children(count.map(|c| div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(c)))
    }

    fn not_kept(&self, family: CoverFamily, cx: &mut Context<Self>) -> Div {
        let text = match family {
            CoverFamily::Wide => tr!(
                "No view on this PC draws wide covers, so updates do not download them. Keep the shape to give games a wide picture anyway; a view that starts using it keeps it too."
            ),
            _ => tr!(
                "No view on this PC draws square covers, so updates do not download them. Keep the shape to give games a square picture anyway; a view that starts using it keeps it too."
            ),
        };
        let title = match family {
            CoverFamily::Wide => tr!("Wide covers are not kept"),
            _ => tr!("Square covers are not kept"),
        };
        let busy = self.busy == Some(Busy::Keeping);
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .flex_1()
            .px(px(40.))
            .child(div().text_size(px(15.)).text_color(theme::text_primary()).child(title))
            .child(div().max_w(px(440.)).text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_muted()).text_center().child(text))
            .child(action_button(
                "cv-keep",
                ButtonKind::Primary,
                if busy { tr!("Saving\u{2026}") } else { tr!("Keep this shape") },
                !busy,
                cx.listener(move |this, _, _, cx| this.keep_family(family, cx)),
            ))
    }

    fn spinner(text: &'static str) -> Div {
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(9.6))
            .py(px(30.))
            .text_size(px(13.5))
            .text_color(theme::text_muted())
            .child(icon(Icon::FaSpinner, px(16.), theme::text_muted()).with_animation(
                "cv-spin",
                Animation::new(Duration::from_secs(1)).repeat().with_max_fps(30.),
                |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
            ))
            .child(text)
    }

    pub(super) fn render_found(&mut self, cx: &mut Context<Self>) -> Div {
        let col = div().flex().flex_col().gap(px(12.)).min_h_0();
        if let Slot::Cover(family) = self.slot {
            if !self.families.is_kept(family) {
                return col.child(self.not_kept(family, cx));
            }
        }
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(self.found_title())
            .child(div().w(px(260.)).flex_none().child(search_box(&self.query)));
        let w = tile_w(self.slot);
        let h = (w / self.slot.aspect()).round();
        let mut tiles = div().flex().flex_wrap().gap(px(8.));
        tiles = tiles.children(self.now_tile(w, h, cx));
        let found: Vec<(&'static str, WebImage)> = self.found().into_iter().map(|(s, i)| (s, i.clone())).collect();
        let total = found.len();
        if !self.loading {
            for (i, (source, image)) in found.into_iter().take(self.visible).enumerate() {
                tiles = tiles.child(self.found_tile(i, source, image, w, h, cx));
            }
        }
        let mut grid = div().id("cv-grid").flex().flex_col().gap(px(10.)).flex_1().min_h_0().overflow_y_scroll().pr(px(4.)).child(tiles);
        if self.loading {
            grid = grid.child(Self::spinner(tr!("Searching\u{2026}")));
        } else if total == 0 && self.error.is_empty() {
            grid = grid.child(
                div()
                    .py(px(24.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(tr!("No pictures found. Try another search.")),
            );
        } else if self.visible < total {
            grid = grid.child(div().flex().justify_center().pt(px(4.)).child(
                button("cv-more", ButtonKind::Glass, tr!("Load more")).min_w(px(180.)).on_click(cx.listener(|this, _, _, cx| {
                    this.visible += PAGE;
                    cx.notify();
                })),
            ));
        }
        let error = (!self.error.is_empty()).then(|| {
            div()
                .px(px(14.))
                .py(px(10.))
                .rounded(px(theme::R_INPUT))
                .bg(theme::danger_fill())
                .text_size(px(13.))
                .text_color(theme::danger_text())
                .child(self.error.clone())
        });
        col.child(head).child(grid).children(error)
    }
}
