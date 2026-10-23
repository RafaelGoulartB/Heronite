//! The desktop look of Settings → Storage: a bar per disk, the "Free up
//! space" card, the games as a treemap of their art over a list, then a
//! list per kind of thing (prefixes, Proton, Steam leftovers, caches,
//! leftover folders), each row with a bar of its size.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_i18n::{tr, trn};
use heronite_storage::{Group, Item, Kind, State as Use, Sweep};

use super::treemap::squarify;
use super::{
    days_since, group_color, group_label, item_meta, open, played_text, remove, seg_color, seg_label, size_text, stale, sweep_color,
    sweep_effect, sweep_label, unused_color, Confirm, GameSort, Hover, Seg, State,
};
use crate::icons::Icon;
use crate::pages::settings::SettingsPage;
use crate::pages::{empty_row, icon_action, list_item, status_pill, PageEvent};
use crate::services::widgets::{busy_bar, cover_box, progress_bar};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card_box, mini_checkbox, tabs, ButtonKind};
use crate::ui::icon;

/// Widest the section's column gets (the treemap wants the room).
pub(in crate::pages::settings) const MAX_W: f32 = 1080.;

/// Rows a list shows before "Show all".
const ROWS: usize = 6;

/// Games with a tile of their own; the rest share one.
const TILES: usize = 12;

fn hover(h: Hover, cx: &mut Context<SettingsPage>) -> impl Fn(&bool, &mut Window, &mut App) + 'static + use<> {
    let page = cx.entity().downgrade();
    move |on: &bool, _: &mut Window, cx: &mut App| {
        let on = *on;
        page.update(cx, |d, cx| {
            if on {
                d.storage.hover = Some(h.clone());
            } else if d.storage.hover.as_ref() == Some(&h) {
                d.storage.hover = None;
            }
            cx.notify();
        })
        .ok();
    }
}

fn dot(color: Hsla, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded_full().bg(color)
}

fn strong(text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .font_family(theme::font_heading())
        .font_weight(theme::W_BOLD)
        .text_size(px(size))
        .text_color(theme::text_primary())
        .child(text.into())
}

fn dim(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).text_color(theme::text_dim()).child(text.into())
}

/// A thin bar of `fraction` (0..=1) of the row's width.
fn size_bar(fraction: f32, color: Hsla) -> Div {
    div()
        .w_full()
        .h(px(4.))
        .mt(px(5.))
        .rounded_full()
        .bg(theme::ink_a(0.06))
        .child(div().h_full().rounded_full().bg(color).w(relative(fraction.clamp(0.012, 1.))))
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    if d.storage.inv.is_none() {
        return card_box()
            .py(px(18.))
            .gap(px(12.))
            .child(dim(tr!("Looking at your disks\u{2026}")))
            .child(busy_bar("storage-loading", 4.))
            .into_any_element();
    }
    // The column: the window minus the sheet's inset, the nav and the padding.
    let column = (f32::from(window.viewport_size().width) - 24. - 230. - 72.).min(MAX_W);
    let mut col =
        div().flex().flex_col().child(overview(&d.storage, cx)).child(free_up(&d.storage, cx)).child(games(d, column, window, cx));
    for group in [Group::Prefixes, Group::Builds, Group::Steam, Group::Caches, Group::Leftovers] {
        col = col.children(group_block(d, group, window, cx));
    }
    col.into_any_element()
}

