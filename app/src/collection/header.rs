//! `header.collection__header`: 42px. Left, over the list column: ☰, the
//! search pill and the View button (filters, sort, grouping in one menu);
//! then one chip per active filter; right: bulk metadata, refresh and the
//! focus panel's close.

use std::time::Duration;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::{collect_facet_options, FACET_KINDS};
use heronite_data::{FacetKind, InstallFilter, SortMode};

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{dropdown, menu_item, menu_panel, menu_separator, Align, PART_BUTTON};
use crate::ui::{icon, tool_button, tool_button_base};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    /// Show / sort / group / filter.
    View,
    Nav,
}

const CLOSE_DELAY: Duration = Duration::from_millis(180);
/// Hover intent: passing over a button on the way elsewhere opens nothing.
const OPEN_DELAY: Duration = Duration::from_millis(140);

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
            s.set_placeholder("Search games", window, cx);
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

    fn open_header_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        if self.open_menu != Some(menu) {
            self.open_menu = Some(menu);
            self.menu_parts = PART_BUTTON;
            self.facet_flyout = None;
        }
        self.menu_open = None;
        self.menu_close = None;
        cx.notify();
    }

    /// `part`: which piece of the dropdown changed hover (see
    /// `ui::dropdown::PART_*`). Opens after [`OPEN_DELAY`] on the button and
    /// closes 180ms after the last part is left.
    pub(super) fn menu_hover(&mut self, menu: Menu, part: u8, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            if self.open_menu != Some(menu) {
                self.menu_open = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(OPEN_DELAY).await;
                    this.update(cx, |this, cx| this.open_header_menu(menu, cx)).ok();
                }));
                return;
            }
            self.menu_parts |= part;
            self.menu_close = None;
            cx.notify();
        } else if self.open_menu == Some(menu) {
            self.menu_parts &= !part;
            if self.menu_parts != 0 {
                return;
            }
            self.menu_close = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_DELAY).await;
                this.update(cx, |this, cx| {
                    if this.open_menu == Some(menu) && this.menu_parts == 0 {
                        this.open_menu = None;
                        this.facet_flyout = None;
                        cx.notify();
                    }
                })
                .ok();
            }));
        } else if part == PART_BUTTON {
            self.menu_open = None;
        }
    }

    pub(super) fn hover_dropdown(
        &self,
        menu: Menu,
        button: Stateful<Div>,
        align: Align,
        body: impl FnOnce(&Self, &mut Context<Self>) -> AnyElement,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.open_menu == Some(menu);
        let content = open.then(|| body(self, cx));
        dropdown(
            SharedString::from(format!("dd-{menu:?}")),
            button.on_click(cx.listener(move |this, _, _, cx| this.open_header_menu(menu, cx))),
            content,
            align,
            {
                let weak = cx.entity().downgrade();
                std::rc::Rc::new(move |part, hovered, _: &mut Window, cx: &mut App| {
                    weak.update(cx, |this, cx| this.menu_hover(menu, part, hovered, cx)).ok();
                })
            },
        )
    }

    /// "Show hidden games": persists the flag and reloads the library with or
    /// without the games hidden in Heroic.
    pub(super) fn set_show_hidden(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.ui.show_hidden == on {
            return;
        }
        self.ui.show_hidden = on;
        self.save_ui();
        self.refreshing = true;
        self.spawn_load(cx);
        cx.notify();
    }

    fn set_install_filter(&mut self, f: InstallFilter, cx: &mut Context<Self>) {
        self.ui.install_filter = f;
        self.save_ui();
        self.refresh_groups();
        cx.notify();
    }

    /// Whether the View menu hides any game (its button turns accent).
    fn view_filtered(&self) -> bool {
        self.ui.install_filter != InstallFilter::All || self.ui.show_hidden || self.ui.facet_filters.is_active()
    }

    pub(super) fn render_header(&mut self, list_w: f32, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let open = self.open_menu == Some(Menu::View);
        let view_btn = tool_button_base("btn-View", open).child(icon(
            Icon::Tune,
            theme::TOOL_ICON,
            if self.view_filtered() {
                theme::accent().into()
            } else if open {
                theme::white_a(1.)
            } else {
                theme::tool_icon()
            },
        ));
        let view = self.hover_dropdown(Menu::View, view_btn, Align::Left, |this, cx| this.view_menu(cx), cx);
        let refresh = self.refresh_button(cx);
        let close = self.focus_close_button(cx);

        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(theme::HEADER_H)
            .px(px(12.))
            .gap(px(10.))
            .child(
                // Spans the list column (up to its border) so the search
                // reads as the list's own.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .w(px((list_w - 24.).max(300.)))
                    .child(self.nav_dropdown(cx))
                    .child(self.search_box(window, cx))
                    .child(view),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .children(self.filter_chips(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .children(self.bulk_chip(cx))
                    .child(refresh)
                    .children(close),
            )
    }

    /// One removable chip per filter the View menu has set.
    fn filter_chips(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let mut chips = Vec::new();
        match self.ui.install_filter {
            InstallFilter::All => {}
            f => chips.push(
                filter_chip("chip-install", None, if f == InstallFilter::Installed { "Installed" } else { "Not installed" })
                    .on_click(cx.listener(|this, _, _, cx| this.set_install_filter(InstallFilter::All, cx))),
            ),
        }
        if self.ui.show_hidden {
            chips.push(
                filter_chip("chip-hidden", None, "Hidden shown")
                    .on_click(cx.listener(|this, _, _, cx| this.set_show_hidden(false, cx))),
            );
        }
        for kind in FACET_KINDS {
            let Some(value) = self.ui.facet_filters.get(kind).filter(|v| !v.is_empty()) else { continue };
            let value = value.to_owned();
            chips.push(
                filter_chip(SharedString::from(format!("chip-{}", kind.as_str())), Some(facet_label(kind)), &value)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_facet(kind.as_str(), &value);
                        cx.notify();
                    })),
            );
        }
        chips
    }

    /// Show (install filter), sort, the two switches, facet filters and
    /// "Manage statuses…" in one panel.
    fn view_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let install = crate::ui::controls::tabs(
            "view-install",
            &[(InstallFilter::All, "All"), (InstallFilter::Installed, "Installed"), (InstallFilter::Uninstalled, "Not installed")],
            self.ui.install_filter,
            {
                let weak = weak.clone();
                move |f, _, cx| {
                    weak.update(cx, |this, cx| this.set_install_filter(f, cx)).ok();
                }
            },
        );
        let sort = crate::ui::controls::tabs(
            "view-sort",
            &[(SortMode::Title, "Title"), (SortMode::LastPlayed, "Last played"), (SortMode::Playtime, "Playtime")],
            self.ui.sort,
            move |m, _, cx| {
                weak.update(cx, |this, cx| {
                    this.ui.sort = m;
                    this.save_ui();
                    this.refresh_groups();
                    cx.notify();
                })
                .ok();
            },
        );
        let facets_active = self.ui.facet_filters.is_active();
        let mut filter_title = section_label("Filter by");
        if facets_active {
            filter_title = filter_title.child(
                div()
                    .id("facet-clear")
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|s| s.text_color(white()))
                    .child("Clear")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.clear_facets();
                        this.save_ui();
                        this.refresh_groups();
                        cx.notify();
                    })),
            );
        }
        menu_panel(300.)
            .w(px(300.))
            .child(section_label("Show").mt(px(2.)))
            .child(div().px(px(4.)).child(install))
            .child(section_label("Sort by"))
            .child(div().px(px(4.)).child(sort))
            .child(menu_separator().mt(px(8.)))
            .child(
                menu_item("status-group", false)
                    .justify_between()
                    .child("Group by status")
                    .child(toggle("status-group-sw", self.ui.group_by_status))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.group_by_status = !this.ui.group_by_status;
                        this.save_ui();
                        this.refresh_groups();
                        cx.notify();
                    })),
            )
            .child(
                menu_item("install-show-hidden", false)
                    .justify_between()
                    .child("Show hidden games")
                    .child(toggle("install-show-hidden-sw", self.ui.show_hidden))
                    .on_click(cx.listener(|this, _, _, cx| this.set_show_hidden(!this.ui.show_hidden, cx))),
            )
            .child(menu_separator())
            .child(filter_title.mt(px(2.)))
            .children(self.facet_rows(cx))
            .child(menu_separator())
            .child(
                menu_item("status-manage", false)
                    .child(icon(Icon::Flag, px(15.), theme::text_muted()))
                    .child("Manage statuses…")
                    .on_click(cx.listener(|this, _, window, cx| this.open_manage_statuses(window, cx))),
            )
            .into_any_element()
    }

    fn refresh_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let refreshing = self.refreshing;
        let refresh_icon = icon(Icon::Refresh, theme::TOOL_ICON, theme::tool_icon());
        tool_button_base("hdr-refresh", false)
            .on_click(cx.listener(|this, _, _, cx| this.refresh_all(cx)))
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
            })
    }

    /// Closes the focus panel (deselects the game); only shown while it is open.
    fn focus_close_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        self.focus.is_some().then(|| {
            tool_button("hdr-focus-close", Icon::Close, false).on_click(cx.listener(|this, _, _, cx| {
                this.focus = None;
                this.selected = None;
                cx.notify();
            }))
        })
    }

    /// 32px pill: faint fill, brighter with an accent ring while focused;
    /// a clear button once something is typed.
    fn search_box(&self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let focused = self.search_input.read(cx).focus_handle(cx).is_focused(window);
        let has_text = !self.search.is_empty();
        div()
            .id("hdr-search")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(8.))
            .h(theme::TOOL_BTN)
            .pl(px(12.))
            .pr(px(if has_text { 4. } else { 12. }))
            .rounded_full()
            .border_1()
            .border_color(if focused { theme::input_border_focus() } else { theme::white_a(0.) })
            .bg(if focused || has_text { theme::search_fill_hover() } else { theme::search_fill() })
            .hover(|s| s.bg(theme::search_fill_hover()))
            .child(icon(Icon::FaSearch, px(12.), theme::white_a(if focused || has_text { 0.75 } else { 0.42 })))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.))
                    .text_color(theme::white_a(0.92))
                    .child(Input::new(&self.search_input).appearance(false)),
            )
            .when(has_text, |d| {
                d.child(
                    div()
                        .id("hdr-search-clear")
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded_full()
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::white_a(0.1)))
                        .child(icon(Icon::Close, px(14.), theme::white_a(0.7)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search_input.update(cx, |s, cx| s.set_value("", window, cx));
                            this.search.clear();
                            this.refresh_groups();
                            cx.notify();
                        })),
                )
            })
    }

    /// The five facet rows (current value on the right) and, for the
    /// hovered one, its searchable flyout opening to the right.
    fn facet_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let Some(lib) = self.lib.clone() else { return Vec::new() };
        let options = collect_facet_options(&lib.games);
        let mut rows = Vec::new();
        for kind in FACET_KINDS {
            let selected = self.ui.facet_filters.get(kind).map(str::to_owned);
            let open = self.facet_flyout == Some(kind);
            let row = menu_item(SharedString::from(format!("facet-{}", kind.as_str())), open)
                .justify_between()
                .child(div().flex_none().child(facet_label(kind)))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .justify_end()
                        .items_center()
                        .gap(px(4.))
                        .children(selected.clone().map(|sel| {
                            div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::accent()).child(sel)
                        }))
                        .child(icon(Icon::ChevronRight, px(16.), theme::white_a(0.5))),
                )
                .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                    if *h && this.facet_flyout != Some(kind) {
                        this.facet_flyout = Some(kind);
                        cx.notify();
                    }
                }));
            let mut wrap = div().relative().child(row);
            if open {
                let mut fly = menu_panel(240.).w(px(280.)).max_h(px(420.)).gap(px(1.9));
                let query = self.facet_query(kind, cx);
                fly = fly.children(self.facet_search_box(kind));
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
                let opts: Vec<_> = options
                    .get(kind)
                    .iter()
                    .filter(|o| super::extras::facet_option_matches(&o.value, &query))
                    .collect();
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
                                    .flex_1()
                                    .items_center()
                                    .gap(px(5.6))
                                    .min_w_0()
                                    .when(is_sel, |d| d.child(icon(Icon::Check, px(14.4), theme::accent())))
                                    .child(div().min_w_0().truncate().child(o.value.clone())),
                            )
                            .child(div().flex_none().text_size(px(12.)).opacity(0.55).child(o.count.to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_facet(kind.as_str(), &value);
                                cx.notify();
                            })),
                    );
                }
                fly = fly.child(list);
                wrap = wrap.child(
                    div()
                        .id(SharedString::from(format!("facet-fly-{}", kind.as_str())))
                        .absolute()
                        .top(px(-6.))
                        .left(relative(1.))
                        // Past the menu padding (6) plus a small gap.
                        .ml(px(10.))
                        .occlude()
                        .on_hover(cx.listener(|this, h: &bool, _, cx| {
                            this.menu_hover(Menu::View, crate::ui::dropdown::PART_FLYOUT, *h, cx)
                        }))
                        .child(fly),
                );
            }
            rows.push(wrap);
        }
        rows
    }
}

