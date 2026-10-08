//! `CollectionFocus`: the detail panel on the right of the list.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::vm::{DetailRow, DetailValue, FocusVm, HltbVm, RelatedGroupVm};
use crate::icons::Icon;
use crate::theme;
use crate::ui::{chip, icon, round_button};

pub type CoverFor = Rc<dyn Fn(&str) -> Option<ImageSource>>;

pub struct FocusProps {
    pub vm: FocusVm,
    pub cover_for: CoverFor,
    /// Window width, for `clamp(1.55rem, 2.2vw, 2rem)`.
    pub viewport_w: f32,
    /// Container width of the focus panel (container queries).
    pub panel_w: f32,
    pub active_facets: Vec<(&'static str, SharedString)>,
}

pub struct FocusActions {
    pub on_close: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_play: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_favourite: Box<dyn Fn(&mut Window, &mut App)>,
    pub on_facet: Rc<dyn Fn(&'static str, SharedString, &mut Window, &mut App)>,
    pub on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
}

fn h3(text: impl Into<SharedString>) -> Div {
    div()
        .mb(px(8.8))
        .font_family(theme::FONT_HEADING)
        .text_size(px(16.8))
        .line_height(px(20.))
        .font_weight(theme::W_BOLD)
        .text_color(white())
        .child(text.into())
}

fn panel(facts: bool) -> Div {
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
    let FocusProps { vm, cover_for, viewport_w, panel_w, active_facets } = props;
    let narrow = panel_w <= 1100. - theme::LIST_COL_W_MD;
    let title_size = (viewport_w * 0.022).clamp(24.8, 32.);
    let cover_w = if narrow { 72. } else { 86. };

    let header = div()
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
                    d.child(
                        div().text_size(px(13.)).text_color(theme::white_a(0.7)).child(vm.meta_line.clone()),
                    )
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
                .children(cover_for(&vm.cover_key).map(|src| img(src).size_full().object_fit(ObjectFit::Cover))),
        );

    let mut main = div().flex().flex_col().gap(px(24.)).min_w_0();
    main = main.child(
        div()
            .flex()
            .flex_col()
            .child(h3("Overview"))
            .child(
                div()
                    .max_w(px(72. * 7.6))
                    .text_size(px(15.2))
                    .line_height(px(15.2 * 1.65))
                    .text_color(theme::white_a(0.92))
                    .child(vm.overview.clone().unwrap_or_else(|| "No description available".into())),
            ),
    );
    if !vm.related.is_empty() {
        main = main.child(related_panel(&vm.related, &cover_for, actions.on_select.clone()));
    }

    let mut side = div().flex().flex_col().gap(px(14.4)).min_w_0();
    side = side.child(
        div()
            .flex()
            .flex_col()
            .child(h3("Details"))
            .child(details_panel(&vm.details, &active_facets, actions.on_facet.clone())),
    );
    if let Some(h) = &vm.hltb {
        side = side.child(div().flex().flex_col().child(h3("How Long To Beat")).child(hltb_panel(h)));
    }

    let body = if narrow {
        div().flex().flex_col().gap(px(20.)).child(main).child(side)
    } else {
        // minmax(0,1.2fr) minmax(22rem,30rem): the side column reaches its
        // max (480) before the flexible column takes the rest.
        let inner_w = panel_w - 72.;
        let side_w = (inner_w - 38.4).clamp(352., 480.).min(inner_w * 0.5).max(352.);
        div()
            .flex()
            .items_start()
            .gap(px(38.4))
            .child(main.flex_1())
            .child(side.w(px(side_w)).flex_none())
    };