/// A bar per disk: what each kind of thing takes on it, the rest, the free
/// space; hovering a part names it.
fn overview(st: &State, cx: &mut Context<SettingsPage>) -> Div {
    let Some(inv) = st.inv.as_ref() else { return div() };
    let mut card = card_box().py(px(14.)).gap(px(16.));
    let mut order: Vec<usize> = (0..inv.disks.len()).collect();
    // The disk holding most of the games first.
    order.sort_by_key(|i| std::cmp::Reverse(st.items().filter(|it| it.disk == Some(*i)).filter_map(|it| st.own(it)).sum::<u64>()));
    for i in order {
        let disk = &inv.disks[i];
        let parts = st.disk_parts(i);
        let total = disk.total.max(1) as f32;
        let right = match &st.hover {
            Some(Hover::Segment(h, seg)) if *h == i => {
                let bytes = parts.iter().find(|(s, _)| s == seg).map(|(_, b)| *b).unwrap_or(0);
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(dot(seg_color(*seg), 8.))
                    .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(seg_label(*seg)))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(human_size(bytes)))
            }
            _ => div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!(
                "{free} free of {total}",
                free = human_size(disk.free),
                total = human_size(disk.total)
            )),
        };
        let bar = div().flex().w_full().h(px(14.)).rounded(px(7.)).overflow_hidden().bg(theme::chart_empty()).children(
            parts.iter().filter(|(_, b)| *b > 0).map(|(seg, bytes)| {
                let on = st.hover == Some(Hover::Segment(i, *seg));
                div()
                    .id(SharedString::from(format!("st-seg-{i}-{seg:?}")))
                    .h_full()
                    .w(relative(*bytes as f32 / total))
                    .bg(seg_color(*seg))
                    .when(*seg != Seg::Free, |d| d.border_r_1().border_color(theme::black_a(0.4)))
                    .when(on, |d| d.opacity(0.75))
                    .on_hover(hover(Hover::Segment(i, *seg), cx))
            }),
        );
        card = card.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(9.))
                                .min_w_0()
                                .child(icon(Icon::FaHardDrive, px(15.), theme::text_muted()))
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .text_color(theme::text_primary())
                                        .truncate()
                                        .child(disk.mount.display().to_string()),
                                )
                                .child(dim(disk.fs.clone())),
                        )
                        .child(right),
                )
                .child(bar),
        );
    }
    let mut legend = div().flex().flex_wrap().gap_x(px(18.)).gap_y(px(6.));
    for g in Group::ALL {
        let bytes = st.group_bytes(g);
        if bytes > 0 {
            legend = legend.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(dot(group_color(g), 8.))
                    .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(group_label(g)))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(human_size(bytes))),
            );
        }
    }
    card.child(legend)
}