/// Small muted heading inside the View menu (with room for an action on
/// the right).
fn section_label(text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(10.))
        .pt(px(8.))
        .pb(px(4.))
        .text_size(px(12.))
        .text_color(theme::text_muted())
        .child(text)
}

/// Active filter in the header: faint pill, `kind` muted before the value;
/// clicking it removes the filter.
fn filter_chip(id: impl Into<ElementId>, kind: Option<&'static str>, value: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .pl(px(10.))
        .pr(px(6.))
        .max_w(px(220.))
        .rounded_full()
        .bg(theme::chip_fill())
        .text_size(px(12.5))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .hover(|s| s.bg(theme::white_a(0.12)))
        .children(kind.map(|k| div().flex_none().text_color(theme::text_muted()).child(k)))
        .child(div().min_w_0().truncate().child(value.to_owned()))
        .child(icon(Icon::Close, px(13.), theme::white_a(0.6)))
}

/// Compact glass switch of the menu rows (the row itself handles the click).
fn toggle(id: &'static str, on: bool) -> Stateful<Div> {
    crate::ui::controls::mini_checkbox(id, on, false).mt(px(0.))
}

impl CollectionView {
    /// Opens the Settings page on `section`.
    pub(super) fn open_settings_section(
        &mut self,
        section: crate::pages::settings::Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::pages::settings::request_section(section, cx);
        self.navigate(crate::pages::Page::Settings, window, cx);
    }

    /// Hands the Settings page what it needs from the Collection (the
    /// settings already loaded and the focused game's stage artwork).
    pub(super) fn offer_settings_data(&self, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.as_ref() else { return };
        let chain = self.selected.as_ref().and_then(|k| self.stage_chains.get(k).cloned()).unwrap_or_default();
        crate::pages::settings::offer(lib.settings.clone(), chain, cx);
    }
}
