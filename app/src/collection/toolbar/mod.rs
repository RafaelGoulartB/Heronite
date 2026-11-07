//! Optional Collection shortcuts, off by default: a track like the Focus /
//! Grid switch (`view_mode.rs`) after the View button. A control at its
//! default is only its icon; a value in use adds its label and a chevron
//! (the accent pill when it hides games). While a control is shown, its
//! filter chip is not (`layout::fit`).
//!
//! The zero-basis header lane is measured after layout; text is shaped in the
//! same font as the controls. Resizing never writes settings, and a vanished
//! trigger cannot keep its menu open.

mod layout;

use std::cell::Cell;
use std::f32::consts::PI;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{GroupBy, InstallFilter};
use heronite_i18n::{tr, trc};

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

/// The track and its segments, as in the Focus / Grid switch.
const TRACK_PAD: f32 = 3.;
const SEG_GAP: f32 = 2.;
const SEG_H: f32 = 26.;
const SEG_PAD: f32 = 11.;
const ICON: f32 = 15.;
const ICON_GAP: f32 = 6.;
const LABEL: f32 = 12.5;
/// An icon-only segment.
const ICON_SEG: f32 = 30.;
/// The chevron after a label, 4px before it and 8px after it.
const CARET: f32 = 12.;
const CARET_SLOT: f32 = 4. + CARET + 8.;
/// A preset's colour dot.
const DOT: f32 = 8.;
/// Between a label's dimmed kind ("by") and its value.
const KIND_GAP: f32 = 4.;
const PRESET_LABEL_MAX: f32 = 160.;
/// The View button and the arrow that shows or hides the toolbar.
const VIEW_GROUP: &str = "collection-view-button";

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

/// Favourites' place in the priority order (it has no menu).
const FAVOURITES: usize = 2;

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