/// The space the ticked choices free, the choices, and the button.
fn free_up(st: &State, cx: &mut Context<SettingsPage>) -> AnyElement {
    let parts: Vec<(Sweep, usize, u64)> = Sweep::ALL
        .into_iter()
        .filter_map(|s| {
            let (items, bytes) = st.sweep_parts(s);
            (!items.is_empty()).then_some((s, items.len(), bytes))
        })
        .collect();
    let ticked = st.ticked_bytes();
    let total: u64 = parts.iter().map(|(_, _, b)| *b).sum();
    let body = card_box().py(px(16.)).gap(px(14.));
    if parts.is_empty() && !st.loading {
        return div()
            .child(caption(tr!("Free up space")))
            .child(
                body.flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child(icon(Icon::Check, px(18.), theme::success()))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Nothing to clean up"))),
            )
            .into_any_element();
    }

    let confirming = st.confirm == Some(Confirm::Sweep);
    let enabled = ticked > 0 && !st.busy();
    let button: AnyElement = if confirming {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(action_button(
                "st-sweep-cancel",
                ButtonKind::Ghost,
                tr!("Cancel"),
                true,
                cx.listener(|d, _, _, cx| {
                    d.storage.confirm = None;
                    cx.notify();
                }),
            ))
            .child(action_button(
                "st-sweep-yes",
                ButtonKind::Danger,
                tr!("Free {size}", size = human_size(ticked)),
                enabled,
                cx.listener(|d, _, _, cx| {
                    let st = &d.storage;
                    let ids = st.items().filter(|i| i.sweep().is_some_and(|s| st.sweeps.contains(&s))).map(|i| i.id.clone()).collect();
                    remove(d, ids, cx);
                }),
            ))
            .into_any_element()
    } else {
        let label = if st.removing { tr!("Freeing up\u{2026}").into() } else { tr!("Free {size}", size = human_size(ticked)) };
        action_button(
            "st-sweep",
            ButtonKind::Primary,
            label,
            enabled,
            cx.listener(|d, _, _, cx| {
                d.storage.confirm = Some(Confirm::Sweep);
                cx.notify();
            }),
        )
        .into_any_element()
    };

    let headline = div()
        .flex()
        .items_end()
        .justify_between()
        .gap(px(16.))
        .child(div().flex().flex_col().gap(px(2.)).child(strong(human_size(ticked), 34.)).child(
            div().text_size(px(13.)).text_color(theme::text_muted()).child(if ticked == total {
                tr!("can be freed").to_string()
            } else {
                tr!("selected, of {total} that can be freed", total = human_size(total))
            }),
        ))
        .child(button);

    // How the total splits, in the choices' colours.
    let split = div().flex().w_full().h(px(8.)).rounded(px(4.)).overflow_hidden().bg(theme::chart_empty()).children(
        parts.iter().filter(|(_, _, b)| *b > 0).map(|(s, _, b)| {
            let on = st.sweeps.contains(s);
            div()
                .h_full()
                .w(relative(*b as f32 / total.max(1) as f32))
                .bg(Hsla { a: if on { 1. } else { 0.25 }, ..sweep_color(*s) })
                .border_r_1()
                .border_color(theme::black_a(0.4))
        }),
    );

    let mut choices = div().flex().flex_col();
    for (s, count, bytes) in &parts {
        let (s, on, open) = (*s, st.sweeps.contains(s), st.open_sweeps.contains(s));
        let chevron = div()
            .id(SharedString::from(format!("st-sweep-more-{s:?}")))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded_full()
            .cursor_pointer()
            .hover(|d| d.bg(theme::ink_a(0.08)))
            .on_click(cx.listener(move |d, _, _, cx| {
                // Not a click on the row: the choice stays as it is.
                cx.stop_propagation();
                let st = &mut d.storage;
                if !st.open_sweeps.remove(&s) {
                    st.open_sweeps.insert(s);
                }
                cx.notify();
            }))
            .child(
                icon(Icon::ExpandMore, px(20.), theme::text_muted())
                    .when(open, |i| i.with_transformation(Transformation::rotate(percentage(0.5)))),
            );
        choices = choices.child(
            div()
                .id(SharedString::from(format!("st-sweep-{s:?}")))
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(7.))
                .mx(px(-8.))
                .px(px(8.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|d| d.bg(theme::ink_a(0.04)))
                .on_click(cx.listener(move |d, _, _, cx| {
                    let st = &mut d.storage;
                    if !st.sweeps.remove(&s) {
                        st.sweeps.insert(s);
                    }
                    st.confirm = None;
                    cx.notify();
                }))
                .child(mini_checkbox(SharedString::from(format!("st-sweep-box-{s:?}")), on, false))
                .child(dot(sweep_color(s), 8.))
                .child(div().flex_1().min_w_0().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(sweep_label(s)))
                .child(dim(count.to_string()))
                .child(div().w(px(84.)).flex().justify_end().text_size(px(13.)).text_color(theme::text_muted()).child(human_size(*bytes)))
                .child(chevron),
        );
        if open {
            choices = choices.child(sweep_details(st, s));
        }
    }

    let mut card = body.child(headline).child(split);
    if let Some((done, all)) = st.walking.filter(|(_, all)| *all > 0) {
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().child(progress_bar(Some(done as f32 * 100. / all as f32), 3., theme::accent())))
                .child(dim(tr!("Measuring {done} of {total}", done, total = all))),
        );
    }
    card = card.child(choices);
    if confirming {
        let trashes = st.items().any(|i| i.goes_to_trash() && i.sweep().is_some_and(|s| st.sweeps.contains(&s)));
        if trashes {
            card = card.child(dim(tr!("Prefixes and leftover folders go to the trash, where they can be restored; the rest is deleted.")));
        }
    }
    div().child(caption(tr!("Free up space"))).child(card).into_any_element()
}

