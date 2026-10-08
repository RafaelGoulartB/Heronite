//! The "Screenshots" strip in the focus panel and the gallery modal. The
//! fullscreen viewer lives in `screenshot_viewer.rs`.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{GALLERY_PAGE, GRID_GAP, PREVIEW_SLOTS, ROW_H};
use crate::collection::{focus, CollectionView};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::icon;

impl CollectionView {
    /// "Screenshots" section of the focus panel (none without screenshots).
    pub(in crate::collection) fn screenshots_section(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.shots.items.len();
        if n == 0 {
            return None;
        }
        let has_more = n > PREVIEW_SLOTS;
        let visible = if has_more { PREVIEW_SLOTS - 1 } else { n };
        let remaining = n - visible;
        let mut row = div().flex().justify_start().gap(px(8.)).h(px(89.6)).overflow_hidden();
        for i in 0..visible {
            let path = self.shots.items[i].path.clone();
            let src = self.shots.thumb(&path);
            row = row.child(
                thumb_tile(SharedString::from(format!("shot-strip-{i}")), src, true)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(i, window, cx))),
            );
        }
        if remaining > 0 {
            let path = self.shots.items[visible].path.clone();
            let src = self.shots.thumb(&path);
            row = row.child(
                thumb_tile("shot-strip-more".into(), src, true)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme::rgb_a(8, 10, 14, 0.62))
                            .text_color(white())
                            .font_weight(theme::W_BOLD)
                            .text_size(px(16.8))
                            .child(format!("+{remaining}")),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.shots.gallery = true;
                        this.shots.gallery_count = GALLERY_PAGE;
                        this.shots.view = None;
                        window.focus(&this.focus_handle, cx);
                        cx.notify();
                    })),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .child(focus::h3("Screenshots"))
                .child(focus::panel(true).child(row))
                .into_any_element(),
        )
    }

    fn shots_open_view(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.shots.view = Some(i);
        self.shots.fit = false;
        self.shots.controls_visible = true;
        self.shots.error = None;
        self.shots.reset_transform();
        self.shots.discard_full();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // --- gallery -----------------------------------------------------------

    pub(in crate::collection) fn shots_close_gallery(&mut self) {
        self.shots.gallery = false;
        self.shots.gallery_count = GALLERY_PAGE;
        self.shots.view = None;
        self.shots.fit = false;
        self.shots.discard_full();
    }

    pub(in crate::collection) fn render_gallery(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shots.gallery || self.shots.items.is_empty() {
            return None;
        }
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let modal_w = 1440f32.min(vw - 64.);
        let max_h = (vh * 0.9).min(1100.);
        let inner = modal_w - 2. - 30.4 - 2.4;
        let col_min = 224f32.min(inner);
        let cols = (((inner + GRID_GAP) / (col_min + GRID_GAP)).floor() as usize).max(1);
        let col_w = (inner - (cols - 1) as f32 * GRID_GAP) / cols as f32;
        let total = self.shots.items.len();
        let shown = self.shots.gallery_count.min(total);

        // Rows near the viewport get their thumbnails.
        let scroll = self.shots.gallery_scroll.clone();
        let view_h = f32::from(scroll.bounds().size.height).max(max_h - 80.);
        let off = -f32::from(scroll.offset().y);
        let step = ROW_H + GRID_GAP;
        let first_row = ((off / step).floor() as isize - 1).max(0) as usize;
        let last_row = ((off + view_h) / step).floor() as usize + 1;

        let grid = self.gallery_grid(cols, col_w, shown, first_row..=last_row, cx);
        let header = Self::gallery_header(total, cx);

        let modal = div()
            .id("shot-gallery-modal")
            .occlude()
            .flex()
            .flex_col()
            .w(px(modal_w))
            .max_h(px(max_h))
            .pt(px(13.6))
            .px(px(15.2))
            .pb(px(16.))
            .rounded(px(12.))
            .bg(theme::rgb_a(18, 22, 28, 0.96))
            .border_1()
            .border_color(theme::white_a(0.1))
            .shadow(vec![BoxShadow {
                color: theme::black_a(0.45),
                offset: point(px(0.), px(24.)),
                blur_radius: px(60.),
                spread_radius: px(0.),
                inset: false,
            }])
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(
                div()
                    .id("shot-gallery-scroll")
                    .flex_1()
                    .min_h_0()
                    .pr(px(2.4))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(grid),
            );
        Some(
            deferred(
                div()
                    .id("shot-gallery")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(32.))
                    // Heronite blurs what is behind (16px); darker instead.
                    .bg(theme::rgb_a(4, 6, 8, 0.8))
                    .font_family(theme::FONT_BODY)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.shots_close_gallery();
                            cx.notify();
                        }),
                    )
                    .child(modal),
            )
            .with_priority(50)
            .into_any_element(),
        )
    }

    /// Thumbnails in rows of `cols`; only rows in `visible_rows` get textures.
    fn gallery_grid(
        &mut self,
        cols: usize,
        col_w: f32,
        shown: usize,
        visible_rows: std::ops::RangeInclusive<usize>,
        cx: &mut Context<Self>,
    ) -> Div {
        let total = self.shots.items.len();
        let mut grid = div().flex().flex_col().gap(px(GRID_GAP));
        for (r, chunk) in (0..shown).collect::<Vec<_>>().chunks(cols).enumerate() {
            let mut row = div().flex().gap(px(GRID_GAP)).h(px(ROW_H)).flex_none();
            for &i in chunk {
                let src = if visible_rows.contains(&r) {
                    let p = self.shots.items[i].path.clone();
                    self.shots.thumb(&p)
                } else {
                    None
                };
                row = row.child(
                    thumb_tile(SharedString::from(format!("shot-grid-{i}")), src, false)
                        .w(px(col_w))
                        .h(px(ROW_H))
                        .flex_none()
                        .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(i, window, cx))),
                );
            }
            grid = grid.child(row);
        }
        if shown < total {
            grid = grid.child(
                div().flex().justify_center().my(px(5.6)).child(
                    button(
                        "shot-load-more",
                        ButtonKind::Glass,
                        format!("Load more ({} remaining)", total - shown),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shots.gallery_count = (this.shots.gallery_count + GALLERY_PAGE).min(this.shots.items.len());
                        cx.notify();
                    })),
                ),
            );
        }
        grid
    }

    /// Title, count and close button.
    fn gallery_header(total: usize, cx: &mut Context<Self>) -> Div {
        let close = icon_button("shot-gallery-close", Icon::Close, 36., 0.42, 0.72)
            .on_click(cx.listener(|this, _, _, cx| {
                this.shots_close_gallery();
                cx.notify();
            }));
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .mb(px(12.))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.8))
                    .child(
                        div()
                            .text_size(px(16.8))
                            .font_weight(theme::W_BOLD)
                            .text_color(white())
                            .font_family(theme::FONT_BODY)
                            .child("Screenshots"),
                    )
                    .child(div().text_size(px(13.6)).text_color(theme::white_a(0.62)).child(total.to_string())),
            )
            .child(close)
    }
}

