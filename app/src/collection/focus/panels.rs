//! Side-column and related panels of the focus panel.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{h3, panel, small_tabs, CoverFor, MeasureChip, RelatedUi};
use crate::collection::vm::{DetailRow, DetailValue, HltbVm, RelatedGroupVm};
use crate::icons::Icon;
use crate::theme;
use crate::ui::{chip, icon, round_button};
use heronite_i18n::tr;

/// Chip of Details: its padding (`ui::chip`) and the gap between two.
const CHIP_PAD_X: f32 = 6.72;
const CHIP_GAP: f32 = 5.12;
const DETAIL_LABEL_W: f32 = 134.4;
const DETAIL_GAP: f32 = 13.6;

/// Width the values of Details get in a column `column_w` wide (`panel(true)`
/// pads 16 and has a 1px border on each side).
pub(super) fn details_value_width(column_w: f32) -> f32 {
    column_w - 2. * 17. - DETAIL_LABEL_W - DETAIL_GAP
}

/// How many of the chips (text widths `widths`) fit on one line `room` wide,
/// leaving room for a "+N" chip (its text width from `plus_w`) when some do
/// not; at least one.
fn chips_on_one_line(widths: &[f32], room: f32, plus_w: impl Fn(usize) -> f32) -> usize {
    let chip_w = |w: f32| w + 2. * CHIP_PAD_X;
    let all: f32 = widths.iter().map(|w| chip_w(*w)).sum::<f32>() + CHIP_GAP * widths.len().saturating_sub(1) as f32;
    if all <= room {
        return widths.len();
    }
    let mut used = 0.;
    for (k, w) in widths.iter().enumerate() {
        let next = used + if k > 0 { CHIP_GAP } else { 0. } + chip_w(*w);
        if next + CHIP_GAP + chip_w(plus_w(widths.len() - k - 1)) > room {
            return k.max(1);
        }
        used = next;
    }
    widths.len()
}