    let on_close = actions.on_close;
    let on_play = actions.on_play;
    let on_fav = actions.on_favourite;
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
        .id("focus")
        .relative()
        .flex()
        .flex_col()
        .size_full()
        .min_w_0()
        .child(
            div()
                .absolute()
                .top(px(13.6))
                .right(px(17.6))
                .flex()
                .gap(px(8.))
                .child(round_button("focus-open", Icon::OpenInNew, 36., 0.42))
                .child(round_button("focus-close", Icon::Close, 36., 0.42).on_click(move |_, w, cx| on_close(w, cx))),
        )
        .child(
            div()
                .id("focus-inner")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .pt(px(if narrow { 32. } else { 38.4 }))
                .px(px(if narrow { 19.2 } else { 36. }))
                .pb(px(16.))
                .child(div().flex().flex_col().max_w(px(1480.)).child(header).child(body)),
        )
        .child(
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
                    round_button(
                        "focus-fav",
                        if vm.favourite { Icon::Star } else { Icon::StarBorder },
                        42.,
                        0.72,
                    )
                    .on_click(move |_, w, cx| on_fav(w, cx)),
                )
                .child(round_button("focus-meta", Icon::TravelExplore, 42., 0.72))
                .child(round_button("focus-settings", Icon::Settings, 42., 0.72))
                .child(
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
                ),
        )
        .into_any_element()
}

fn details_panel(
    rows: &[DetailRow],
    active: &[(&'static str, SharedString)],
    on_facet: Rc<dyn Fn(&'static str, SharedString, &mut Window, &mut App)>,
) -> Div {
    let mut p = panel(true);
    for (ri, row) in rows.iter().enumerate() {
        let label = div()
            .w(px(134.4))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.4))
            .line_height(px(18.2))
            .text_color(theme::white_a(0.52))
            .when(matches!(row.value, DetailValue::Folder { .. }), |d| {
                d.child(icon(Icon::FolderOpen, px(15.2), theme::white_a(0.52 * 0.72)))
            })
            .child(row.label.clone());
        let value: AnyElement = match &row.value {
            DetailValue::Text(t) => div().line_height(px(18.2)).child(t.clone()).into_any_element(),
            DetailValue::Status { name, color } => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .line_height(px(18.2))
                .child(div().size(px(8.)).flex_none().rounded_full().bg(*color))
                .child(name.clone())
                .into_any_element(),
            DetailValue::Chips { values, facet, clamp } => {
                let mut wrap = div().flex().flex_wrap().gap(px(5.12));
                for (ci, v) in values.iter().enumerate() {
                    let is_active = facet.is_some_and(|f| active.iter().any(|(af, av)| *af == f && av == v));
                    let mut c = chip(v.clone());
                    if is_active {
                        c = c.bg(theme::accent()).text_color(rgb(0x111111));
                    }
                    match facet {
                        Some(f) => {
                            let f = *f;
                            let v2 = v.clone();
                            let cb = on_facet.clone();
                            wrap = wrap.child(
                                c.id(SharedString::from(format!("chip-{ri}-{ci}")))
                                    .cursor_pointer()
                                    .when(!is_active, |d| d.hover(|s| s.bg(theme::white_a(0.2))))
                                    .on_click(move |_, w, cx| cb(f, v2.clone(), w, cx)),
                            );
                        }
                        None => wrap = wrap.child(c),
                    }
                }
                if *clamp {
                    div().max_h(px(73.6)).overflow_hidden().child(wrap).into_any_element()
                } else {
                    wrap.into_any_element()
                }
            }
            DetailValue::Folder { path, size } => div()
                .flex()
                .flex_col()
                .gap(px(2.4))
                .child(div().line_height(px(18.2)).child(path.clone()))
                .children(size.clone().map(|s| div().text_size(px(12.)).text_color(theme::white_a(0.52)).child(s)))
                .into_any_element(),
        };
        p = p.child(
            div()
                .flex()
                .items_start()
                .gap(px(13.6))
                .py(px(6.08))
                .text_size(px(13.))
                .when(row.split, |d| d.mt(px(6.4)).pt(px(11.2)).border_t_1().border_color(theme::white_a(0.12)))
                .child(label)
                .child(div().flex_1().min_w_0().text_color(theme::white_a(0.94)).child(value)),
        );
    }
    p
}

fn format_hours(v: Option<f32>) -> SharedString {
    let Some(v) = v.filter(|v| *v > 0.) else { return "—".into() };
    let total = (v * 60.).round() as i64;
    let (h, m) = (total / 60, total % 60);
    match (h, m) {
        (0, m) => format!("{m}m").into(),
        (h, 0) => format!("{h}h").into(),
        (h, m) => format!("{h}h {m}m").into(),
    }
}

fn hltb_panel(h: &HltbVm) -> Div {
    let max = h.complete.unwrap_or(0.).max(h.extra.unwrap_or(0.)).max(h.main.unwrap_or(0.));
    let marker = (max > 0. && h.played_hours > 0.).then(|| (h.played_hours / max).min(1.));
    let seg = |c: u32| div().flex_1().min_w_0().h_full().rounded_full().bg(rgb(c));
    let bar = div()
        .relative()
        .flex()
        .gap(px(3.))
        .h(px(12.))
        .mt(px(5.6))
        .mb(px(16.))
        .child(seg(0x2aa9a0))
        .child(seg(0x5b6ef5))
        .child(seg(0x43b35c))
        .children(marker.map(|m| {
            div()
                .absolute()
                .top(px(-3.))
                .left(relative(m))
                .ml(px(-7.))
                .w(px(14.))
                .h(px(18.))
                .rounded(px(6.))
                .bg(rgb(0x1a1c20))
                .shadow(vec![BoxShadow {
                    color: theme::black_a(0.45),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(4.),
                    spread_radius: px(0.),
                    inset: false,
                }])
        }));
    let stat = |label: &'static str, v: Option<f32>| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.4))
            .child(div().text_size(px(12.)).line_height(px(15.)).text_color(theme::white_a(0.52)).child(label))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .py(px(6.72))
                    .px(px(5.6))
                    .rounded(px(8.))
                    .bg(theme::black_a(0.48))
                    .text_size(px(13.))
                    .line_height(px(15.6))
                    .font_weight(theme::W_500)
                    .text_color(theme::white_a(0.94))
                    .child(format_hours(v)),
            )
    };
    panel(true).child(bar).child(
        div()
            .flex()
            .gap(px(8.8))
            .child(stat("Main Story", h.main))
            .child(stat("Main + Extras", h.extra))
            .child(stat("Completionist", h.complete)),
    )
}

