//! Optional Collection shortcuts, off by default: one capsule after the
//! View button. A control at its default shows only its icon; a value in use
//! adds its label, in the accent when it hides games. While a control is
//! shown, its filter chip is not (`layout::fit`).
//!
//! The zero-basis header lane is measured after layout; text is shaped in the
//! same font as the controls. Resizing never writes settings, and a vanished
//! trigger cannot keep its menu open.

mod layout;

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{GroupBy, InstallFilter};
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

const SEGMENT_H: f32 = 26.;
const LABEL: f32 = 12.5;
const ICON: f32 = 14.;
/// Side padding of a segment with a label, and of an icon-only one.
const PAD: f32 = 8.;
const ICON_PAD: f32 = 6.;
const INNER_GAP: f32 = 5.;
const CAPSULE_PAD: f32 = 3.;
/// The capsule's padding and 1px border, both sides.
const CHROME: f32 = 2. * CAPSULE_PAD + 2.;
const PRESET_LABEL_MAX: f32 = 160.;
/// The View button and the arrow that shows or hides the capsule.
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

/// Whether a capsule showing `shown` controls already carries this filter,
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

fn segment_width(label: Option<f32>) -> f32 {
    match label {
        Some(text) => 2. * PAD + ICON + INNER_GAP + text,
        None => 2. * ICON_PAD + ICON,
    }
}

/// This render's segment widths, from the labels they show now.
pub(super) struct Widths {
    sort: f32,
    direction: f32,
    install: f32,
    favourites: f32,
    group: f32,
    preset: f32,
    presets: bool,
}

impl Widths {
    /// Lane cost of each control in priority order. A separator (`GAP`)
    /// precedes every control after the first; the last control's `GAP`
    /// is the space before the chips.
    fn items(&self) -> Vec<f32> {
        let mut items = vec![CHROME + self.sort + self.direction, self.install, self.favourites, self.group];
        if self.presets {
            items.push(self.preset);
        }
        items
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// At its default.
    Plain,
    /// Changed, but hides no game (sort, grouping, preset).
    Set,
    /// A filter that hides games.
    Filter,
}

impl Tone {
    fn color(self) -> Hsla {
        match self {
            Self::Plain => theme::text_muted().into(),
            Self::Set => theme::text_bright(),
            Self::Filter => theme::accent().into(),
        }
    }
}

fn segment(id: impl Into<ElementId>, width: f32, tone: Tone, open: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(INNER_GAP))
        .h(px(SEGMENT_H))
        .w(px(width))
        .rounded_full()
        .text_size(px(LABEL))
        .text_color(tone.color())
        .cursor_pointer()
        .when(tone == Tone::Filter, |d| d.bg(theme::accent_tint()))
        .when(open, |d| d.bg(theme::ink_a(0.08)))
        .hover(move |s| {
            let s = s.bg(theme::ink_a(0.08));
            if tone == Tone::Filter {
                s
            } else {
                s.text_color(theme::text_bright())
            }
        })
}

fn separator() -> Div {
    div().flex_none().w(px(1.)).h(px(14.)).mx(px((GAP - 1.) / 2.)).bg(theme::ink_a(0.1))
}

impl CollectionView {
    pub(super) fn quick_widths(&self, window: &Window) -> Widths {
        let preset = self.current_view().map(|i| text_width(&self.ui.saved_views[i].name, window).min(PRESET_LABEL_MAX));
        Widths {
            sort: segment_width(Some(text_width(sort_label(self.ui.sort), window))),
            direction: segment_width(None),
            install: segment_width(
                (self.ui.install_filter != InstallFilter::All).then(|| text_width(install_label(self.ui.install_filter), window)),
            ),
            favourites: segment_width(None),
            group: segment_width((self.ui.group_by != GroupBy::None).then(|| text_width(group_label(self.ui.group_by), window))),
            preset: segment_width(preset),
            presets: !self.ui.saved_views.is_empty(),
        }
    }

    /// How many controls the capsule shows, given each filter chip's
    /// `(pick, width)`.
    pub(super) fn quick_fit(&self, widths: &Widths, chips: &[(Pick, f32)]) -> usize {
        let lane = self.toolbar.width.get().unwrap_or(0.);
        let chip_width = |shown: usize| chips.iter().filter(|(pick, _)| !covers(pick, shown)).map(|(_, w)| w + GAP).sum();
        layout::fit(theme::appearance().quick_toolbar_controls, lane, &widths.items(), chip_width)
    }

