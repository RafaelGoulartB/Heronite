//! Optional Collection shortcuts. The zero-basis header lane is measured
//! after layout; text is shaped in the same font as the controls. Resizing
//! never writes settings, and a vanished trigger cannot keep its menu open.

mod layout;

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{GroupBy, InstallFilter, SortMode};
use heronite_i18n::tr;

use super::header::{install_label, Menu};
use super::view_filters::Only;
use super::view_menu::organize::{group_label, sort_direction, sort_direction_icon, sort_icon, sort_label};
use super::view_menu::Pick;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel, Align};
use crate::ui::{icon, tooltip};

pub(super) use layout::GAP;

const HEIGHT: f32 = 32.;
const LABEL: f32 = 12.5;
const ICON: f32 = 14.;
const PAD: f32 = 10.;
const PAIR_GAP: f32 = 2.;
const PRESET_LABEL_MAX: f32 = 160.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::collection) enum QuickMenu {
    Sort,
    Install,
    Group,
    Presets,
}

impl QuickMenu {
    fn index(self) -> usize {
        match self {
            Self::Sort => 0,
            Self::Install => 1,
            Self::Group => 3,
            Self::Presets => 4,
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub width: Rc<Cell<Option<f32>>>,
    visible: usize,
}

impl State {
    pub fn shows(&self, menu: QuickMenu) -> bool {
        self.visible > menu.index()
    }
}

pub(super) fn text_width(text: &str, window: &Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: font(theme::font_body()),
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(text.to_owned().into(), px(LABEL), &[run], None);
    f32::from(line.width).ceil() + 1.
}

fn select_width<'a>(labels: impl Iterator<Item = &'a str>, window: &Window) -> f32 {
    labels.map(|s| text_width(s, window)).fold(0., f32::max) + 2. * PAD + 2. * ICON + 2. * GAP + 2.
}

fn button(id: impl Into<ElementId>, width: f32, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(GAP))
        .h(px(HEIGHT))
        .w(px(width))
        .px(px(PAD))
        .rounded_full()
        .border_1()
        .border_color(if active { theme::accent_alpha(0.3).into() } else { theme::ink_a(0.08) })
        .bg(if active { theme::accent_tint() } else { theme::search_fill() })
        .text_size(px(LABEL))
        .text_color(if active { theme::text_bright() } else { theme::text_primary().into() })
        .cursor_pointer()
        .hover(|s| s.bg(theme::search_fill_hover()).text_color(theme::text_bright()))
}

