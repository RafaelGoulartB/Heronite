//! `CollectionFocus`: the detail panel on the right of the list.

mod description;
mod panels;

pub use description::html_image_size;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use self::description::render_html;
use self::panels::{details_panel, hltb_panel, related_panel};
use super::vm::FocusVm;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, round_button};

pub type CoverFor = Rc<dyn Fn(&str) -> Option<ImageSource>>;

/// Main-column width in logical px (for sizing description images).
pub fn main_column_width(panel_w: f32) -> f32 {
    let narrow = panel_w <= 1100. - theme::LIST_COL_W_MD;
    if narrow {
        panel_w - 2. * 19.2
    } else {
        // `.collectionFocus__header/__body { width: min(1480px, 100%); margin-inline: auto }`
        let inner_w = (panel_w - 72.).min(1480.);
        let side_w = (inner_w - 38.4).clamp(352., 480.).min(inner_w * 0.5).max(352.);
        (inner_w - side_w - 38.4).min(1480.)
    }
}

pub struct FocusProps {
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
    /// "Screenshots" section (first block of the main column).
    pub screenshots: Option<AnyElement>,
    /// Epic install / update progress in the footer.
    pub epic: Option<super::epic::FocusEpic>,
    /// Achievements and session history panels of the side column.
    pub extras: FocusExtras,
}

/// Extra panels of the side column (see `extras.rs`).
#[derive(Default)]
pub struct FocusExtras {
    /// Above "Details" (`collectionFocus__panel` of achievements).
    pub top: Option<AnyElement>,
}

pub struct FocusActions {
    pub on_scroll: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_play: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_favourite: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_metadata: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_game_settings: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_facet: Rc<dyn Fn(&'static str, SharedString, &mut Window, &mut App)>,
    pub on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
}

pub fn h3(text: impl Into<SharedString>) -> Div {
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

pub fn focus_panel(props: FocusProps, actions: FocusActions) -> AnyElement {
    let FocusProps { vm, cover_for, html_images, viewport_w, panel_w, active_facets, scroll, screenshots, epic, extras } = props;
    let on_scroll: Rc<dyn Fn(&mut Window, &mut App)> = Rc::from(actions.on_scroll);
    let (epic_el, hide_play) = match epic {
        Some(e) => (Some(e.element), e.replaces_play),
        None => (None, false),
    };
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
    let narrow = panel_w <= 1100. - theme::LIST_COL_W_MD;
    let title_size = (viewport_w * 0.022).clamp(24.8, 32.);
    let cover_w = if narrow { 72. } else { 86. };

    let header = header(&vm, title_size, cover_w, &cover_for);
    let main = main_column(&vm, screenshots, &html_images, &cover_for, panel_w, actions.on_select);
    let side = side_column(&vm, extras.top, &active_facets, actions.on_facet);
    let body = body(narrow, panel_w, main, side);

    let on_scroll_wheel = on_scroll.clone();
    div()
        .id("focus")
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("focus-inner")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .on_scroll_wheel(move |_, w, cx| on_scroll_wheel(w, cx))
                        .pt(px(if narrow { 32. } else { 38.4 }))
                        .px(px(if narrow { 19.2 } else { 36. }))
                        .pb(px(16.))
                        .child(div().flex().flex_col().w_full().max_w(px(1480.)).mx_auto().child(header).child(body)),
                )
                .children(scrollbar),
        )
        .child(footer(&vm, epic_el, hide_play, actions.on_play, actions.on_favourite, actions.on_metadata, actions.on_game_settings))
        .into_any_element()
}

/// Title, meta line and cover.
fn header(vm: &FocusVm, title_size: f32, cover_w: f32, cover_for: &CoverFor) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap(px(24.))
        .mb(px(28.))
        .pr(px(84.))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
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
                }),
        )
        .child(
            div()
                .flex_none()
                .w(px(cover_w))
                .h(px(cover_w * 1.5))
                .rounded(px(6.))
                .overflow_hidden()
                .shadow(vec![BoxShadow {
                    color: theme::black_a(0.5),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .children(cover_for(&vm.cover_key).map(|src| img(src).size_full().rounded(px(6.)).object_fit(ObjectFit::Cover))),
        )
}

/// Screenshots, overview and related games.
fn main_column(
    vm: &FocusVm,
    screenshots: Option<AnyElement>,
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

/// Main and side columns: stacked when narrow, side by side otherwise.
fn body(narrow: bool, panel_w: f32, main: Div, side: Div) -> Div {
    if narrow {
        return div().flex().flex_col().gap(px(20.)).child(main).child(side);
    }
    // minmax(0,1.2fr) minmax(22rem,30rem): the side column reaches its
    // max (480) before the flexible column takes the rest.
    // `.collectionFocus__header/__body { width: min(1480px, 100%); margin-inline: auto }`
    let inner_w = (panel_w - 72.).min(1480.);
    let side_w = (inner_w - 38.4).clamp(352., 480.).min(inner_w * 0.5).max(352.);
    div().flex().items_start().gap(px(38.4)).child(main.flex_1()).child(side.w(px(side_w)).flex_none())
}

/// Favourite, metadata and settings buttons, Epic progress and Play.
fn footer(
    vm: &FocusVm,
    epic_el: Option<AnyElement>,
    hide_play: bool,
    on_play: Box<dyn Fn(&mut Window, &mut App)>,
    on_fav: Box<dyn Fn(&mut Window, &mut App)>,
    on_meta: Box<dyn Fn(&mut Window, &mut App)>,
    on_gsettings: Box<dyn Fn(&mut Window, &mut App)>,
) -> Div {
    let play_label = if vm.playing {
        "Stop"
    } else if vm.installed {
        "Play Now"
    } else {
        "Install"
    };
    let play_icon = if vm.playing {
        Icon::Stop
    } else if vm.installed {
        Icon::PlayArrow
    } else {
        Icon::Download
    };
    div()
        .flex()
        .flex_none()
        .justify_end()
        .items_center()
        .gap(px(10.))
        .pt(px(11.2))
        .px(px(20.))
        .pb(px(16.))
        .child(
            round_button("focus-fav", if vm.favourite { Icon::Star } else { Icon::StarBorder }, 42., 0.72)
                .on_click(move |_, w, cx| on_fav(w, cx)),
        )
        .child(round_button("focus-meta", Icon::TravelExplore, 42., 0.72).on_click(move |_, w, cx| on_meta(w, cx)))
        .child(round_button("focus-settings", Icon::Settings, 42., 0.72).on_click(move |_, w, cx| on_gsettings(w, cx)))
        .children(epic_el)
        .when(!hide_play, |d| {
            d.child(
                div()
                    .id("focus-play")
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .min_w(px(118.))
                    .py(px(8.32))
                    .px(px(20.))
                    .rounded_full()
                    .bg(rgb(0xf3f5f7))
                    .hover(|s| s.bg(rgb(0xffffff)))
                    .text_color(rgb(0x161616))
                    .font_weight(theme::W_BOLD)
                    .text_size(px(15.2))
                    .cursor_pointer()
                    .shadow(vec![BoxShadow {
                        color: theme::black_a(0.28),
                        offset: point(px(0.), px(8.)),
                        blur_radius: px(24.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .on_click(move |_, w, cx| on_play(w, cx))
                    .child(icon(play_icon, px(19.2), rgb(0x161616)))
                    .child(play_label),
            )
        })
}
