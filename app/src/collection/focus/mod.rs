//! `CollectionFocus`: the detail panel on the right of the list: a fixed left
//! column (cover, Play, status, rating, facts) next to the scrolling details.

mod description;
mod panels;

pub use description::html_image_size;
pub(super) use panels::{RELATED_COVER_H, RELATED_COVER_W};

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use self::description::render_html;
use self::panels::{details_panel, details_value_width, hltb_panel, related_panel};
use super::vm::FocusVm;
use crate::theme;
use heronite_i18n::tr;

pub(super) type CoverFor = Rc<dyn Fn(&str) -> Option<ImageSource>>;
/// Width of a chip's text (`ui::chip`: 12px body font), in logical px.
pub(super) type MeasureChip = Rc<dyn Fn(&str) -> f32>;

/// What Details needs besides its rows.
#[derive(Clone)]
pub(super) struct DetailsUi {
    /// Labels of the chip rows showing every value (their "+N" was clicked).
    pub open: Vec<SharedString>,
    pub measure: MeasureChip,
}

/// Widths of the focus panel's columns, in logical px.
#[derive(Clone, Copy, Debug)]
pub(super) struct Layout {
    /// Everything stacked in one scrolling column (the left column becomes a row).
    pub narrow: bool,
    /// Fixed left column (cover, Play, status, rating, facts); 0 when narrow.
    pub left_w: f32,
    /// Main and side sections in one column (no room for both columns at a
    /// readable width).
    pub stacked: bool,
    pub main_w: f32,
    pub side_w: f32,
    /// Empty space before the left column, so that the left column and the
    /// details (at most 1480 wide) sit centred in a very wide panel.
    pub offset: f32,
}

const LEFT_PAD: f32 = 36.;
/// Gap between the left column and the details (same as between main and side).
const CONTENT_PAD_L: f32 = COL_GAP;
const CONTENT_PAD_R: f32 = 36.;
const COL_GAP: f32 = 38.4;
/// Narrowest main column (screenshots, overview, timeline) kept next to the
/// side column; below it the side sections join the main column.
const MAIN_MIN_W: f32 = 520.;
const SIDE_MIN_W: f32 = 352.;
const SIDE_MAX_W: f32 = 480.;

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
    // minmax(0,1.2fr) minmax(22rem,30rem): the side column reaches its
    // max (480) before the flexible column takes the rest.
    let side_w = (inner_w - COL_GAP).clamp(SIDE_MIN_W, SIDE_MAX_W).min(inner_w * 0.5).max(SIDE_MIN_W);
    let main_w = inner_w - side_w - COL_GAP;
    if main_w < MAIN_MIN_W {
        return Layout { narrow, left_w, stacked: true, main_w: inner_w, side_w: inner_w, offset };
    }
    Layout { narrow, left_w, stacked: false, main_w, side_w, offset }
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
    pub related_ui: RelatedUi,
    pub details_ui: DetailsUi,
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
    /// A folder row of Details (install, emulator, ROM): opens it in the file manager.
    pub on_open_folder: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
    /// Shows every value of a chip row of Details (by its label).
    pub on_expand_detail: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
    /// Picks the group of "Related games".
    pub on_related: Rc<dyn Fn(RelatedUi, &mut Window, &mut App)>,
}