impl CollectionView {
    pub(super) fn quick_toolbar(&mut self, chips: f32, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let enabled = theme::appearance().quick_toolbar_controls;
        let sort_w = select_width(SortMode::ALL.into_iter().map(sort_label), window);
        let install_w =
            select_width([InstallFilter::All, InstallFilter::Installed, InstallFilter::Uninstalled].into_iter().map(install_label), window);
        let group_w = select_width(GroupBy::ALL.into_iter().map(group_label), window);
        let current = self.current_view();
        let preset = current.map_or(tr!("Custom"), |i| self.ui.saved_views[i].name.as_str());
        let preset_w = text_width(preset, window).min(PRESET_LABEL_MAX) + 2. * PAD + 2. * ICON + 2. * GAP + 2.;
        let widths = [sort_w + PAIR_GAP + HEIGHT, install_w, HEIGHT, group_w, preset_w];
        let offered = if self.ui.saved_views.is_empty() { &widths[..4] } else { &widths[..] };
        let visible = layout::visible_count(enabled, self.toolbar.width.get().unwrap_or(0.), chips, offered);
        if visible < self.toolbar.visible {
            self.menu_open = None;
        }
        self.toolbar.visible = visible;
        if let Some(Menu::Quick(which)) = self.open_menu {
            if (!enabled || self.toolbar.width.get().is_some()) && !self.toolbar.shows(which) {
                self.close_quick_menu(cx);
            }
        }
        if visible == 0 {
            return None;
        }
        let max_h = (f32::from(window.viewport_size().height) - 58.).max(80.);
        let sort = self.quick_select(
            QuickMenu::Sort,
            "quick-sort",
            sort_label(self.ui.sort),
            sort_icon(self.ui.sort),
            sort_w,
            tr!("Sort"),
            max_h,
            cx,
        );
        let direction = button("quick-sort-direction", HEIGHT, false)
            .px(px(0.))
            .tooltip(tooltip(sort_direction(self.ui.sort, self.ui.sort_descending)))
            .child(icon(sort_direction_icon(self.ui.sort, self.ui.sort_descending), px(16.), theme::text_primary()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sort_direction(cx)));
        let mut row = div()
            .id("collection-quick-toolbar")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(GAP))
            .child(div().flex().flex_none().gap(px(PAIR_GAP)).child(sort).child(direction));
        if visible >= 2 {
            row = row.child(self.quick_select(
                QuickMenu::Install,
                "quick-install",
                install_label(self.ui.install_filter),
                Icon::Download,
                install_w,
                tr!("Show"),
                max_h,
                cx,
            ));
        }
        if visible >= 3 {
            row = row.child(
                button("quick-favourites", HEIGHT, self.ui.only.favourites)
                    .px(px(0.))
                    .tooltip(tooltip(Only::Favourites.label()))
                    .child(icon(
                        if self.ui.only.favourites { Icon::Star } else { Icon::StarBorder },
                        px(16.),
                        if self.ui.only.favourites { theme::accent() } else { theme::text_primary() },
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_pick(&Pick::Only(Only::Favourites), cx))),
            );
        }
        if visible >= 4 {
            row = row.child(self.quick_select(
                QuickMenu::Group,
                "quick-group",
                group_label(self.ui.group_by),
                Icon::Folder,
                group_w,
                tr!("Group by"),
                max_h,
                cx,
            ));
        }
        if visible >= 5 {
            let preset = self.current_view().map_or(tr!("Custom"), |i| self.ui.saved_views[i].name.as_str());
            row = row.child(self.quick_select(
                QuickMenu::Presets,
                "quick-presets",
                preset,
                Icon::Bookmark,
                preset_w,
                tr!("Presets"),
                max_h,
                cx,
            ));
        }
        Some(row)
    }

    fn quick_select(
        &self,
        menu: QuickMenu,
        id: &'static str,
        label: &str,
        glyph: Icon,
        width: f32,
        hint: &'static str,
        max_h: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let active =
            self.open_menu == Some(Menu::Quick(menu)) || matches!(menu, QuickMenu::Install) && self.ui.install_filter != InstallFilter::All;
        let trigger = button(id, width, active)
            .tooltip(tooltip(format!("{hint}: {label}")))
            .child(icon(glyph, px(ICON), theme::text_muted()))
            .child(div().flex_1().min_w_0().truncate().child(label.to_owned()))
            .child(icon(Icon::ExpandMore, px(ICON), theme::text_dim()));
        self.hover_dropdown(Menu::Quick(menu), trigger, Align::Left, |this, cx| this.quick_menu(menu, max_h, cx), cx)
    }

    fn close_quick_menu(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.menu_open = None;
        self.menu_close = None;
        self.menu_parts = 0;
        cx.notify();
    }

    fn quick_menu(&self, which: QuickMenu, max_h: f32, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().id("quick-menu-scroll").min_h_0().overflow_y_scroll();
        match which {
            QuickMenu::Sort => {
                for mode in SortMode::ALL {
                    rows = rows.child(option(("quick-sort-opt", mode as usize), sort_label(mode), self.ui.sort == mode).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.set_sort(mode, cx);
                            this.close_quick_menu(cx);
                        }),
                    ));
                }
            }
            QuickMenu::Install => {
                for filter in [InstallFilter::All, InstallFilter::Installed, InstallFilter::Uninstalled] {
                    rows = rows.child(
                        option(("quick-install-opt", filter as usize), install_label(filter), self.ui.install_filter == filter).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.set_install_filter(filter, cx);
                                this.close_quick_menu(cx);
                            }),
                        ),
                    );
                }
            }
            QuickMenu::Group => {
                for by in GroupBy::ALL {
                    rows = rows.child(option(("quick-group-opt", by as usize), group_label(by), self.ui.group_by == by).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.set_group_by(by, cx);
                            this.close_quick_menu(cx);
                        }),
                    ));
                }
            }
            QuickMenu::Presets => {
                let current = self.current_view();
                for (i, view) in self.ui.saved_views.iter().enumerate() {
                    let filters = view.filters.clone();
                    rows = rows.child(
                        option(("quick-preset-opt", i), &view.name, current == Some(i)).tooltip(tooltip(view.name.clone())).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.apply_view_filters(&filters, cx);
                                this.close_quick_menu(cx);
                            }),
                        ),
                    );
                }
            }
        }
        let title = match which {
            QuickMenu::Sort => tr!("Sort"),
            QuickMenu::Install => tr!("Show"),
            QuickMenu::Group => tr!("Group by"),
            QuickMenu::Presets => tr!("Presets"),
        };
        menu_panel(240.)
            .w(px(260.))
            .max_h(px(max_h))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_quick_menu(cx)))
            .child(div().px(px(10.)).py(px(6.)).text_size(px(12.)).text_color(theme::text_muted()).child(title))
            .child(rows)
            .into_any_element()
    }
}

fn option(id: impl Into<ElementId>, label: &str, active: bool) -> Stateful<Div> {
    menu_item(id, active)
        .child(div().flex_1().min_w_0().truncate().child(label.to_owned()))
        .children(active.then(|| icon(Icon::Check, px(14.), theme::accent())))
}