/// The rows of Details. Chip rows keep to one line, the rest behind "+N";
/// clicking it (`on_expand` with the row's label) shows them all, for the
/// rows in `open`. `measure` gives a chip text's width, `value_w` the room.
pub(super) fn details_panel(
    rows: &[DetailRow],
    active: &[(&'static str, SharedString)],
    value_w: f32,
    open: &[SharedString],
    measure: &MeasureChip,
    on_facet: Rc<dyn Fn(&'static str, SharedString, &mut Window, &mut App)>,
    on_open_folder: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
    on_expand: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
) -> Div {
    let mut p = panel(true);
    for (ri, row) in rows.iter().enumerate() {
        let label = div()
            .w(px(DETAIL_LABEL_W))
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
            DetailValue::Chips { values, facet } => {
                let shown = if open.contains(&row.label) {
                    values.len()
                } else {
                    let widths: Vec<f32> = values.iter().map(|v| measure(v)).collect();
                    chips_on_one_line(&widths, value_w, |n| measure(&format!("+{n}")))
                };
                let mut wrap = div().flex().flex_wrap().gap(px(CHIP_GAP));
                for (ci, v) in values[..shown].iter().enumerate() {
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
                if shown < values.len() {
                    let (expand, label) = (on_expand.clone(), row.label.clone());
                    wrap = wrap.child(
                        chip(format!("+{}", values.len() - shown))
                            .id(SharedString::from(format!("chip-more-{ri}")))
                            .text_color(theme::white_a(0.6))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white_a(0.2)).text_color(white()))
                            .on_click(move |_, w, cx| expand(label.clone(), w, cx)),
                    );
                }
                wrap.into_any_element()
            }
            DetailValue::Folder { path, size } => div()
                .flex()
                .flex_col()
                .gap(px(2.4))
                .child({
                    let (open_dir, target) = (on_open_folder.clone(), path.clone());
                    div()
                        .id(SharedString::from(format!("focus-folder-{ri}")))
                        .line_height(px(18.2))
                        .cursor_pointer()
                        .hover(|s| s.text_color(theme::accent()))
                        .on_click(move |_, w, cx| open_dir(target.clone(), w, cx))
                        .child(path.clone())
                })
                .children(size.clone().map(|s| div().text_size(px(12.)).text_color(theme::white_a(0.52)).child(s)))
                .into_any_element(),
        };
        p = p.child(
            div()
                .flex()
                .items_start()
                .gap(px(DETAIL_GAP))
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
            div().absolute().top(px(-3.)).left(relative(m)).ml(px(-7.)).w(px(14.)).h(px(18.)).rounded(px(6.)).bg(rgb(0x1a1c20)).shadow(
                vec![BoxShadow {
                    color: theme::black_a(0.45),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(4.),
                    spread_radius: px(0.),
                    inset: false,
                }],
            )
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
            .child(stat(tr!("Main Story"), h.main))
            .child(stat(tr!("Main + Extras"), h.extra))
            .child(stat(tr!("Completionist"), h.complete)),
    )
}

/// Cover size of "Related games", in logical px.
pub(in crate::collection) const RELATED_COVER_W: f32 = 100.;
pub(in crate::collection) const RELATED_COVER_H: f32 = 150.;

const RELATED_GAP: f32 = 12.;
/// Covers an arrow of the title moves the row by.
const RELATED_STEP: f32 = 3. * (RELATED_COVER_W + RELATED_GAP);

/// "Related games": one group at a time (`ui.tab`, an index into `groups`),
/// picked with the pills of the title; the series comes first when there is
/// one. The covers sit on one row that scrolls sideways: with the arrows of
/// the title, a sideways swipe, or the mouse wheel while the row has room to
/// move (at its ends the wheel scrolls the page again).
pub(super) fn related_panel(
    groups: &[RelatedGroupVm],
    ui: RelatedUi,
    cover_for: &CoverFor,
    on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
    on_change: Rc<dyn Fn(RelatedUi, &mut Window, &mut App)>,
) -> Div {
    let RelatedUi { tab, scroll } = ui;
    let tab = tab.min(groups.len().saturating_sub(1));
    let Some(g) = groups.get(tab) else { return panel(false) };
    let picker: AnyElement = if groups.len() > 1 {
        let labels: Vec<SharedString> = groups.iter().map(|g| g.title.clone()).collect();
        let pick = on_change.clone();
        let on_tab: Rc<dyn Fn(usize, &mut Window, &mut App)> =
            Rc::new(move |i, w, cx| pick(RelatedUi { tab: i, ..Default::default() }, w, cx));
        small_tabs("related-tab", &labels, tab, on_tab).into_any_element()
    } else {
        div().text_size(px(12.5)).text_color(theme::white_a(0.52)).child(g.title.clone()).into_any_element()
    };
    let title = h3(tr!("Related games")).flex().justify_between().items_center().gap(px(12.)).child(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .font_family(theme::font_body())
            .font_weight(theme::W_REGULAR)
            .children(scroll_arrows(&scroll))
            .child(picker),
    );
    let mut row = div()
        .id(SharedString::from(format!("rel-row-{tab}")))
        .flex()
        .gap(px(RELATED_GAP))
        .pb(px(4.))
        .overflow_x_scroll()
        // Sideways swipes only; the wheel goes through `scroll_by` below.
        .restrict_scroll_to_axis()
        .track_scroll(&scroll)
        .on_scroll_wheel({
            let scroll = scroll.clone();
            move |ev, window, cx| {
                let d = ev.delta.pixel_delta(window.line_height());
                if d.y.abs() > d.x.abs() && scroll_by(&scroll, d.y) {
                    cx.stop_propagation();
                    window.refresh();
                }
            }
        });
    for game in &g.games {
        let key = game.key.clone();
        let cb = on_select.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!("rel-{tab}-{key}")))
                .flex_none()
                .w(px(RELATED_COVER_W))
                .flex()
                .flex_col()
                .cursor_pointer()
                .on_click(move |_, w, cx| cb(key.clone(), w, cx))
                .child(
                    div()
                        .w(px(RELATED_COVER_W))
                        .h(px(RELATED_COVER_H))
                        .rounded(px(6.))
                        .overflow_hidden()
                        .bg(theme::white_a(0.06))
                        .shadow(vec![BoxShadow {
                            color: theme::black_a(0.45),
                            offset: point(px(0.), px(8.)),
                            blur_radius: px(20.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .children(cover_for(&game.cover_key).map(|src| img(src).size_full().rounded(px(6.)).object_fit(ObjectFit::Cover))),
                )
                .child(div().mt(px(7.)).text_size(px(12.)).text_color(theme::white_a(0.88)).truncate().child(game.title.clone()))
                .child(
                    div()
                        .mt(px(2.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .min_w_0()
                        .text_size(px(11.))
                        .text_color(theme::white_a(0.52))
                        .children(game.status_color.map(|c| div().size(px(6.)).flex_none().rounded_full().bg(c)))
                        .child(div().min_w_0().truncate().child(game.line.clone())),
                ),
        );
    }
    panel(false).child(title).child(row)
}

/// Moves a sideways row by `delta` (negative: towards its end), within its
/// bounds; false when it could not move (already at that end).
fn scroll_by(scroll: &ScrollHandle, delta: Pixels) -> bool {
    let (offset, max) = (scroll.offset(), scroll.max_offset().x);
    let x = (offset.x + delta).clamp(-max, px(0.));
    if x == offset.x {
        return false;
    }
    scroll.set_offset(point(x, offset.y));
    true
}

/// ‹ › of the title, while the row is wider than the panel (known from the
/// last frame); the one at an end is dimmed.
fn scroll_arrows(scroll: &ScrollHandle) -> Option<Div> {
    let max = scroll.max_offset().x;
    if max <= px(0.) {
        return None;
    }
    let x = scroll.offset().x;
    let arrow = |id: &'static str, ic: Icon, delta: f32, live: bool| {
        let scroll = scroll.clone();
        round_button(id, ic, 26., 0.35).when(!live, |d| d.opacity(0.35)).on_click(move |_, window, _| {
            if scroll_by(&scroll, px(delta)) {
                window.refresh();
            }
        })
    };
    Some(div().flex().gap(px(4.)).child(arrow("related-prev", Icon::ChevronLeft, RELATED_STEP, x < px(0.))).child(arrow(
        "related-next",
        Icon::ChevronRight,
        -RELATED_STEP,
        x > -max,
    )))
}

#[cfg(test)]
mod tests {
    use super::{chips_on_one_line, CHIP_GAP, CHIP_PAD_X};

    #[test]
    fn chips_keep_to_one_line_with_a_plus() {
        let chip = |w: f32| w + 2. * CHIP_PAD_X;
        let widths = [40., 40., 40., 40.];
        let plus = |_: usize| 10.;
        // All of them fit: no "+N".
        let all = 4. * chip(40.) + 3. * CHIP_GAP;
        assert_eq!(chips_on_one_line(&widths, all, plus), 4);
        // One pixel short: the last one makes way for "+1".
        assert_eq!(chips_on_one_line(&widths, all - 1., plus), 3);
        // Room for two and "+2", not for three and "+1".
        assert_eq!(chips_on_one_line(&widths, 2. * chip(40.) + 2. * CHIP_GAP + chip(10.), plus), 2);
        // Never fewer than one, even in a tiny room.
        assert_eq!(chips_on_one_line(&widths, 5., plus), 1);
        assert_eq!(chips_on_one_line(&[], 100., plus), 0);
    }
}