/// What "Related games" shows of the focused game.
#[derive(Clone, Default)]
pub(super) struct RelatedUi {
    /// Group on screen, an index into `FocusVm::related` (the first, the
    /// series when there is one, for each game).
    pub tab: usize,
    /// The sideways row of covers (back to its start with each group).
    pub scroll: ScrollHandle,
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

/// Small pills on a segmented track for a section title (the size of the
/// command palette's filters, so the title stays one line).
pub(super) fn small_tabs(
    id: &'static str,
    labels: &[SharedString],
    active: usize,
    on_pick: Rc<dyn Fn(usize, &mut Window, &mut App)>,
) -> Div {
    let mut bar = div().flex().flex_none().items_center().gap(px(2.)).p(px(2.)).rounded_full().bg(theme::seg_track());
    for (i, label) in labels.iter().enumerate() {
        let on = i == active;
        let pick = on_pick.clone();
        bar = bar.child(
            div()
                .id(SharedString::from(format!("{id}-{i}")))
                .px(px(10.))
                .py(px(2.))
                .rounded_full()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(if on { white() } else { Hsla::from(theme::seg_text()) })
                .when(on, |d| d.bg(theme::seg_active()))
                .when(!on, |d| d.hover(|s| s.text_color(white())))
                .on_click(move |_, w, cx| pick(i, w, cx))
                .child(label.clone()),
        );
    }
    bar
}

/// A section surface. Dense enough that its secondary text (labels at .52)
/// keeps a 4.5:1 contrast over bright Collection art.
pub fn panel(facts: bool) -> Div {
    let d = div().flex().flex_col().border_1();
    if facts {
        d.pt(px(10.4)).px(px(16.)).pb(px(12.8)).rounded(px(12.)).bg(theme::rgb_a(18, 22, 28, 0.45)).border_color(theme::white_a(0.1))
    } else {
        d.py(px(15.2)).px(px(16.8)).rounded(px(10.)).bg(theme::rgb_a(12, 14, 18, 0.5)).border_color(theme::white_a(0.07))
    }
}

pub(super) fn focus_panel(props: FocusProps, actions: FocusActions) -> AnyElement {
    let FocusProps {
        vm,
        cover_for,
        html_images,
        viewport_w,
        panel_w,
        active_facets,
        scroll,
        left,
        screenshots,
        timeline,
        extras,
        related_ui,
        details_ui,
    } = props;
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
    let details_w = if lay.stacked { lay.main_w } else { lay.side_w };
    let details = details_panel(
        &vm.details,
        &active_facets,
        details_value_width(details_w),
        &details_ui.open,
        &details_ui.measure,
        actions.on_facet,
        actions.on_open_folder,
        actions.on_expand_detail,
    );
    let sections = Sections {
        screenshots,
        overview: overview(&vm, &html_images, main_column_width(panel_w)),
        timeline,
        related: (!vm.related.is_empty())
            .then(|| related_panel(&vm.related, related_ui, &cover_for, actions.on_select, actions.on_related)),
        top: extras.top,
        details: div().flex().flex_col().child(h3(tr!("Details"))).child(details),
        hltb: vm.hltb.as_ref().map(|h| div().flex().flex_col().child(h3(tr!("How Long To Beat"))).child(hltb_panel(h))),
    };
    let body = body(&lay, sections);
    // Wide: the title sits above a fixed left column and the scrolling
    // details, so the cover, "Screenshots" and "Details" start on one line.
    // Narrow: everything scrolls, the left block is a row under the title.
    let (top, scrolled_header, left_col, left_row) =
        if lay.narrow { (None, Some(header), None, Some(div().mb(px(24.)).child(left))) } else { (Some(header), None, Some(left), None) };

    let on_scroll_wheel = on_scroll.clone();
    div()
        .id("focus")
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .children(top.map(|h| {
            div()
                .flex_none()
                .pt(px(38.4))
                .pl(px(lay.offset + LEFT_PAD))
                .pr(px(CONTENT_PAD_R))
                .child(h.max_w(px(lay.left_w + CONTENT_PAD_L + 1480.)))
        }))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .children(left_col.map(|left| {
                    div()
                        .id("focus-left")
                        .flex_none()
                        .w(px(lay.offset + lay.left_w + LEFT_PAD))
                        .h_full()
                        .overflow_y_scroll()
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
                                .pt(px(if lay.narrow { 32. } else { 0. }))
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
                                        .children(scrolled_header)
                                        .children(left_row)
                                        .child(body),
                                ),
                        )
                        .children(scrollbar),
                ),
        )
        .into_any_element()
}

