//! `CollectionFocus`: the detail panel on the right of the list: a fixed left
//! column (cover, Play, status, rating, facts) next to the scrolling details.

mod description;
mod panels;

pub use description::html_image_size;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use self::description::render_html;
use self::panels::{details_panel, hltb_panel, related_panel};
use super::vm::FocusVm;
use crate::theme;

pub(super) type CoverFor = Rc<dyn Fn(&str) -> Option<ImageSource>>;

/// Widths of the focus panel's columns, in logical px.
#[derive(Clone, Copy, Debug)]
pub(super) struct Layout {
    /// Everything stacked in one scrolling column (the left column becomes a row).
    pub narrow: bool,
    /// Fixed left column (cover, Play, status, rating, facts); 0 when narrow.
    pub left_w: f32,
    /// Main and side columns stacked (not enough room for both).
    pub stacked: bool,
    pub main_w: f32,
    pub side_w: f32,
    /// Empty space before the left column, so that the left column and the
    /// details (at most 1480 wide) sit centred in a very wide panel.
    pub offset: f32,
}

const LEFT_PAD: f32 = 36.;
const CONTENT_PAD_L: f32 = 32.;
const CONTENT_PAD_R: f32 = 36.;
const COL_GAP: f32 = 38.4;

pub(super) fn layout(panel_w: f32) -> Layout {
    let narrow = panel_w <= 1100. - theme::LIST_COL_W_MD;
    if narrow {
        let w = panel_w - 2. * 19.2;
        return Layout { narrow, left_w: 0., stacked: true, main_w: w, side_w: w, offset: 0. };
    }
    let left_w = if panel_w >= 1400. { 264. } else { 232. };
    // `.collectionFocus__body { width: min(1480px, 100%) }`, next to the left column.
    let room = panel_w - LEFT_PAD - left_w - CONTENT_PAD_L - CONTENT_PAD_R;
    let inner_w = room.min(1480.);
    let offset = ((room - inner_w) / 2.).max(0.);
    if inner_w < 352. + COL_GAP + 440. {
        return Layout { narrow, left_w, stacked: true, main_w: inner_w, side_w: inner_w, offset };
    }
    // minmax(0,1.2fr) minmax(22rem,30rem): the side column reaches its
    // max (480) before the flexible column takes the rest.
    let side_w = (inner_w - COL_GAP).clamp(352., 480.).min(inner_w * 0.5).max(352.);
    Layout { narrow, left_w, stacked: false, main_w: inner_w - side_w - COL_GAP, side_w, offset }
}

/// Main-column width in logical px (for sizing description images).
pub(super) fn main_column_width(panel_w: f32) -> f32 {
    layout(panel_w).main_w
}

