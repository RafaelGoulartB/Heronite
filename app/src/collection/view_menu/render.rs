//! The View menu's panel: search box with tokens, then either the results
//! or Views / Sort / Group by / Show / Only / Browse, and a footer with the
//! count, "Reset all" and "Statuses…".

use gpui_kit::component::input::Input;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{InstallFilter, SortMode};
use heronite_i18n::tr;

use super::super::header::Menu;
use super::super::view_filters::Only;
use super::super::CollectionView;
use super::choices::{self, Kind, Row};
use super::Pick;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::menu_panel;
use crate::ui::icon;

const W: f32 = 404.;
/// Inner padding of the panel's sections.
const PAD: f32 = 12.;

impl CollectionView {
    pub(in crate::collection) fn view_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let searching = !self.view_menu.query.trim().is_empty();
        let body = div()
            .id("vm-body")
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(PAD))
            .min_h_0()
            .overflow_y_scroll()
            .children(searching.then(|| self.vm_results(cx)))
            .when(!searching, |d| {
                d.child(self.vm_views(cx))
                    .child(self.vm_sort(cx))
                    .child(self.vm_group(cx))
                    .child(self.vm_show(cx))
                    .child(self.vm_only(cx))
                    .child(self.vm_browse(cx))
            });
        menu_panel(W)
            .w(px(W))
            .p(px(0.))
            .gap(px(0.))
            .max_h(px(self.view_menu.room))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.open_menu == Some(Menu::View) {
                    this.open_menu = None;
                    cx.notify();
                }
            }))
            .child(self.vm_query_box(cx))
            .child(body)
            .child(self.vm_footer(cx))
            .into_any_element()
    }

    fn vm_query_box(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let tokens = self.view_tokens().into_iter().enumerate().map(|(i, (kind, c))| {
            let pick = c.pick.clone();
            div()
                .id(("vm-tok", i))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .h(px(26.))
                .pl(px(9.))
                .pr(px(5.))
                .max_w(px(260.))
                .rounded(px(8.))
                .bg(theme::accent_tint())
                .text_size(px(12.5))
                .text_color(white())
                .cursor_pointer()
                .hover(|s| s.bg(theme::accent_tint_border()))
                .children(c.color.map(dot))
                .children(kind.map(|k| div().flex_none().text_color(theme::text_muted()).child(format!("{}:", k.to_lowercase()))))
                .child(div().min_w_0().truncate().child(c.label))
                .child(icon(Icon::Close, px(12.), theme::white_a(0.7)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.apply_pick(&pick, cx);
                    this.focus_view_search(window, cx);
                }))
        });
        div()
            .id("vm-query")
            .m(px(PAD))
            .mb(px(0.))
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .min_h(px(44.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(theme::R_TILE))
            .bg(theme::black_a(0.28))
            .border_1()
            .border_color(if self.view_menu.focused { theme::input_border_focus() } else { theme::white_a(0.08) })
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| this.focus_view_search(window, cx)))
            .child(icon(Icon::FaSearch, px(13.), theme::text_dim()))
            .children(tokens)
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .text_size(px(14.))
                    .text_color(white())
                    .child(Input::new(&self.view_menu.input).appearance(false)),
            )
    }

    fn vm_results(&self, cx: &mut Context<Self>) -> Div {
        let found = self.view_results();
        let mut list = div().flex().flex_col();
        if found.is_empty() {
            let q = self.view_menu.query.trim().to_string();
            list = list.child(
                div()
                    .py(px(18.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(theme::text_dim())
                    .child(tr!("No filter matches “{query}”", query = q).to_string()),
            );
        }
        let highlight = self.view_menu.highlight;
        let mut n = 0;
        for f in found {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(10.))
                    .pt(px(8.))
                    .pb(px(4.))
                    .text_size(px(11.5))
                    .text_color(theme::text_dim())
                    .child(icon(f.kind.icon(), px(13.), theme::text_dim()))
                    .child(f.kind.label().to_uppercase()),
            );
            for row in f.rows {
                let lit = n == highlight;
                list = list.child(self.vm_result_row(n, row, lit, cx));
                n += 1;
            }
        }
        if choices::parse_query(&self.view_menu.query).0 == Some(Kind::Category) {
            list = list.child(
                plain_row("vm-manage-cats", false)
                    .child(icon(Icon::Label, px(15.), theme::text_muted()))
                    .child(tr!("Manage categories…"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_manage_categories(window, cx))),
            );
        }
        list
    }

    fn vm_result_row(&self, n: usize, row: Row, lit: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let el = plain_row(("vm-row", n), lit);
        let el = match &row {
            Row::Choice(c) => el
                .child(check(c.on))
                .children(c.color.map(dot))
                .child(div().flex_1().min_w_0().truncate().when(c.on, |d| d.text_color(white())).child(c.label.clone()))
                .children(c.count.map(count_label))
                .when(c.count == Some(0) && !c.on, |d| d.opacity(0.45)),
            Row::All { kind, matches } => el.text_color(theme::accent()).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(tr!("All {n} in {kind}…", n = matches, kind = kind.label().to_lowercase()).to_string()),
            ),
        };
        el.on_click(cx.listener(move |this, _, window, cx| {
            this.view_menu.highlight = n;
            this.activate_row(row.clone(), window, cx);
        }))
    }

    fn vm_views(&self, cx: &mut Context<Self>) -> Div {
        let cards: Vec<Stateful<Div>> = self
            .view_cards()
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let on = self.ui.shows_view(&v.filters);
                let filters = v.filters.clone();
                div()
                    .id(("vm-view", i))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .flex_1()
                    .min_w_0()
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(12.))
                    .border_1()
                    .cursor_pointer()
                    .border_color(if on { theme::accent_tint_border() } else { theme::white_a(0.06) })
                    .bg(if on { theme::accent_tint() } else { theme::white_a(0.05) })
                    .hover(|s| s.bg(theme::white_a(0.09)))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(8.))
                            .bg(Rgba { a: 0.16, ..v.color })
                            .child(icon(v.icon, px(16.), v.color)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .child(div().truncate().text_size(px(13.5)).text_color(white()).child(v.name))
                            .child(div().truncate().text_size(px(11.5)).text_color(theme::text_dim()).child(v.sub)),
                    )
                    .children(v.saved.map(|index| {
                        div()
                            .id(("vm-view-del", i))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded_full()
                            .hover(|s| s.bg(theme::white_a(0.12)))
                            .child(icon(Icon::Close, px(12.), theme::white_a(0.55)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.delete_view(index, cx);
                            }))
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.apply_view_filters(&filters, cx)))
            })
            .collect();
        // Two per row; an odd last card keeps half the width.
        let mut rows = Vec::new();
        let mut cards = cards.into_iter();
        while let Some(first) = cards.next() {
            let second = cards.next().map(IntoElement::into_any_element).unwrap_or_else(|| div().flex_1().into_any_element());
            rows.push(div().flex().gap(px(8.)).child(first).child(second));
        }
        let save: AnyElement = match &self.view_menu.naming {
            Some((input, _)) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .w_full()
                .child(
                    div()
                        .flex_1()
                        .h(px(36.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .rounded(px(10.))
                        .bg(theme::input_fill())
                        .border_1()
                        .border_color(theme::input_border_focus())
                        .text_size(px(13.5))
                        .child(div().flex_1().child(Input::new(input).appearance(false))),
                )
                .child(
                    crate::ui::controls::button_sm("vm-view-save", crate::ui::controls::ButtonKind::Primary, tr!("Save"))
                        .on_click(cx.listener(|this, _, window, cx| this.save_named_view(window, cx))),
                )
                .into_any_element(),
            None => div()
                .id("vm-view-new")
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .w_full()
                .h(px(36.))
                .rounded(px(12.))
                .border_1()
                .border_dashed()
                .border_color(theme::white_a(0.16))
                .text_size(px(13.))
                .text_color(theme::text_muted())
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.05)).text_color(white()))
                .child(icon(Icon::Add, px(15.), theme::text_muted()))
                .child(tr!("Save current as a view"))
                .on_click(cx.listener(|this, _, window, cx| this.start_naming_view(window, cx)))
                .into_any_element(),
        };
        section(tr!("Views")).child(div().flex().flex_col().gap(px(8.)).children(rows).child(save))
    }

    fn vm_sort(&self, cx: &mut Context<Self>) -> Div {
        let items = [
            (SortMode::Title, tr!("Title"), Icon::Label),
            (SortMode::LastPlayed, tr!("Last played"), Icon::History),
            (SortMode::Playtime, tr!("Playtime"), Icon::AccessTime),
        ];
        let seg = seg_track("vm-sort").children(items.into_iter().map(|(mode, label, ic)| {
            let on = self.ui.sort == mode;
            seg_item(("vm-sort", mode as usize), on)
                .child(icon(ic, px(15.), if on { white().into() } else { theme::seg_text() }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.sort = mode;
                    this.save_ui();
                    this.refresh_groups();
                    cx.notify();
                }))
        }));
        let descending = self.ui.sort_descending;
        let dir = div()
            .id("vm-sort-dir")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(36.))
            .rounded_full()
            .bg(theme::seg_track())
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.12)))
            .child(icon(if descending { Icon::ArrowDownward } else { Icon::ArrowUpward }, px(17.), theme::text_primary()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sort_direction(cx)));
        section(tr!("Sort")).child(div().flex().items_center().gap(px(6.)).child(seg.flex_1()).child(dir))
    }

    fn vm_group(&self, cx: &mut Context<Self>) -> Div {
        let grouped = self.ui.group_by_status;
        let colors: Vec<Rgba> =
            self.lib.as_deref().map(|l| l.statuses.iter().take(5).map(|s| theme::status_color(&s.slug)).collect()).unwrap_or_default();
        let dots = div().flex().flex_none().items_center().children(colors.into_iter().enumerate().map(|(i, c)| {
            div().size(px(8.)).rounded_full().bg(c).border_1().border_color(theme::rgb_a(20, 22, 30, 0.9)).when(i > 0, |d| d.ml(px(-3.)))
        }));
        let set = |on: bool| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                if this.ui.group_by_status != on {
                    this.ui.group_by_status = on;
                    this.save_ui();
                    this.refresh_groups();
                    cx.notify();
                }
            })
        };
        section(tr!("Group by")).child(
            seg_track("vm-group")
                .child(seg_item("vm-group-none", !grouped).child(tr!("None")).on_click(set(false)))
                .child(seg_item("vm-group-status", grouped).child(dots).child(tr!("Status")).on_click(set(true))),
        )
    }

    fn vm_show(&self, cx: &mut Context<Self>) -> Div {
        let (all, installed) = self.show_counts();
        let items = [
            (InstallFilter::All, tr!("All"), all),
            (InstallFilter::Installed, tr!("Installed"), installed),
            (InstallFilter::Uninstalled, tr!("Not installed"), all - installed),
        ];
        section(tr!("Show")).child(seg_track("vm-show").children(items.into_iter().map(|(f, label, n)| {
            let on = self.ui.install_filter == f;
            seg_item(("vm-show", f as usize), on)
                .child(label)
                .child(div().text_size(px(11.)).text_color(if on { theme::text_primary() } else { theme::text_dim() }).child(n.to_string()))
                .on_click(cx.listener(move |this, _, _, cx| this.set_install_filter(f, cx)))
        })))
    }

    fn vm_only(&self, cx: &mut Context<Self>) -> Div {
        let tiles = [
            (Pick::Only(Only::Favourites), tr!("Favourites"), Icon::Star, self.ui.only.favourites),
            (Pick::Only(Only::Updates), tr!("Update"), Icon::Update, self.ui.only.updates),
            (Pick::Only(Only::Offline), tr!("Offline"), Icon::WifiOff, self.ui.only.offline),
            (Pick::Hidden, tr!("Hidden"), Icon::VisibilityOff, self.ui.show_hidden),
        ];
        let tile_w = (W - PAD * 2. - 18.) / 4.;
        section(tr!("Only")).child(div().flex().gap(px(6.)).children(tiles.into_iter().enumerate().map(|(i, (pick, label, ic, on))| {
            div()
                .id(("vm-only", i))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .w(px(tile_w))
                .pt(px(10.))
                .pb(px(8.))
                .rounded(px(12.))
                .border_1()
                .border_color(if on { theme::accent_tint_border() } else { theme::white_a(0.) })
                .bg(if on { theme::accent_tint() } else { theme::white_a(0.05) })
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.09)))
                .text_size(px(11.5))
                .text_color(if on { white().into() } else { theme::text_muted() })
                .child(icon(ic, px(19.), if on { theme::accent() } else { theme::text_primary() }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.apply_pick(&pick, cx)))
        })))
    }

    fn vm_browse(&self, cx: &mut Context<Self>) -> Div {
        let kinds = Kind::ALL.into_iter().filter(|k| *k != Kind::Only && (*k != Kind::Category || !self.categories.is_empty()));
        section(tr!("Browse")).child(div().flex().flex_wrap().gap(px(6.)).children(kinds.enumerate().map(|(i, kind)| {
            div()
                .id(("vm-browse", i))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(30.))
                .px(px(11.))
                .rounded_full()
                .bg(theme::white_a(0.05))
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.1)))
                .text_size(px(12.5))
                .text_color(theme::text_primary())
                .child(icon(kind.icon(), px(14.), theme::text_muted()))
                .child(kind.label())
                .on_click(cx.listener(move |this, _, window, cx| {
                    let text = choices::scoped_query(kind, "");
                    this.set_view_query(&text, window, cx);
                }))
        })))
    }

    fn vm_footer(&self, cx: &mut Context<Self>) -> Div {
        let shown: usize = self.groups.iter().map(|g| g.cards.len()).sum();
        let total = self.lib.as_deref().map_or(0, |l| l.games.len());
        let filtered = self.ui.active_filters() > 0 || self.ui.show_hidden;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme::white_a(0.08))
            .text_size(px(12.5))
            .text_color(theme::text_muted())
            .child(div().flex_1().child(tr!("{shown} of {total} games", shown, total).to_string()))
            .when(filtered, |d| {
                d.child(
                    div()
                        .id("vm-reset")
                        .cursor_pointer()
                        .text_color(theme::accent())
                        .hover(|s| s.text_color(white()))
                        .child(tr!("Reset all"))
                        .on_click(cx.listener(|this, _, _, cx| this.reset_view_filters(cx))),
                )
            })
            .child(crate::ui::controls::button_sm("vm-statuses", crate::ui::controls::ButtonKind::Glass, tr!("Statuses…")).on_click(
                cx.listener(|this, _, window, cx| {
                    this.open_menu = None;
                    this.open_manage_statuses(window, cx);
                }),
            ))
    }
}

