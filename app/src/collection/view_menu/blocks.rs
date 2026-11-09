//! The View menu's parts that pick what shows: the smart groups select next
//! to the presets one, the "Filter" block (Show with "Include hidden", the
//! "Only" switches, "Filter by" with the other kinds and "Ages up to" in its
//! "More" list) and the footer's "Manage" list.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::FacetKind;
use heronite_i18n::{tr, trc, trn};

use super::super::smart_groups::group_colour;
use super::super::view_filters::Only;
use super::super::CollectionView;
use super::choices::{self, Kind};
use super::organize::More;
use super::render::{check, control, control_hover, count_label, dot, pill, section_head, Density};
use super::Pick;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel, menu_separator};
use crate::ui::icon;

/// Kinds with a pill of their own in "Filter by"; the others are in its "More" list.
const FILTER_BY: [Kind; 4] = [Kind::Facet(FacetKind::Genre), Kind::Facet(FacetKind::Store), Kind::Status, Kind::Category];

impl CollectionView {
    /// A block's title: a short bold word and a line to the panel's edge.
    pub(super) fn vm_block_head(&self, label: &'static str) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(4.))
            .text_size(px(12.5))
            .font_weight(theme::W_BOLD)
            .text_color(theme::text_primary())
            .child(label)
            .child(div().flex_1().h(px(1.)).bg(theme::ink_a(0.08)))
    }