pub(super) struct FocusProps {
    pub vm: FocusVm,
    pub cover_for: CoverFor,
    /// Description images by URL.
    pub html_images: CoverFor,
    /// Window width, for `clamp(1.55rem, 2.2vw, 2rem)`.
    pub viewport_w: f32,
    /// Container width of the focus panel (container queries).
    pub panel_w: f32,
    pub active_facets: Vec<(&'static str, SharedString)>,
    pub scroll: ScrollHandle,
    /// Cover, Play, status, rating and facts (see `focus_left.rs`): a fixed
    /// column on the left, or a row under the title when narrow.
    pub left: AnyElement,
    /// "Screenshots" section (first block of the main column).
    pub screenshots: Option<AnyElement>,
    /// Sessions and screenshots by day (after the overview).
    pub timeline: Option<AnyElement>,
    /// Achievements and session history panels of the side column.
    pub extras: FocusExtras,
}

/// Extra panels of the side column (see `extras.rs`).
#[derive(Default)]
pub(super) struct FocusExtras {
    /// Above "Details" (`collectionFocus__panel` of achievements).
    pub top: Option<AnyElement>,
}

pub(super) struct FocusActions {
    pub on_scroll: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_facet: Rc<dyn Fn(&'static str, SharedString, &mut Window, &mut App)>,
    pub on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
}

pub(super) fn h3(text: impl Into<SharedString>) -> Div {
    div()
        .mb(px(8.8))
        .font_family(theme::FONT_HEADING)
        .text_size(px(16.8))
        .line_height(px(20.))
        .font_weight(theme::W_BOLD)
        .text_color(white())
        .child(text.into())
}

pub fn panel(facts: bool) -> Div {
    let d = div().flex().flex_col().border_1();
    if facts {
        d.pt(px(10.4))
            .px(px(16.))
            .pb(px(12.8))
            .rounded(px(12.))
            .bg(theme::rgb_a(18, 22, 28, 0.28))
            .border_color(theme::white_a(0.1))
    } else {
        d.py(px(15.2))
            .px(px(16.8))
            .rounded(px(10.))
            .bg(theme::rgb_a(12, 14, 18, 0.42))
            .border_color(theme::white_a(0.07))
    }
}

pub(super) fn focus_panel(props: FocusProps, actions: FocusActions) -> AnyElement {
    let FocusProps { vm, cover_for, html_images, viewport_w, panel_w, active_facets, scroll, left, screenshots, timeline, extras } =
        props;
    let on_scroll: Rc<dyn Fn(&mut Window, &mut App)> = Rc::from(actions.on_scroll);
    let scrollbar = crate::ui::scrollbar_draggable(
        "focus-scrollbar",
        scroll.offset().y,
        scroll.max_offset().y,
        scroll.bounds().size.height,
        {
            let (scroll, on_scroll) = (scroll.clone(), on_scroll.clone());
            move |distance, window, cx| {
                scroll.set_offset(point(px(0.), px(-distance)));
                on_scroll(window, cx);
            }
        },
        None,
    );
    let lay = layout(panel_w);
    let title_size = (viewport_w * 0.022).clamp(24.8, 32.);

    let header = header(&vm, title_size);
    let main = main_column(&vm, screenshots, timeline, &html_images, &cover_for, panel_w, actions.on_select);
    let side = side_column(&vm, extras.top, &active_facets, actions.on_facet);
    let body = body(&lay, main, side);
    let (left_col, left_row) = if lay.narrow { (None, Some(div().mb(px(24.)).child(left))) } else { (Some(left), None) };

    let on_scroll_wheel = on_scroll.clone();
    div()
        .id("focus")
        .relative()
        .flex()
        .size_full()
        .min_w_0()
        .children(left_col.map(|left| {
            div()
                .id("focus-left")
                .flex_none()
                .w(px(lay.offset + lay.left_w + LEFT_PAD))
                .h_full()
                .overflow_y_scroll()
                .pt(px(38.4))
                .pl(px(lay.offset + LEFT_PAD))
                .pb(px(16.))
                .child(left)
        }))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    div()
                        .id("focus-inner")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .on_scroll_wheel(move |_, w, cx| on_scroll_wheel(w, cx))
                        .pt(px(if lay.narrow { 32. } else { 38.4 }))
                        .pl(px(if lay.narrow { 19.2 } else { CONTENT_PAD_L }))
                        .pr(px(if lay.narrow { 19.2 } else { CONTENT_PAD_R }))
                        .pb(px(24.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .w_full()
                                .max_w(px(1480.))
                                .when(lay.narrow, |d| d.mx_auto())
                                .child(header)
                                .children(left_row)
                                .child(body),
                        ),
                )
                .children(scrollbar),
        )
        .into_any_element()
}

/// Title and meta line.
fn header(vm: &FocusVm, title_size: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .mb(px(28.))
        .child(
            div()
                .mb(px(6.4))
                .font_family(theme::FONT_HEADING)
                .text_size(px(title_size))
                .line_height(px(title_size * 1.15))
                .font_weight(theme::W_BOLD)
                .text_color(white())
                .child(vm.title.clone()),
        )
        .when(!vm.meta_line.is_empty(), |d| {
            d.child(div().text_size(px(13.)).text_color(theme::white_a(0.7)).child(vm.meta_line.clone()))
        })
}

/// Screenshots, overview, timeline and related games.
fn main_column(
    vm: &FocusVm,
    screenshots: Option<AnyElement>,
    timeline: Option<AnyElement>,
    html_images: &CoverFor,
    cover_for: &CoverFor,
    panel_w: f32,
    on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
) -> Div {
    let mut main = div().flex().flex_col().gap(px(24.)).min_w_0();
    if let Some(s) = screenshots {
        main = main.child(s);
    }
    main = main.child(
        div()
            .flex()
            .flex_col()
            .child(h3("Overview"))
            .child(match &vm.overview_html {
                Some(blocks) => render_html(blocks, &**html_images, main_column_width(panel_w)).into_any_element(),
                None => div()
                    .max_w(px(72. * 7.6))
                    .text_size(px(15.2))
                    .line_height(px(15.2 * 1.65))
                    .text_color(theme::white_a(0.92))
                    .child(vm.overview.clone().unwrap_or_else(|| "No description available".into()))
                    .into_any_element(),
            }),
    );
    main = main.children(timeline);
    if !vm.related.is_empty() {
        main = main.child(related_panel(&vm.related, cover_for, on_select));
    }
    main
}

/// Extra top panel, details and How Long To Beat.
fn side_column(
    vm: &FocusVm,
    top: Option<AnyElement>,
    active_facets: &[(&'static str, SharedString)],
    on_facet: Rc<dyn Fn(&'static str, SharedString, &mut Window, &mut App)>,
) -> Div {
    let mut side = div().flex().flex_col().gap(px(14.4)).min_w_0();
    side = side.children(top);
    side = side.child(div().flex().flex_col().child(h3("Details")).child(details_panel(&vm.details, active_facets, on_facet)));
    if let Some(h) = &vm.hltb {
        side = side.child(div().flex().flex_col().child(h3("How Long To Beat")).child(hltb_panel(h)));
    }
    side
}

/// Main and side columns: stacked when there is no room, side by side otherwise.
fn body(lay: &Layout, main: Div, side: Div) -> Div {
    if lay.stacked {
        return div().flex().flex_col().gap(px(20.)).child(main).child(side);
    }
    div().flex().items_start().gap(px(COL_GAP)).child(main.flex_1()).child(side.w(px(lay.side_w)).flex_none())
}
