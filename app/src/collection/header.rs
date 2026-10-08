//! `header.collection__header`: 42px, three columns (left tools, centred
//! search + filters, right tools) and its hover dropdowns.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::{collect_facet_options, FACET_KINDS};
use heronite_data::{FacetKind, InstallFilter, SortMode};

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{dropdown, menu_item, menu_panel, menu_separator, Align};
use crate::ui::{icon, tool_button};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    Install,
    Facets,
    Sort,
    Status,
}

const CLOSE_DELAY: Duration = Duration::from_millis(180);

fn install_icon(f: InstallFilter) -> Icon {
    match f {
        InstallFilter::All => Icon::FaBorderAll,
        InstallFilter::Installed => Icon::FaHardDrive,
        InstallFilter::Uninstalled => Icon::FaHardDriveRegular,
    }
}

fn facet_label(k: FacetKind) -> &'static str {
    match k {
        FacetKind::Series => "Series",
        FacetKind::Developer => "Developer",
        FacetKind::Publisher => "Publisher",
        FacetKind::Genre => "Genre",
        FacetKind::Platform => "Platform",
    }
}

impl CollectionView {
    pub(super) fn new_search_input(window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let state = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_placeholder("Search for Games", window, cx);
            s
        });
        cx.subscribe_in(&state, window, |this, state, _ev: &gpui_kit::component::input::InputEvent, _window, cx| {
            let value = state.read(cx).value().to_string();
            if value != this.search {
                this.search = value;
                this.refresh_groups();
                cx.notify();
            }
        })
        .detach();
        state
    }

    fn menu_hover(&mut self, menu: Menu, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.menu_close = None;
            if self.open_menu != Some(menu) {
                self.open_menu = Some(menu);
                self.facet_flyout = None;
                cx.notify();
            }
        } else if self.open_menu == Some(menu) {
            self.menu_close = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_DELAY).await;
                this.update(cx, |this, cx| {
                    if this.open_menu == Some(menu) {
                        this.open_menu = None;
                        this.facet_flyout = None;
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
    }

    fn hover_dropdown(
        &self,
        menu: Menu,
        ic: Icon,
        align: Align,
        body: impl FnOnce(&Self, &mut Context<Self>) -> AnyElement,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.open_menu == Some(menu);
        let content = open.then(|| body(self, cx));
        dropdown(
            SharedString::from(format!("dd-{menu:?}")),
            tool_button(SharedString::from(format!("btn-{menu:?}")), ic, open).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.open_menu = Some(menu);
                    cx.notify();
                },
            )),
            content,
            align,
            cx.listener(move |this, hovered: &bool, _, cx| this.menu_hover(menu, *hovered, cx)),
        )
    }

    pub(super) fn render_header(&mut self, cx: &mut Context<Self>) -> Div {
        let install = self.hover_dropdown(
            Menu::Install,
            install_icon(self.ui.install_filter),
            Align::Left,
            |this, cx| {
                let mut p = menu_panel(220.);
                for (i, (f, label)) in [
                    (InstallFilter::All, "All games"),
                    (InstallFilter::Installed, "Installed"),
                    (InstallFilter::Uninstalled, "Not installed / Uninstalled"),
                ]
                .into_iter()
                .enumerate()
                {
                    let active = this.ui.install_filter == f;
                    p = p.child(
                        menu_item(SharedString::from(format!("install-{i}")), active)
                            .child(icon(install_icon(f), px(14.4), if active { theme::accent() } else { rgb(0xcaf3fd) }))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ui.install_filter = f;
                                this.save_ui();
                                this.refresh_groups();
                                cx.notify();
                            })),
                    );
                }
                p.into_any_element()
            },
            cx,
        );

        let sort = self.hover_dropdown(
            Menu::Sort,
            Icon::SortByAlpha,
            Align::Right,
            |this, cx| {
                let mut p = menu_panel(200.);
                for (i, (m, label)) in
                    [(SortMode::Title, "Title"), (SortMode::LastPlayed, "Last played"), (SortMode::Playtime, "Playtime")]
                        .into_iter()
                        .enumerate()
                {
                    p = p.child(menu_item(SharedString::from(format!("sort-{i}")), this.ui.sort == m).child(label).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.ui.sort = m;
                            this.save_ui();
                            this.refresh_groups();
                            cx.notify();
                        }),
                    ));
                }
                p.into_any_element()
            },
            cx,
        );

        let status = self.hover_dropdown(
            Menu::Status,
            Icon::Flag,
            Align::Right,
            |this, cx| {
                let on = this.ui.group_by_status;
                menu_panel(240.)
                    .child(
                        menu_item("status-group", false)
                            .justify_between()
                            .child("Group by status")
                            .child(toggle(on))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.group_by_status = !this.ui.group_by_status;
                                this.save_ui();
                                this.refresh_groups();
                                cx.notify();
                            })),
                    )
                    .child(menu_separator())
                    .child(menu_item("status-manage", false).child("Manage statuses…"))
                    .into_any_element()
            },
            cx,
        );

        let facets_active = self.ui.facet_filters.is_active();
        let facets = self.hover_dropdown(
            Menu::Facets,
            Icon::FilterList,
            Align::Right,
            |this, cx| this.render_facet_menu(cx).into_any_element(),
            cx,
        );
        let facets = if facets_active {
            facets.child(
                div().absolute().top(px(5.)).right(px(5.)).size(px(6.)).rounded_full().bg(theme::accent()),
            )
        } else {
            facets
        };

        let refreshing = self.refreshing;
        let refresh_icon = icon(Icon::Refresh, theme::TOOL_ICON, theme::white_a(0.58));
        let refresh = div()
            .id("hdr-refresh")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(theme::TOOL_BTN)
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.08)))
            .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
            .child(if refreshing {
                refresh_icon
                    .with_animation(
                        "refresh-spin",
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |svg, t| svg.with_transformation(Transformation::rotate(percentage(t))),
                    )
                    .into_any_element()
            } else {
                refresh_icon.into_any_element()
            });

        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(theme::HEADER_H)
            .px(px(12.))
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .min_w(px(32.))
                    .child(tool_button("hdr-sidebar", Icon::Menu, false))
                    .child(install),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .child(self.search_box())
                    .child(facets)
                    .child(sort)
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_end()
                    .min_w(px(32.))
                    .child(refresh)
                    .child(tool_button("hdr-playnite", Icon::SyncAlt, false))
                    .child(tool_button("hdr-emulated", Icon::SportsEsports, false))
                    .child(tool_button("hdr-settings", Icon::Settings, false)),
            )
    }

    /// 20rem × 32px search with an underline only, centred text.
    fn search_box(&self) -> Div {
        let focused = !self.search.is_empty();
        div()
            .relative()
            .flex()
            .items_center()
            .w(px(320.))
            .h(px(32.))
            .border_b_1()
            .border_color(theme::white_a(if focused { 0.48 } else { 0.18 }))
            .child(
                div()
                    .absolute()
                    .left(px(4.))
                    .child(icon(Icon::FaSearch, px(12.8), theme::white_a(if focused { 0.72 } else { 0.42 }))),
            )
            .child(
                div()
                    .flex_1()
                    .pl(px(22.))
                    .text_size(px(16.))
                    .text_color(theme::white_a(0.88))
                    .child(Input::new(&self.search_input).appearance(false).text_center()),
            )
    }

    fn render_facet_menu(&self, cx: &mut Context<Self>) -> Div {
        let Some(lib) = self.lib.clone() else { return menu_panel(200.) };
        let options = collect_facet_options(&lib.games);
        let mut p = menu_panel(200.).max_w(px(240.));
        for kind in FACET_KINDS {
            let selected = self.ui.facet_filters.get(kind).map(str::to_owned);
            let open = self.facet_flyout == Some(kind);
            let mut label = div().flex().flex_col().min_w_0().gap(px(1.6)).child(facet_label(kind));
            if let Some(sel) = &selected {
                label = label.child(div().max_w(px(160.)).truncate().text_size(px(11.5)).opacity(0.7).child(sel.clone()));
            }
            let row = menu_item(SharedString::from(format!("facet-{}", kind.as_str())), selected.is_some() || open)
                .justify_between()
                .child(label)
                .child(icon(Icon::ChevronRight, px(16.), theme::white_a(0.7)))
                .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                    if *h && this.facet_flyout != Some(kind) {
                        this.facet_flyout = Some(kind);
                        cx.notify();
                    }
                }));
            let mut wrap = div().relative().child(row);
            if open {
                let mut fly = menu_panel(240.).max_w(px(320.)).max_h(px(420.)).gap(px(1.9));
                fly = fly.child(
                    menu_item(SharedString::from(format!("facet-all-{}", kind.as_str())), selected.is_none())
                        .child("All")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(sel) = this.ui.facet_filters.get(kind).map(str::to_owned) {
                                this.toggle_facet(kind.as_str(), &sel);
                                cx.notify();
                            }
                        })),
                );
                let opts = options.get(kind);
                if opts.is_empty() {
                    fly = fly.child(
                        div().mx(px(5.6)).text_size(px(13.33)).text_color(theme::white_a(0.45)).child("None in this library"),
                    );
                }
                let mut list = div().id(SharedString::from(format!("facet-list-{}", kind.as_str()))).flex().flex_col().gap(px(1.9)).overflow_y_scroll().max_h(px(380.));
                for (i, o) in opts.iter().enumerate() {
                    let is_sel = selected.as_deref().is_some_and(|s| heronite_data::text::facet_key(s) == heronite_data::text::facet_key(&o.value));
                    let value = o.value.clone();
                    list = list.child(
                        menu_item(SharedString::from(format!("facet-{}-{i}", kind.as_str())), is_sel)
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.6))
                                    .min_w_0()
                                    .when(is_sel, |d| d.child(icon(Icon::Check, px(14.4), theme::accent())))
                                    .child(o.value.clone()),
                            )
                            .child(div().text_size(px(12.)).opacity(0.55).child(o.count.to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_facet(kind.as_str(), &value);
                                cx.notify();
                            })),
                    );
                }
                fly = fly.child(list);
                wrap = wrap.child(div().absolute().top_0().right(relative(1.)).mr(px(-2.)).occlude().child(fly));
            }
            p = p.child(wrap);
        }
        if self.ui.facet_filters.is_active() {
            p = p.child(menu_separator()).child(
                menu_item("facet-clear", false)
                    .justify_center()
                    .text_color(theme::accent())
                    .child("Clear filters")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.clear_facets();
                        this.save_ui();
                        this.refresh_groups();
                        cx.notify();
                    })),
            );
        }
        p
    }
}

/// Heroic's ToggleSwitch, compact.
fn toggle(on: bool) -> Div {
    div()
        .w(px(30.))
        .h(px(16.))
        .rounded_full()
        .p(px(2.))
        .bg(if on { theme::accent().into() } else { theme::white_a(0.2) })
        .flex()
        .when(on, |d| d.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(rgb(0x191c22)))
}
