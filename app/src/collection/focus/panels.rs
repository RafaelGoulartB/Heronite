//! Side-column and related panels of the focus panel.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{h3, panel, CoverFor};
use crate::collection::vm::{DetailRow, DetailValue, HltbVm, RelatedGroupVm};
use crate::icons::Icon;
use crate::theme;
use crate::ui::{chip, icon};

pub(super) fn details_panel(
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

pub(super) fn hltb_panel(h: &HltbVm) -> Div {
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

pub(super) fn related_panel(
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
                                    .map(|src| img(src).size_full().rounded(px(6.)).object_fit(ObjectFit::Cover)),
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