/// Under an open "Free up space" choice: what freeing it does, then each
/// thing it takes with its folders and size.
fn sweep_details(st: &State, s: Sweep) -> Div {
    let (mut items, _) = st.sweep_parts(s);
    items.sort_by_key(|i| std::cmp::Reverse(st.own(i).unwrap_or(0)));
    // Lined up with the choice's label (switch, gap, dot, gap) and its size
    // (the chevron's room on the right).
    let mut list = div().flex().flex_col().ml(px(62.)).mr(px(40.)).mb(px(8.)).pl(px(12.)).border_l_1().border_color(theme::ink_a(0.1));
    list = list.child(div().pb(px(6.)).text_size(px(12.)).line_height(px(17.)).text_color(theme::text_muted()).child(sweep_effect(s)));
    for item in items {
        let action = if item.goes_to_trash() { tr!("To the trash") } else { tr!("Deleted") };
        let paths = item.paths.iter().map(|p| dim(p.display().to_string()).truncate());
        list = list.child(
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .py(px(5.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(item.title.clone()))
                        .children(paths),
                )
                .child(dim(action).flex_none())
                .child(
                    div()
                        .w(px(84.))
                        .flex()
                        .flex_none()
                        .justify_end()
                        .text_size(px(13.))
                        .text_color(theme::text_muted())
                        .child(size_text(st.own(item))),
                ),
        );
    }
    list
}

/// Sorted for a list: what can go first, then the biggest.
fn sorted<'a>(st: &State, mut items: Vec<&'a Item>) -> Vec<&'a Item> {
    items.sort_by_key(|i| (i.state != Use::Unused, std::cmp::Reverse(st.own(i).unwrap_or(0))));
    items
}

/// The games: the treemap and the list.
fn games(d: &mut SettingsPage, column: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let st = &d.storage;
    let mut games: Vec<Item> = st.group_items(Group::Games).into_iter().cloned().collect();
    let total: u64 = games.iter().filter_map(|i| i.size).sum();
    let count = games.len();
    let sort = st.sort;
    let page = cx.entity().downgrade();
    let bar = tabs("st-sort", &[(GameSort::Size, tr!("Size")), (GameSort::Played, tr!("Last played"))], sort, move |s, _, cx| {
        page.update(cx, |d, cx| {
            d.storage.sort = s;
            cx.notify();
        })
        .ok();
    });
    let title =
        trn!(count, "Games \u{b7} {n} installed \u{b7} {size}", "Games \u{b7} {n} installed \u{b7} {size}", size = human_size(total));
    if games.is_empty() {
        return div().child(caption(tr!("Games"))).child(card_box().child(empty_row(tr!("No installed games")))).into_any_element();
    }
    let tree = treemap(d, &games, column - 34., window, cx);

    let st = &d.storage;
    match sort {
        GameSort::Size => games.sort_by_key(|i| std::cmp::Reverse(i.size.unwrap_or(0))),
        // Least recently played first: what to uninstall.
        GameSort::Played => games.sort_by_key(|i| std::cmp::Reverse(days_since(i.last_played.as_deref()).unwrap_or(i64::MAX))),
    }
    let biggest = games.iter().filter_map(|i| i.size).max().unwrap_or(1).max(1);
    let expanded = st.expanded.contains(&Group::Games);
    let shown: Vec<Item> = games.iter().take(if expanded { usize::MAX } else { ROWS }).cloned().collect();
    let scale = window.scale_factor();
    let mut list = card_box().py(px(2.));
    for (n, item) in shown.iter().enumerate() {
        let Kind::Game { key } = &item.kind else { continue };
        let days = days_since(item.last_played.as_deref());
        let old = stale(days);
        let cover = d
            .storage
            .chains
            .get(key.as_str())
            .cloned()
            .and_then(|chain| d.storage.covers.as_mut()?.get_chain(&format!("st:{key}"), &chain, 30., 40., scale));
        let source = item.source.clone().map(|s| format!("{s} \u{b7} ")).unwrap_or_default();
        let content = div()
            .flex()
            .flex_col()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(item.title.clone()))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(if old { Hsla::from(unused_color()) } else { theme::text_dim().into() })
                    .child(format!("{source}{}", played_text(days))),
            )
            .child(size_bar(item.size.unwrap_or(0) as f32 / biggest as f32, group_color(Group::Games)));
        let row = list_item(Some(cover_box(cover, 30., 40.).into_any_element()), content, trailing(d, item, cx));
        if n > 0 {
            list = list.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
        }
        list = list.child(row);
    }
    if games.len() > ROWS {
        list = list.child(show_all(Group::Games, expanded, games.len(), cx));
    }
    // As tall as the switch: a caption's fixed height would let it overlap the card.
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .mt(px(14.))
        .mb(px(8.))
        .child(div().mx(px(6.)).text_size(px(12.)).text_color(theme::text_muted()).truncate().child(title))
        .child(bar);
    div().child(header).child(tree).child(div().h(px(10.))).child(list).into_any_element()
}