/// Section of the panel: small uppercase label above its content.
fn section(label: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().px(px(4.)).text_size(px(11.5)).text_color(theme::text_dim()).child(label.to_uppercase()))
}

/// Results row (checkbox, label, count); `lit`: picked by Enter.
fn plain_row(id: impl Into<ElementId>, lit: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.))
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .rounded(px(9.))
        .text_size(px(13.5))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .when(lit, |d| d.bg(theme::white_a(0.07)))
        .hover(|s| s.bg(theme::menu_item_hover()))
}

/// Square checkbox of a results row (the row takes the click).
fn check(on: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(17.))
        .rounded(px(5.))
        .border_1()
        .when(on, |d| d.bg(theme::accent()).border_color(theme::accent()).child(icon(Icon::Check, px(13.), theme::on_accent())))
        .when(!on, |d| d.border_color(theme::white_a(0.28)))
}

fn count_label(n: usize) -> Div {
    div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(n.to_string())
}

fn dot(color: Rgba) -> Div {
    div().flex_none().size(px(8.)).rounded_full().bg(color)
}

/// Segmented control: equal parts on a faint pill.
fn seg_track(id: &'static str) -> Stateful<Div> {
    div().id(id).flex().items_center().gap(px(2.)).p(px(3.)).rounded_full().bg(theme::seg_track())
}

fn seg_item(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .py(px(6.))
        .px(px(8.))
        .rounded_full()
        .text_size(px(13.))
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(if on { white() } else { theme::seg_text().into() })
        .when(on, |d| d.bg(theme::seg_active()))
        .when(!on, |d| d.hover(|s| s.text_color(white())))
}