fn related_panel(
    groups: &[RelatedGroupVm],
    cover_for: &CoverFor,
    on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
) -> Div {
    let mut p = panel(false).child(h3("Related games"));
    for (gi, g) in groups.iter().enumerate() {
        let mut row = div().flex().gap(px(11.2)).pb(px(4.));
        for game in &g.games {
            let key = game.key.clone();
            let cb = on_select.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("rel-{gi}-{key}")))
                    .flex_none()
                    .w(px(78.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .cursor_pointer()
                    .on_click(move |_, w, cx| cb(key.clone(), w, cx))
                    .child(
                        div()
                            .w(px(78.))
                            .h(px(117.))
                            .rounded(px(6.))
                            .overflow_hidden()
                            .shadow(vec![BoxShadow {
                                color: theme::black_a(0.45),
                                offset: point(px(0.), px(8.)),
                                blur_radius: px(20.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .children(
                                cover_for(&game.cover_key)
                                    .map(|src| img(src).size_full().object_fit(ObjectFit::Cover)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::white_a(0.82))
                            .truncate()
                            .child(game.title.clone()),
                    ),
            );
        }
        p = p.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(13.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::white_a(0.72))
                        .child(icon(Icon::ExpandMore, px(18.), theme::white_a(0.72)))
                        .child(g.title.clone()),
                )
                .child(div().id(SharedString::from(format!("rel-row-{gi}"))).overflow_x_scroll().child(row)),
        );
    }
    p
}