/// The biggest games as tiles of their art, sized by the space they take.
fn treemap(d: &mut SettingsPage, games: &[Item], width: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let mut sized: Vec<&Item> = games.iter().filter(|i| i.size.is_some_and(|s| s > 0)).collect();
    sized.sort_by_key(|i| std::cmp::Reverse(i.size.unwrap_or(0)));
    let height = (width * 0.42).clamp(220., 380.);
    let total: u64 = sized.iter().filter_map(|i| i.size).sum();
    let (own, rest) = sized.split_at(sized.len().min(TILES));
    let rest_bytes: u64 = rest.iter().filter_map(|i| i.size).sum();
    let mut values: Vec<f64> = own.iter().map(|i| i.size.unwrap_or(0) as f64).collect();
    if !rest.is_empty() {
        // Big enough to read, however small the games.
        values.push((rest_bytes as f64).max(total as f64 * 0.04));
    }
    let tiles = squarify(&values, width, height);
    let scale = window.scale_factor();
    let hovered = match &d.storage.hover {
        Some(Hover::Tile(id)) => d.storage.item(id).cloned(),
        _ => None,
    };
    let line = match &hovered {
        Some(item) => {
            format!("{} \u{b7} {} \u{b7} {}", item.title, size_text(item.size), played_text(days_since(item.last_played.as_deref())))
        }
        None => trn!(sized.len(), "{n} game \u{b7} {size}", "{n} games \u{b7} {size}", size = human_size(total)),
    };
    let mut area = div().relative().w(px(width)).h(px(height));
    for t in &tiles {
        let (x, y, w, h) = (t.x + 2., t.y + 2., (t.w - 4.).max(0.), (t.h - 4.).max(0.));
        let Some(item) = own.get(t.index) else {
            area = area.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(10.))
                    .bg(theme::ink_a(0.06))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .when(h > 40. && w > 56., |d| d.child(strong(format!("+{}", rest.len()), 15.)).child(dim(human_size(rest_bytes)))),
            );
            continue;
        };
        let Kind::Game { key } = &item.kind else { continue };
        let chain = d.storage.stages.get(key.as_str()).filter(|c| !c.is_empty()).or_else(|| d.storage.chains.get(key.as_str())).cloned();
        let art = chain.and_then(|c| d.storage.covers.as_mut()?.get_art(&format!("st-tile:{key}"), &c, w.max(1.), h.max(1.), scale));
        let on = hovered.as_ref().is_some_and(|h| h.id == item.id);
        let label = h >= 46. && w >= 76.;
        area = area.child(
            div()
                .id(SharedString::from(format!("st-tile-{}", t.index)))
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w))
                .h(px(h))
                .rounded(px(10.))
                .overflow_hidden()
                .bg(rgb(0x11151c))
                .on_hover(hover(Hover::Tile(item.id.clone()), cx))
                .children(art.map(|s| img(s).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)))
                .child(div().absolute().inset_0().bg(theme::black_a(if on { 0. } else { 0.18 })))
                .when(label, |d| {
                    d.child(div().absolute().left_0().right_0().bottom_0().h(relative(0.7)).bg(linear_gradient(
                        180.,
                        linear_color_stop(theme::black_a(0.), 0.),
                        linear_color_stop(theme::black_a(0.82), 1.),
                    )))
                    .child(
                        div()
                            .absolute()
                            .left(px(10.))
                            .right(px(10.))
                            .bottom(px(8.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(theme::W_BOLD)
                                    .text_size(px(if w > 180. { 14. } else { 12. }))
                                    .text_color(white())
                                    .truncate()
                                    .child(item.title.clone()),
                            )
                            .child(div().text_size(px(12.)).text_color(theme::white_a(0.7)).child(size_text(item.size))),
                    )
                })
                .when(stale(days_since(item.last_played.as_deref())), |d| {
                    d.child(dot(unused_color().into(), 8.).absolute().top(px(9.)).right(px(9.)))
                })
                .when(on, |d| d.border_2().border_color(theme::accent())),
        );
    }
    card_box().py(px(12.)).gap(px(10.)).child(div().text_size(px(13.)).text_color(theme::text_muted()).truncate().child(line)).child(area)
}