/// What [`stacked_details`] shows besides the view model.
pub(super) struct StackedProps {
    pub vm: FocusVm,
    pub cover_for: CoverFor,
    pub html_images: CoverFor,
    /// Width of the column, in logical px.
    pub width: f32,
    pub active_facets: Vec<(&'static str, SharedString)>,
    pub screenshots: Option<AnyElement>,
    pub timeline: Option<AnyElement>,
    pub extras: FocusExtras,
    pub related_ui: RelatedUi,
    pub details_ui: DetailsUi,
}

/// The details of the focus panel in one column (the grid's drawer): no
/// title and no left column, which the caller lays out itself.
pub(super) fn stacked_details(props: StackedProps, actions: FocusActions) -> Div {
    let StackedProps { vm, cover_for, html_images, width, active_facets, screenshots, timeline, extras, related_ui, details_ui } = props;
    let lay = Layout { narrow: true, left_w: 0., stacked: true, main_w: width, side_w: width, offset: 0. };
    let details = details_panel(
        &vm.details,
        &active_facets,
        details_value_width(width),
        &details_ui.open,
        &details_ui.measure,
        actions.on_facet,
        actions.on_open_folder,
        actions.on_expand_detail,
    );
    body(
        &lay,
        Sections {
            screenshots,
            overview: overview(&vm, &html_images, width),
            timeline,
            related: (!vm.related.is_empty())
                .then(|| related_panel(&vm.related, related_ui, &cover_for, actions.on_select, actions.on_related)),
            top: extras.top,
            details: div().flex().flex_col().child(h3(tr!("Details"))).child(details),
            hltb: vm.hltb.as_ref().map(|h| div().flex().flex_col().child(h3(tr!("How Long To Beat"))).child(hltb_panel(h))),
        },
    )
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
        .when(!vm.meta_line.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(theme::white_a(0.8)).child(vm.meta_line.clone())))
}

/// The blocks of the details, placed by [`body`].
struct Sections {
    screenshots: Option<AnyElement>,
    overview: Div,
    timeline: Option<AnyElement>,
    related: Option<Div>,
    /// Extra panel above "Details" (achievements).
    top: Option<AnyElement>,
    details: Div,
    hltb: Option<Div>,
}

/// `column_w`: width the description images are sized for.
fn overview(vm: &FocusVm, html_images: &CoverFor, column_w: f32) -> Div {
    div().flex().flex_col().child(h3(tr!("Overview"))).child(match &vm.overview_html {
        Some(blocks) => render_html(blocks, &**html_images, column_w).into_any_element(),
        None => div()
            .max_w(px(72. * 7.6))
            .text_size(px(15.2))
            .line_height(px(15.2 * 1.65))
            .text_color(theme::white_a(0.92))
            .child(vm.overview.clone().unwrap_or_else(|| tr!("No description available").into()))
            .into_any_element(),
    })
}

/// Two columns (screenshots, overview, timeline, related | extra panel,
/// details, How Long To Beat), or one when stacked. Stacked, the short side
/// sections come right after the overview so a long timeline does not push
/// them out of sight.
fn body(lay: &Layout, s: Sections) -> Div {
    let Sections { screenshots, overview, timeline, related, top, details, hltb } = s;
    if lay.stacked {
        return div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .min_w_0()
            .children(screenshots)
            .child(overview)
            .children(hltb)
            .children(top)
            .child(details)
            .children(timeline)
            .children(related);
    }
    let main = div().flex().flex_col().gap(px(24.)).min_w_0().children(screenshots).child(overview).children(timeline).children(related);
    let side = div().flex().flex_col().gap(px(14.4)).min_w_0().children(top).child(details).children(hltb);
    div().flex().items_start().gap(px(COL_GAP)).child(main.flex_1()).child(side.w(px(lay.side_w)).flex_none())
}

#[cfg(test)]
mod tests {
    use super::{layout, MAIN_MIN_W, SIDE_MAX_W};
    use crate::theme;

    #[test]
    fn wide_panel_keeps_two_detail_columns() {
        let lay = layout(1920. - theme::LIST_COL_W);
        assert!(!lay.narrow && !lay.stacked);
        assert!(lay.main_w >= MAIN_MIN_W);
        assert_eq!(lay.side_w, SIDE_MAX_W);
    }

    #[test]
    fn medium_panel_stacks_the_side_sections() {
        // 1600 px window next to the 3-cover list: two columns would leave
        // the main one about 380 px wide.
        let lay = layout(1600. - theme::LIST_COL_W);
        assert!(!lay.narrow && lay.stacked);
        assert_eq!(lay.left_w, 232.);
    }

    #[test]
    fn small_panel_is_narrow() {
        let lay = layout(1000. - theme::LIST_COL_W_MD);
        assert!(lay.narrow && lay.stacked);
        assert_eq!(lay.left_w, 0.);
    }
}