    /// The smart groups select, beside the presets one: the picked group (or
    /// how many), and a list to pick, pin, edit and make groups.
    pub(super) fn vm_groups_select(&self, d: Density, cx: &mut Context<Self>) -> Div {
        let open = self.view_menu.more == Some(More::Groups);
        let picked: Vec<_> = self.ui.smart_groups.iter().filter_map(|id| self.smart_groups.get(id)).collect();
        let label: SharedString = match picked.as_slice() {
            [] => tr!("None").into(),
            [one] => one.name.clone().into(),
            many => trn!(many.len(), "{n} group", "{n} groups").into(),
        };
        let trigger = div()
            .id("vm-groups-select")
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(d.row_h))
            .pl(px(12.))
            .pr(px(9.))
            .rounded_full()
            .map(control)
            .when(open, |s| s.bg(control_hover()))
            .cursor_pointer()
            .hover(|s| s.bg(control_hover()))
            .text_size(px(d.text))
            .text_color(if picked.is_empty() { theme::text_muted() } else { theme::text_primary() })
            .children(picked.first().filter(|_| picked.len() == 1).map(|g| dot(group_colour(g))))
            .children((picked.len() > 1).then(|| icon(Icon::AutoAwesome, px(14.), theme::accent())))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(icon(Icon::ExpandMore, px(16.), if open { theme::text_primary() } else { theme::text_dim() }))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_more(More::Groups, cx)));
        let list = open.then(|| self.vm_groups_list(cx));
        div().relative().child(trigger).children(list)
    }

    /// Every group: a click shows its games (several add up), the pin puts it
    /// in the header, the pencil edits it. Then "New" and "Save filters as".
    fn vm_groups_list(&self, cx: &mut Context<Self>) -> Deferred {
        let mut list = menu_panel(300.).w(px(300.));
        for (i, g) in self.smart_groups.groups.iter().enumerate() {
            let on = self.ui.smart_groups.contains(&g.id);
            let (id, pin_id, edit_id, pinned) = (g.id.clone(), g.id.clone(), g.id.clone(), g.pinned);
            list = list.child(
                menu_item(("vm-group", i), false)
                    .child(check(on))
                    .child(dot(group_colour(g)))
                    .child(div().flex_1().min_w_0().truncate().when(on, |d| d.text_color(theme::text_bright())).child(g.name.clone()))
                    .child(count_label(self.group_count(g)))
                    .child(
                        round(("vm-group-pin", i))
                            .child(if pinned {
                                icon(Icon::PushPin, px(15.), theme::accent())
                            } else {
                                icon(Icon::PushPinOutlined, px(15.), theme::text_dim())
                            })
                            .tooltip(crate::ui::tooltip(if pinned { tr!("Unpin from the header") } else { tr!("Pin to the header") }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.set_group_pinned(&pin_id, !pinned, cx);
                            })),
                    )
                    .child(
                        round(("vm-group-edit", i))
                            .child(icon(Icon::Edit, px(15.), theme::text_muted()))
                            .tooltip(crate::ui::tooltip(tr!("Edit rules")))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_group_editor(Some(&edit_id), None, window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_smart_group_filter(&id, cx))),
            );
        }
        if self.smart_groups.is_empty() {
            list = list.child(
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.5))
                    .text_color(theme::text_dim())
                    .child(tr!("A smart group keeps itself up to date: games join and leave it as they match its rules.")),
            );
        }
        list = list.child(menu_separator()).child(
            menu_item("vm-group-new", false)
                .child(icon(Icon::Add, px(16.), theme::text_muted()))
                .child(div().flex_1().child(tr!("New smart group…")))
                .on_click(cx.listener(|this, _, window, cx| this.open_group_editor(None, None, window, cx))),
        );
        if self.ui.active_filters() > self.ui.smart_groups.len() {
            list = list.child(
                menu_item("vm-group-save", false)
                    .child(icon(Icon::AutoAwesome, px(16.), theme::text_muted()))
                    .child(div().flex_1().child(tr!("Save filters as smart group…")))
                    .on_click(cx.listener(|this, _, window, cx| this.save_filters_as_group(window, cx))),
            );
        }
        popover("vm-groups-list", list, |el| el.top(relative(1.)).right_0().pt(px(4.)), cx)
    }

    /// "Include hidden", in the Show row's title.
    pub(super) fn vm_include_hidden(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let on = self.ui.show_hidden;
        div()
            .id("vm-hidden")
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(if on { theme::text_bright() } else { theme::text_muted().into() })
            .hover(|s| s.text_color(theme::text_bright()))
            .child(check(on).size(px(15.)))
            .child(tr!("Include hidden"))
            .on_click(cx.listener(|this, _, _, cx| this.apply_pick(&Pick::Hidden, cx)))
    }

    /// The "Only" switches, one pill each (accent while on).
    pub(super) fn vm_only(&self, cx: &mut Context<Self>) -> Div {
        let d = self.vm_density();
        let pills =
            [(Only::Favourites, Icon::Star), (Only::Updates, Icon::Update), (Only::Offline, Icon::WifiOff)].into_iter().map(|(o, ic)| {
                let on = self.only_on(o);
                pill(("vm-only", o as usize), d)
                    .when(on, |s| s.bg(theme::accent_tint()).border_color(theme::accent_tint_border()).text_color(theme::text_bright()))
                    .child(icon(ic, px(14.), if on { theme::accent() } else { theme::text_muted() }))
                    .child(o.label())
                    .on_click(cx.listener(move |this, _, _, cx| this.apply_pick(&Pick::Only(o), cx)))
            });
        div()
            .flex()
            .flex_col()
            .gap(px(d.head_gap))
            .child(section_head(tr!("Only")))
            .child(div().flex().flex_wrap().gap(px(6.)).children(pills))
    }

    /// The kinds used most as pills (each fills the search with `kind:`),
    /// then "More" with the others and "Ages up to", whose row shows under
    /// them once picked or while it filters.
    pub(super) fn vm_filter_by(&self, cx: &mut Context<Self>) -> Div {
        let d = self.vm_density();
        let shown = |k: &Kind| *k != Kind::Category || !self.categories.is_empty();
        let pills = FILTER_BY.into_iter().filter(shown).enumerate().map(|(i, kind)| {
            pill(("vm-filter-by", i), d).child(icon(kind.icon(), px(14.), theme::text_muted())).child(kind.label()).on_click(cx.listener(
                move |this, _, window, cx| {
                    let text = choices::scoped_query(kind, "");
                    this.set_view_query(&text, window, cx);
                },
            ))
        });
        let pills: Vec<_> = pills.collect();
        let open = self.view_menu.more == Some(More::Filters);
        let more = pill("vm-filter-more", d)
            .when(open, |s| s.bg(control_hover()))
            .text_color(theme::accent())
            .child(tr!("More"))
            .child(icon(Icon::ExpandMore, px(15.), theme::accent()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_more(More::Filters, cx)));
        let list = open.then(|| self.vm_filter_more_list(cx));
        let ages = (self.view_menu.ages_open || self.ui.age.is_active()).then(|| self.vm_age(cx));
        div()
            .flex()
            .flex_col()
            .gap(px(d.head_gap))
            .child(section_head(tr!("Filter by")))
            .child(div().relative().flex().flex_wrap().gap(px(6.)).children(pills).child(more).children(list))
            .children(ages)
    }

    fn vm_filter_more_list(&self, cx: &mut Context<Self>) -> Deferred {
        let kinds = Kind::ALL.into_iter().filter(|k| !matches!(k, Kind::Only | Kind::Group) && !FILTER_BY.contains(k));
        let mut list = menu_panel(240.).w(px(240.));
        for (i, kind) in kinds.enumerate() {
            list = list.child(
                menu_item(("vm-filter-kind", i), false)
                    .child(icon(kind.icon(), px(16.), theme::text_muted()))
                    .child(div().flex_1().child(kind.label()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.view_menu.more = None;
                        let text = choices::scoped_query(kind, "");
                        this.set_view_query(&text, window, cx);
                    })),
            );
        }
        let ages_on = self.view_menu.ages_open || self.ui.age.is_active();
        list = list.child(menu_separator()).child(
            menu_item("vm-filter-ages", ages_on)
                .child(icon(Icon::AccountCircle, px(16.), theme::text_muted()))
                .child(div().flex_1().child(tr!("Ages up to")))
                .when(ages_on, |d| d.child(icon(Icon::Check, px(15.), theme::accent())))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.view_menu.more = None;
                    this.view_menu.ages_open = true;
                    cx.notify();
                })),
        );
        popover("vm-filter-more-list", list, |el| el.top(relative(1.)).right_0().pt(px(4.)), cx)
    }

    /// "Manage ▾" in the footer: the managers that change the library's
    /// lists, opening above it.
    pub(super) fn vm_manage_button(&self, cx: &mut Context<Self>) -> Div {
        let open = self.view_menu.more == Some(More::Manage);
        let button = div()
            .id("vm-manage")
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(26.))
            .px(px(9.))
            .rounded_full()
            .map(control)
            .when(open, |s| s.bg(control_hover()))
            .cursor_pointer()
            .hover(|s| s.bg(control_hover()))
            .text_color(theme::text_primary())
            .child(icon(Icon::Settings, px(14.), theme::text_muted()))
            .child(tr!("Manage"))
            .child(icon(Icon::ExpandMore, px(14.), theme::text_dim()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_more(More::Manage, cx)));
        let list = open.then(|| {
            let close = |this: &mut Self| {
                this.view_menu.more = None;
                this.open_menu = None;
            };
            let mut list = menu_panel(220.).w(px(220.)).child(
                menu_item("vm-manage-statuses", false)
                    .child(icon(Icon::Flag, px(16.), theme::text_muted()))
                    .child(tr!("Statuses…"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        close(this);
                        this.open_manage_statuses(window, cx);
                    })),
            );
            if !self.categories.is_empty() {
                list = list.child(
                    menu_item("vm-manage-categories", false)
                        .child(icon(Icon::Label, px(16.), theme::text_muted()))
                        .child(tr!("Manage categories…"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            close(this);
                            this.open_manage_categories(window, cx);
                        })),
                );
            }
            list = list.child(
                menu_item("vm-manage-fields", false)
                    .child(icon(Icon::CallMerge, px(16.), theme::text_muted()))
                    .child(tr!("Library fields…"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        close(this);
                        this.open_manage_fields(None, window, cx);
                    })),
            );
            if !self.ui.saved_views.is_empty() {
                list = list.child(
                    menu_item("vm-manage-presets", false)
                        .child(icon(Icon::Edit, px(16.), theme::text_muted()))
                        .child(tr!("Edit presets…"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view_menu.more = None;
                            this.view_menu.editing = true;
                            cx.notify();
                        })),
                );
            }
            list = list.child(
                menu_item("vm-manage-group", false)
                    .child(icon(Icon::AutoAwesome, px(16.), theme::text_muted()))
                    .child(tr!("New smart group…"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_group_editor(None, None, window, cx))),
            );
            popover("vm-manage-list", list, |el| el.bottom(relative(1.)).right_0().pb(px(4.)), cx)
        });
        div().relative().child(button).children(list)
    }
}

/// A list floating over the menu: a click in it is not outside the menu.
fn popover(id: &'static str, list: Div, place: impl FnOnce(Stateful<Div>) -> Stateful<Div>, cx: &mut Context<CollectionView>) -> Deferred {
    let el = div().id(id).absolute().occlude().on_hover(cx.listener(|this, hovered: &bool, _, _| this.view_menu.more_hovered = *hovered));
    deferred(place(el).child(list)).with_priority(40)
}

/// 26px round button inside a list row (the row keeps its own click).
fn round(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded_full()
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.1)))
}

/// The word over the "Filter" block.
pub(super) fn filter_block() -> &'static str {
    trc!("view menu block", "Filter")
}

/// The word over the "Organize" block.
pub(super) fn organize_block() -> &'static str {
    trc!("view menu block", "Organize")
}