    /// The View button with the arrow that shows or hides the capsule on its
    /// right; the arrow appears while the pointer is over either.
    pub(super) fn view_button_group(&self, view: Stateful<Div>, cx: &mut Context<Self>) -> Div {
        let on = theme::appearance().quick_toolbar_controls;
        let toggle = div()
            .id("quick-toolbar-toggle")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(16.))
            .h(theme::TOOL_BTN)
            .rounded(px(theme::R_ITEM / 2.))
            .cursor_pointer()
            .opacity(0.)
            .group_hover(VIEW_GROUP, |s| s.opacity(1.))
            .hover(|s| s.bg(theme::ink_a(0.08)))
            .tooltip(tooltip(if on { tr!("Hide quick controls") } else { tr!("Show quick controls") }))
            .child(icon(if on { Icon::ChevronLeft } else { Icon::ChevronRight }, px(14.), theme::accent()))
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
        let sort = self.quick_select(
            QuickMenu::Sort,
            "quick-sort",
            Some(sort_label(mode)),
            sort_icon(mode),
            widths.sort,
            Tone::Set,
            (tr!("Sort"), sort_label(mode)),
            max_h,
            cx,
        );
        let direction = segment("quick-sort-direction", widths.direction, Tone::Plain, false)
            .tooltip(tooltip(sort_direction(mode, descending)))
            .child(icon(sort_direction_icon(mode, descending), px(ICON), Tone::Plain.color()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sort_direction(cx)));
        let mut capsule = div()
            .id("collection-quick-toolbar")
            .flex()
            .flex_none()
            .items_center()
            .h(theme::TOOL_BTN)
            .px(px(CAPSULE_PAD))
            .rounded_full()
            .border_1()
            .border_color(theme::ink_a(0.08))
            .bg(theme::search_fill())
            .child(sort)
            .child(direction);
        if visible > QuickMenu::Install.index() {
            let filter = self.ui.install_filter;
            let filtered = filter != InstallFilter::All;
            capsule = capsule.child(separator()).child(self.quick_select(
                QuickMenu::Install,
                "quick-install",
                filtered.then(|| install_label(filter)),
                Icon::Download,
                widths.install,
                if filtered { Tone::Filter } else { Tone::Plain },
                (tr!("Show"), install_label(filter)),
                max_h,
                cx,
            ));
        }
        if visible > FAVOURITES {
            let on = self.ui.only.favourites;
            let tone = if on { Tone::Filter } else { Tone::Plain };
            capsule = capsule.child(separator()).child(
                segment("quick-favourites", widths.favourites, tone, false)
                    .tooltip(tooltip(Only::Favourites.label()))
                    .child(icon(if on { Icon::Star } else { Icon::StarBorder }, px(ICON), tone.color()))
                    .on_click(cx.listener(|this, _, _, cx| this.apply_pick(&Pick::Only(Only::Favourites), cx))),
            );
        }
        if visible > QuickMenu::Group.index() {
            let by = self.ui.group_by;
            let grouped = by != GroupBy::None;
            capsule = capsule.child(separator()).child(self.quick_select(
                QuickMenu::Group,
                "quick-group",
                grouped.then(|| group_label(by)),
                Icon::Folder,
                widths.group,
                if grouped { Tone::Set } else { Tone::Plain },
                (tr!("Group by"), group_label(by)),
                max_h,
                cx,
            ));
        }
        if visible > QuickMenu::Presets.index() {
            let name = self.current_view().map(|i| self.ui.saved_views[i].name.clone());
            capsule = capsule.child(separator()).child(self.quick_select(
                QuickMenu::Presets,
                "quick-presets",
                name.as_deref(),
                Icon::Bookmark,
                widths.preset,
                if name.is_some() { Tone::Set } else { Tone::Plain },
                (tr!("Presets"), name.as_deref().unwrap_or(tr!("Custom"))),
                max_h,
                cx,
            ));
        }
        Some(capsule)
    }

    /// A segment that opens `menu`; `hint` is the tooltip's `(name, value)`.
    fn quick_select(
        &self,
        menu: QuickMenu,
        id: &'static str,
        label: Option<&str>,
        glyph: Icon,
        width: f32,
        tone: Tone,
        hint: (&str, &str),
        max_h: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.open_menu == Some(Menu::Quick(menu));
        let trigger = segment(id, width, tone, open)
            .tooltip(tooltip(format!("{}: {}", hint.0, hint.1)))
            .child(icon(glyph, px(ICON), tone.color()))
            .children(label.map(|label| div().flex_1().min_w_0().truncate().child(label.to_owned())));
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
