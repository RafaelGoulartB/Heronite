//! The right column: for the picked slot, the picture it shows now, then
//! the pictures LaunchBox (the game's boxes of every region), SteamGridDB
//! and the web found for it, each in the slot's shape with its source's
//! mark, its region and its size, a search field and "Load more". A family
//! that is not kept offers to keep it instead.

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

const LAUNCHBOX: &str = heronite_metadata::launchbox::SOURCE;

/// A found picture as a tile draws it: its source, region and picture.
type Tile = (&'static str, String, WebImage);

/// Width of a found picture of each slot (its height follows the shape).
fn tile_w(slot: Slot) -> f32 {
    match slot {
        Slot::Cover(CoverFamily::Tall) => 104.,
        Slot::Cover(CoverFamily::Square) => 110.,
        Slot::Cover(CoverFamily::Wide) | Slot::Background => 186.,
    }
}

/// A picture found: its source, the region it is of (LaunchBox's box art;
/// `""` for none) and the picture.
pub(super) struct Found<'a> {
    pub(super) source: &'static str,
    pub(super) region: &'a str,
    pub(super) image: &'a WebImage,
}

impl CoversDialog {
    /// Every picture found for the slot: the game's boxes from LaunchBox
    /// first, then SteamGridDB's and the web's.
    pub(super) fn found(&self) -> Vec<Found<'_>> {
        let r = &self.results;
        let launchbox = r.launchbox.iter().map(|c| Found { source: LAUNCHBOX, region: &c.region, image: &c.image });
        let sgdb = r.steamgrid.iter().map(|image| Found { source: ids::STEAMGRIDDB, region: "", image });
        let web = r.web.iter().map(|image| Found { source: ids::WEB_IMAGES, region: "", image });
        launchbox.chain(sgdb).chain(web).collect()
    }

    /// The sources that found pictures, in the grid's order, and how many each.
    fn source_counts(&self) -> Vec<(&'static str, usize)> {
        let r = &self.results;
        [(LAUNCHBOX, r.launchbox.len()), (ids::STEAMGRIDDB, r.steamgrid.len()), (ids::WEB_IMAGES, r.web.len())]
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    fn sources_found(&self) -> usize {
        self.source_counts().len()
    }

    /// The source listed alone, while it found pictures (a new search may
    /// find none there: then every source is).
    fn source_shown(&self) -> Option<&'static str> {
        self.source.filter(|id| self.source_counts().iter().any(|(s, _)| s == id))
    }

    /// Lists the pictures of `source` alone (`None`: of every source). A
    /// picture picked of another source is no longer picked.
    fn show_source(&mut self, source: Option<&'static str>, cx: &mut Context<Self>) {
        self.source = source;
        self.visible = PAGE;
        if let (Some(id), Some(i)) = (source, self.selected) {
            if self.found().get(i).is_some_and(|f| f.source != id) {
                self.selected = None;
            }
        }
        cx.notify();
    }

    /// "All 90", then a pill per source that found pictures, with its mark
    /// and count; the one listed is lit. None while one source alone found.
    fn source_pills(&self, cx: &mut Context<Self>) -> Option<Div> {
        let counts = self.source_counts();
        if self.loading || counts.len() < 2 {
            return None;
        }
        let shown = self.source_shown();
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        let pill = |id: SharedString, on: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .border_1()
                .text_size(px(12.))
                .whitespace_nowrap()
                .cursor_pointer()
                .when(on, |d| d.bg(theme::accent_tint()).border_color(theme::accent_tint_border()).text_color(theme::text_primary()))
                .when(!on, |d| {
                    d.border_color(theme::ink_a(0.08))
                        .text_color(theme::text_muted())
                        .hover(|s| s.bg(theme::ink_a(0.06)).text_color(theme::text_bright()))
                })
        };
        let count = |n: usize| div().text_color(theme::text_dim()).child(n.to_string());
        let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(
            pill("cv-source-all".into(), shown.is_none())
                .child(tr!("All"))
                .child(count(total))
                .on_click(cx.listener(|this, _, _, cx| this.show_source(None, cx))),
        );
        for (id, n) in counts {
            let name = crate::dialogs::metadata_update::words::source_name(&self.service, id);
            row = row.child(
                pill(SharedString::from(format!("cv-source-{id}")), shown == Some(id))
                    .child(labels::mark(id, &name, 16.))
                    .child(name)
                    .child(count(n))
                    .on_click(cx.listener(move |this, _, _, cx| this.show_source(Some(id), cx))),
            );
        }
        Some(row)
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

    fn found_tile(&mut self, i: usize, (source, region, image): Tile, w: f32, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.selected == Some(i);
        let pic = self.picture_box("found", &[ImageRef::Url(image.thumb_url.clone())], w, h, cx);
        let size = (image.width > 0 && image.height > 0).then(|| format!("{}\u{d7}{}", image.width, image.height));
        // LaunchBox's region names are English; the ones Settings offers are translated.
        let region = (!region.is_empty()).then(|| heronite_i18n::tr(&region).to_owned());
        let size = region.into_iter().chain(size).collect::<Vec<_>>().join(" \u{b7} ");
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
        let shown = self.source_shown();
        let mut tiles = div().flex().flex_wrap().gap(px(8.));
        if shown.is_none() {
            tiles = tiles.children(self.now_tile(w, h, cx));
        }
        // Each keeps its index of every picture found, which `selected` is.
        let found: Vec<(usize, Tile)> = self
            .found()
            .into_iter()
            .enumerate()
            .filter(|(_, f)| shown.is_none_or(|id| f.source == id))
            .map(|(i, f)| (i, (f.source, f.region.to_owned(), f.image.clone())))
            .collect();
        let total = found.len();
        if !self.loading {
            for (i, tile) in found.into_iter().take(self.visible) {
                tiles = tiles.child(self.found_tile(i, tile, w, h, cx));
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
        let pills = self.source_pills(cx);
        col.child(head).children(pills).child(grid).children(error)
    }
}