/// One kind of thing other than games: a caption with its total, the rows.
fn group_block(d: &mut SettingsPage, group: Group, window: &mut Window, cx: &mut Context<SettingsPage>) -> Option<AnyElement> {
    let st = &d.storage;
    let items: Vec<Item> = sorted(st, st.group_items(group)).into_iter().cloned().collect();
    if items.is_empty() {
        return None;
    }
    let bytes = st.group_bytes(group);
    let biggest = items.iter().filter_map(|i| st.own(i)).max().unwrap_or(1).max(1);
    let expanded = st.expanded.contains(&group);
    let scale = window.scale_factor();
    let mut list = card_box().py(px(2.));
    for (n, item) in items.iter().take(if expanded { usize::MAX } else { ROWS }).enumerate() {
        let own = d.storage.own(item);
        let leading = leading(d, item, scale);
        let mut title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(item.title.clone()));
        if item.state == Use::Unused {
            title = title.child(status_pill(tr!("Unused"), unused_color()));
        }
        if matches!(item.kind, Kind::Prefix { default: true } | Kind::Build { default: true, .. }) {
            title = title.child(status_pill(tr!("Default"), theme::accent()));
        }
        let color = if item.state == Use::Unused { Hsla::from(unused_color()) } else { group_color(group) };
        let content = div()
            .flex()
            .flex_col()
            .child(title)
            .child(dim(item_meta(&d.storage, item)).truncate())
            .child(size_bar(own.unwrap_or(0) as f32 / biggest as f32, color));
        if n > 0 {
            list = list.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
        }
        list = list.child(list_item(Some(leading), content, trailing(d, item, cx)));
    }
    if items.len() > ROWS {
        list = list.child(show_all(group, expanded, items.len(), cx));
    }
    Some(div().child(caption(format!("{} \u{b7} {}", group_label(group), human_size(bytes)))).child(list).into_any_element())
}