/// `.collectionScreenshots__thumb`: rounded tile with cover image; the image
/// grows 4% on hover (no transform in GPUI, so the box is enlarged instead).
fn thumb_tile(id: SharedString, src: Option<ImageSource>, hover_zoom: bool) -> Stateful<Div> {
    let group: SharedString = format!("{id}-g").into();
    let mut tile = div()
        .id(id)
        .group(group.clone())
        .relative()
        .rounded(px(8.))
        .overflow_hidden()
        .bg(theme::black_a(0.35))
        .cursor_pointer();
    if let Some(src) = src {
        let mut holder = div().absolute().top_0().left_0().size_full();
        if hover_zoom {
            holder = holder.group_hover(group, |s| s.top(relative(-0.02)).left(relative(-0.02)).w(relative(1.04)).h(relative(1.04)));
        }
        tile = tile.child(holder.child(img(src).size_full().rounded(px(8.)).object_fit(ObjectFit::Cover)));
    }
    tile
}

/// Round translucent icon button (`collectionFocus__iconBtn`).
pub fn icon_button(id: impl Into<ElementId>, ic: Icon, d: f32, alpha: f32, hover_alpha: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(d))
        .rounded_full()
        .cursor_pointer()
        .bg(theme::rgb_a(10, 12, 14, alpha))
        .hover(move |s| s.bg(theme::rgb_a(10, 12, 14, hover_alpha)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(ic, px(24.), rgb(0xf4f6f8)))
}