/// Whether a toolbar showing `shown` controls already carries this filter,
/// so its chip would repeat it.
pub(super) fn covers(pick: &Pick, shown: usize) -> bool {
    match pick {
        Pick::Show(_) => shown > QuickMenu::Install.index(),
        Pick::Only(Only::Favourites) => shown > FAVOURITES,
        _ => false,
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

/// An icon-only segment, or one with a label of this width and a chevron.
fn segment_width(label: Option<f32>) -> f32 {
    label.map_or(ICON_SEG, |text| SEG_PAD + ICON + ICON_GAP + text + CARET_SLOT)
}

/// This render's segment widths, from what they show now.
pub(super) struct Widths {
    sort: f32,
    install: f32,
    group: f32,
    preset: f32,
    presets: bool,
}

impl Widths {
    /// Lane cost of each control in priority order, so that each plus `GAP`
    /// is what it adds to the lane: the first carries the track's padding
    /// and the direction button, the others their `SEG_GAP`, and the last
    /// `GAP` is the space before the chips.
    fn items(&self) -> Vec<f32> {
        let next = |w: f32| w + SEG_GAP - GAP;
        let mut items = vec![2. * TRACK_PAD + self.sort + SEG_GAP + ICON_SEG, next(self.install), next(ICON_SEG), next(self.group)];
        if self.presets {
            items.push(next(self.preset));
        }
        items
    }
}

/// The dimmed word before the grouping ("by Status").
fn group_kind() -> &'static str {
    trc!("grouped", "by")
}

fn idle() -> Hsla {
    theme::ink_a(0.68)
}

/// A segment of the track. `filter`: the accent pill of a filter in use.
fn segment(id: impl Into<ElementId>, width: f32, filter: bool, open: bool) -> Stateful<Div> {
    let hover = if filter { theme::accent_alpha(0.24).into() } else { theme::ink_a(0.08) };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .w(px(width))
        .h(px(SEG_H))
        .rounded_full()
        .overflow_hidden()
        .cursor_pointer()
        .text_size(px(LABEL))
        .whitespace_nowrap()
        .text_color(if filter { theme::accent().into() } else { idle() })
        .when(filter, |d| d.bg(theme::accent_tint()))
        .when(open && !filter, |d| d.bg(theme::ink_a(0.08)).text_color(theme::text_bright()))
        .hover(move |s| if filter { s.bg(hover) } else { s.bg(hover).text_color(theme::text_bright()) })
}

fn icon_segment(id: impl Into<ElementId>, glyph: Icon, filter: bool, open: bool) -> Stateful<Div> {
    segment(id, ICON_SEG, filter, open).justify_center().child(icon(glyph, px(ICON), if filter { theme::accent().into() } else { idle() }))
}

/// What a select segment shows before its label.
enum Lead {
    Icon(Icon),
    Dot(Rgba),
}

impl CollectionView {
    pub(super) fn quick_widths(&self, window: &Window) -> Widths {
        let label = |text: &str| Some(text_width(text, window));
        let preset = self.current_view().map(|i| text_width(&self.ui.saved_views[i].name, window).min(PRESET_LABEL_MAX) + DOT - ICON);
        Widths {
            sort: segment_width(label(sort_label(self.ui.sort))),
            install: segment_width(
                (self.ui.install_filter != InstallFilter::All).then(|| text_width(install_label(self.ui.install_filter), window)),
            ),
            group: segment_width(
                (self.ui.group_by != GroupBy::None)
                    .then(|| text_width(group_kind(), window) + KIND_GAP + text_width(group_label(self.ui.group_by), window)),
            ),
            preset: segment_width(preset),
            presets: !self.ui.saved_views.is_empty(),
        }
    }

    /// How many controls the toolbar shows, given each filter chip's
    /// `(pick, width)`.
    pub(super) fn quick_fit(&self, widths: &Widths, chips: &[(Pick, f32)]) -> usize {
        let lane = self.toolbar.width.get().unwrap_or(0.);
        let chip_width = |shown: usize| chips.iter().filter(|(pick, _)| !covers(pick, shown)).map(|(_, w)| w + GAP).sum();
        layout::fit(theme::appearance().quick_toolbar_controls, lane, &widths.items(), chip_width)
    }

    /// The View button with, on its right, the arrow that shows or hides the
    /// toolbar; the arrow appears while the pointer is over either.
    pub(super) fn view_button_group(&self, view: Stateful<Div>, cx: &mut Context<Self>) -> Div {
        let on = theme::appearance().quick_toolbar_controls;
        let toggle = div()
            .id("quick-toolbar-toggle")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(18.))
            .h(px(SEG_H))
            .rounded_full()
            .cursor_pointer()
            .opacity(0.)
            .group_hover(VIEW_GROUP, |s| s.opacity(1.))
            .hover(|s| s.bg(theme::ink_a(0.08)))
            .tooltip(tooltip(if on { tr!("Hide quick controls") } else { tr!("Show quick controls") }))
            .child(icon(if on { Icon::ChevronLeft } else { Icon::ChevronRight }, px(16.), theme::tool_icon()))
            .on_click(cx.listener(|this, _, _, cx| this.set_look(|a| a.quick_toolbar_controls = !a.quick_toolbar_controls, cx)));
        div().group(VIEW_GROUP).flex().flex_none().items_center().child(view).child(toggle)
    }

    pub(super) fn quick_toolbar(
        &mut self,
        visible: usize,
        widths: &Widths,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let enabled = theme::appearance().quick_toolbar_controls;
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
        let (mode, descending) = (self.ui.sort, self.ui.sort_descending);
        let mut track = div()
            .id("collection-quick-toolbar")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(SEG_GAP))
            .p(px(TRACK_PAD))
            .rounded_full()
            .bg(theme::ink_a(0.06))
            .child(self.quick_select(
                QuickMenu::Sort,
                "quick-sort",
                Lead::Icon(sort_icon(mode)),
                Some(sort_label(mode)),
                None,
                widths.sort,
                false,
                max_h,
                cx,
            ))
            .child(
                icon_segment("quick-sort-direction", sort_direction_icon(mode, descending), false, false)
                    .tooltip(tooltip(sort_direction(mode, descending)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sort_direction(cx))),
            );
        if visible > QuickMenu::Install.index() {
            let filter = self.ui.install_filter;
            let filtered = filter != InstallFilter::All;
            track = track.child(self.quick_select(
                QuickMenu::Install,
                "quick-install",
                Lead::Icon(Icon::Download),
                filtered.then(|| install_label(filter)),
                None,
                widths.install,
                filtered,
                max_h,
                cx,
            ));
        }
        if visible > FAVOURITES {
            let on = self.ui.only.favourites;
            track = track.child(
                icon_segment("quick-favourites", if on { Icon::Star } else { Icon::StarBorder }, on, false)
                    .tooltip(tooltip(Only::Favourites.label()))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_pick(&Pick::Only(Only::Favourites), cx))),
            );
        }
        if visible > QuickMenu::Group.index() {
            let by = self.ui.group_by;
            track = track.child(self.quick_select(
                QuickMenu::Group,
                "quick-group",
                Lead::Icon(Icon::ViewAgenda),
                (by != GroupBy::None).then(|| group_label(by)),
                Some(group_kind()),
                widths.group,
                false,
                max_h,
                cx,
            ));
        }
        if visible > QuickMenu::Presets.index() {
            let current = self.current_view().map(|i| self.view_cards().swap_remove(i));
            let (lead, label) = match &current {
                Some(card) => (Lead::Dot(card.color), Some(card.name.as_str())),
                None => (Lead::Icon(Icon::BookmarkBorder), None),
            };
            track = track.child(self.quick_select(QuickMenu::Presets, "quick-presets", lead, label, None, widths.preset, false, max_h, cx));
        }
        Some(track)
    }

    /// A segment that opens `menu` on hover, like the header's other menus.
    /// Without a label it is only its icon; `kind` goes dimmed before the
    /// label ("by Status": a grouping, not a filter).
    #[allow(clippy::too_many_arguments)]
    fn quick_select(
        &self,
        menu: QuickMenu,
        id: &'static str,
        lead: Lead,
        label: Option<&str>,
        kind: Option<&'static str>,
        width: f32,
        filter: bool,
        max_h: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.open_menu == Some(Menu::Quick(menu));
        let tint: Hsla = if filter { theme::accent().into() } else { idle() };
        let lead = match lead {
            Lead::Icon(glyph) => icon(glyph, px(ICON), tint).into_any_element(),
            Lead::Dot(color) => div().flex_none().size(px(DOT)).rounded_full().bg(color).into_any_element(),
        };
        let trigger = match label {
            None => segment(id, width, filter, open).justify_center().child(lead),
            Some(label) => segment(id, width, filter, open)
                .pl(px(SEG_PAD))
                .child(lead)
                .children(kind.map(|kind| div().flex_none().ml(px(ICON_GAP)).text_color(theme::ink_a(0.42)).child(kind)))
                .child(div().flex_1().min_w_0().ml(px(if kind.is_some() { KIND_GAP } else { ICON_GAP })).truncate().child(label.to_owned()))
                .child(
                    icon(Icon::ExpandMore, px(CARET), tint)
                        .ml(px(4.))
                        .mr(px(8.))
                        .with_transformation(Transformation::rotate(radians(if open { PI } else { 0. }))),
                ),
        };
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
                for mode in heronite_data::SortMode::ALL {
                    let row = option(
                        ("quick-sort-opt", mode as usize),
                        Some(Lead::Icon(sort_icon(mode))),
                        sort_label(mode),
                        self.ui.sort == mode,
                    );
                    rows = rows.child(row.on_click(cx.listener(move |this, _, _, cx| {
                        this.set_sort(mode, cx);
                        this.close_quick_menu(cx);
                    })));
                }
            }
            QuickMenu::Install => {
                for filter in [InstallFilter::All, InstallFilter::Installed, InstallFilter::Uninstalled] {
                    let row = option(("quick-install-opt", filter as usize), None, install_label(filter), self.ui.install_filter == filter);
                    rows = rows.child(row.on_click(cx.listener(move |this, _, _, cx| {
                        this.set_install_filter(filter, cx);
                        this.close_quick_menu(cx);
                    })));
                }
            }
            QuickMenu::Group => {
                for by in GroupBy::ALL {
                    let row = option(("quick-group-opt", by as usize), None, group_label(by), self.ui.group_by == by);
                    rows = rows.child(row.on_click(cx.listener(move |this, _, _, cx| {
                        this.set_group_by(by, cx);
                        this.close_quick_menu(cx);
                    })));
                }
            }
            QuickMenu::Presets => {
                let current = self.current_view();
                for (i, card) in self.view_cards().into_iter().enumerate() {
                    let filters = card.filters;
                    let row = option(("quick-preset-opt", i), Some(Lead::Dot(card.color)), &card.name, current == Some(i))
                        .tooltip(tooltip(card.name.clone()));
                    rows = rows.child(row.on_click(cx.listener(move |this, _, _, cx| {
                        this.apply_view_filters(&filters, cx);
                        this.close_quick_menu(cx);
                    })));
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

fn option(id: impl Into<ElementId>, lead: Option<Lead>, label: &str, active: bool) -> Stateful<Div> {
    let lead = lead.map(|lead| match lead {
        Lead::Icon(glyph) => icon(glyph, px(15.), if active { theme::accent() } else { theme::text_muted() }).into_any_element(),
        Lead::Dot(color) => div().flex_none().size(px(DOT)).mx(px(3.5)).rounded_full().bg(color).into_any_element(),
    });
    menu_item(id, active)
        .children(lead)
        .child(div().flex_1().min_w_0().truncate().child(label.to_owned()))
        .children(active.then(|| icon(Icon::Check, px(14.), theme::accent())))
}

#[cfg(test)]
mod tests {
    use super::{covers, Only, Pick};
    use heronite_data::InstallFilter;

    #[test]
    fn a_shown_control_replaces_only_its_own_chip() {
        let install = Pick::Show(InstallFilter::Installed);
        let favourites = Pick::Only(Only::Favourites);
        assert!(!covers(&install, 0) && !covers(&install, 1));
        assert!(covers(&install, 2));
        assert!(!covers(&favourites, 2));
        assert!(covers(&favourites, 3));
        for o in Only::ALL.into_iter().filter(|o| *o != Only::Favourites) {
            assert!(!covers(&Pick::Only(o), 5));
        }
        assert!(!covers(&Pick::Hidden, 5));
    }
}