/// The covers of the games using a prefix or build (up to three), else the
/// kind's icon.
fn leading(d: &mut SettingsPage, item: &Item, scale: f32) -> AnyElement {
    let keys: Vec<String> = item.games.iter().take(3).cloned().collect();
    if !keys.is_empty() {
        let mut stack = div().relative().flex_none().w(px(32. + 10. * (keys.len() as f32 - 1.))).h(px(40.));
        for (i, key) in keys.iter().enumerate() {
            let chain = d.storage.chains.get(key.as_str()).cloned();
            let src = chain.and_then(|c| d.storage.covers.as_mut()?.get_chain(&format!("st:{key}"), &c, 30., 40., scale));
            stack = stack
                .child(cover_box(src, 30., 40.).absolute().left(px(10. * i as f32)).top_0().border_1().border_color(theme::black_a(0.6)));
        }
        return stack.into_any_element();
    }
    let ic = match item.kind {
        Kind::Prefix { .. } | Kind::Build { .. } => Icon::WineBar,
        Kind::Runtime => Icon::Build,
        Kind::SteamPrefix { .. } | Kind::SteamShaders { .. } => Icon::SportsEsports,
        Kind::Cache(_) => Icon::Refresh,
        Kind::Leftover | Kind::Game { .. } => Icon::Folder,
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(px(9.))
        .bg(Hsla { a: 0.16, ..group_color(item.group) })
        .child(icon(ic, px(16.), group_color(item.group)))
        .into_any_element()
}

/// Size, open, remove (or uninstall); the inline confirmation in its place.
fn trailing(d: &SettingsPage, item: &Item, cx: &mut Context<SettingsPage>) -> AnyElement {
    let st = &d.storage;
    let id = item.id.clone();
    let tag = |what: &str| SharedString::from(format!("st-{what}-{id}"));
    if st.confirm.as_ref() == Some(&Confirm::Item(id.clone())) {
        let label = if item.goes_to_trash() { tr!("Move to the trash") } else { tr!("Delete") };
        let target = id.clone();
        return div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(button_sm(tag("no"), ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|d, _, _, cx| {
                d.storage.confirm = None;
                cx.notify();
            })))
            .child(button_sm(tag("yes"), ButtonKind::Danger, label).on_click(cx.listener(move |d, _, _, cx| {
                remove(d, vec![target.clone()], cx);
            })))
            .into_any_element();
    }
    let size = div().w(px(76.)).flex().justify_end().text_size(px(13.)).text_color(theme::text_muted()).child(size_text(st.own(item)));
    let folder = item.clone();
    let open_btn = icon_action(tag("open"), Icon::FolderOpen, true).on_click(move |_, _, _| open(&folder));
    let remove_btn: AnyElement = match &item.kind {
        Kind::Game { key } => {
            let key = key.clone();
            icon_action(tag("rm"), Icon::DeleteOutline, !st.busy())
                .when(!st.busy(), |b| b.on_click(cx.listener(move |_, _, _, cx| cx.emit(PageEvent::Uninstall(key.clone())))))
                .into_any_element()
        }
        _ if item.removable() => {
            let enabled = !st.busy();
            icon_action(tag("rm"), Icon::DeleteOutline, enabled)
                .when(enabled, |b| {
                    b.on_click(cx.listener(move |d, _, _, cx| {
                        d.storage.confirm = Some(Confirm::Item(id.clone()));
                        cx.notify();
                    }))
                })
                .into_any_element()
        }
        // Keeps the columns lined up.
        _ => div().size(px(32.)).into_any_element(),
    };
    div().flex().items_center().gap(px(6.)).child(size).child(open_btn).child(remove_btn).into_any_element()
}

fn show_all(group: Group, expanded: bool, count: usize, cx: &mut Context<SettingsPage>) -> Div {
    let label = if expanded { tr!("Show fewer").into() } else { tr!("Show all {n}", n = count) };
    div().flex().justify_center().py(px(8.)).border_t_1().border_color(theme::row_separator()).child(
        button_sm(SharedString::from(format!("st-all-{group:?}")), ButtonKind::Ghost, label).on_click(cx.listener(move |d, _, _, cx| {
            let st = &mut d.storage;
            if !st.expanded.remove(&group) {
                st.expanded.insert(group);
            }
            cx.notify();
        })),
    )
}
