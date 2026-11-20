//! The View menu's parts that pick what shows: the "Filter" block (Show with
//! "Include hidden", the "Only" switches, the Categories of both kinds,
//! "Filter by" with the other kinds and "Ages up to" in its "More" list) and
//! the footer's "Manage" list.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::FacetKind;
use heronite_i18n::{tr, trc};

use heronite_data::categories::UNCATEGORIZED;

use super::super::view_filters::{category_label, Only};
use super::super::CollectionView;
use super::choices::{self, Kind};
use super::organize::More;
use super::render::{check, control, control_hover, count_label, dot, hint_text, pill, section_head};
use super::Pick;
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel, menu_separator};
use crate::ui::icon;

/// Kinds with a pill of their own in "Filter by"; the others are in its "More" list.
/// Categories have a section of their own.
const FILTER_BY: [Kind; 3] = [Kind::Facet(FacetKind::Genre), Kind::Facet(FacetKind::Store), Kind::Status];

/// Rows of category chips shown before "Show all": a long list would push
/// Filter by, Sort and Group by out of the panel.
pub(super) const CATEGORY_ROWS: usize = 5;
/// Space between the category chips.
const CHIP_GAP: f32 = 6.;

/// The end of the first rows of a wrapping row of chips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RowsCut {
    /// Height of those rows.
    height: f32,
    /// Chips in them (the first ones, in order).
    shown: usize,
}

/// Where the first `rows` rows of chips laid out at `bounds` end, or `None`
/// when there are no more rows than that.
fn rows_cut(bounds: &[Bounds<Pixels>], rows: usize) -> Option<RowsCut> {
    if rows == 0 {
        return None;
    }
    let first = bounds.iter().map(|b| f32::from(b.top())).reduce(f32::min)?;
    let mut tops: Vec<f32> = Vec::new();
    for b in bounds {
        let top = f32::from(b.top());
        if !tops.iter().any(|t| (t - top).abs() < 1.) {
            tops.push(top);
        }
    }
    if tops.len() <= rows {
        return None;
    }
    tops.sort_by(f32::total_cmp);
    let last = tops[rows - 1];
    let in_rows = |b: &&Bounds<Pixels>| f32::from(b.top()) < last + 1.;
    let bottom = bounds.iter().filter(in_rows).map(|b| f32::from(b.bottom())).reduce(f32::max)?;
    Some(RowsCut { height: bottom - first, shown: bounds.iter().filter(in_rows).count() })
}

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

    /// The Categories section: a chip for every category, manual ones (their
    /// colour's dot) then smart ones (the sparkle), each with its games;
    /// "Uncategorized"; "Manage…" in the title, and "Save filters as smart
    /// category…" while other filters are on.
    pub(super) fn vm_categories(&self, cx: &mut Context<Self>) -> Div {
        let d = self.vm_density();
        let entries = self.category_entries();
        let manage = div()
            .id("vm-cats-manage")
            .cursor_pointer()
            .text_color(theme::accent())
            .hover(|s| s.text_color(theme::text_bright()))
            .child(tr!("Manage…"))
            .on_click(cx.listener(|this, _, window, cx| this.open_manage_categories(window, cx)));
        let chip = |id: ElementId, on: bool| {
            pill(id, d).when(on, |s| s.bg(theme::accent_tint()).border_color(theme::accent_tint_border()).text_color(theme::text_bright()))
        };
        // Chip `i`: a category of `entries`, or "Uncategorized" after them.
        let uncategorized = !self.categories.is_empty();
        let count = entries.len() + usize::from(uncategorized);
        let picks: Vec<bool> = entries
            .iter()
            .map(|c| self.category_on(&c.r))
            .chain(uncategorized.then(|| self.ui.categories.contains(UNCATEGORIZED)))
            .collect();
        let make = |i: usize, id: ElementId| -> Stateful<Div> {
            let Some(c) = entries.get(i) else {
                return chip(id, picks[i])
                    .child(icon(Icon::Label, px(13.), theme::text_muted()))
                    .child(category_label(UNCATEGORIZED))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_category_filter(UNCATEGORIZED, cx)));
            };
            let r = c.r.clone();
            chip(id, picks[i])
                .child(if c.smart() {
                    icon(Icon::AutoAwesome, px(13.), c.colour).into_any_element()
                } else {
                    dot(c.colour).into_any_element()
                })
                .child(div().min_w_0().truncate().child(c.name.clone()))
                .child(count_label(self.category_count(&c.r)))
                .when(c.smart(), |el| {
                    el.tooltip(crate::ui::tooltip(tr!("Smart category: games join and leave it as they match its rules.")))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_category_ref(&r, cx)))
        };
        let chips: Vec<Stateful<Div>> =
            (0..count).map(|i| make(i, if i < entries.len() { ("vm-cat", i).into() } else { "vm-cat-none".into() })).collect();
        let picked = self.ui.categories.len() + self.ui.smart_groups.len();
        let save = (self.ui.active_filters() > picked).then(|| {
            div()
                .id("vm-cats-save")
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::accent())
                .hover(|s| s.text_color(theme::text_bright()))
                .child(icon(Icon::AutoAwesome, px(13.), theme::accent()))
                .child(tr!("Save filters as smart category…"))
                .on_click(cx.listener(|this, _, window, cx| this.save_filters_as_group(window, cx)))
        });
        // Past CATEGORY_ROWS rows the chips are cut off until "Show all"; the
        // picked ones that are cut off show after the rows, so a filter on
        // never hides. The rows are only known once laid out: each frame
        // measures them, and a change draws once more.
        let measured = self.view_menu.cats_cut.clone();
        let cut = measured.get();
        let all = self.view_menu.cats_all;
        let folded = cut.filter(|_| !all);
        let wrap = (count > 0).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(CHIP_GAP))
                .children(chips)
                .on_children_prepainted(move |bounds, window, _| {
                    let rows = rows_cut(&bounds, CATEGORY_ROWS);
                    if measured.get() != rows {
                        measured.set(rows);
                        window.request_animation_frame();
                    }
                })
                .when_some(folded, |el, c| el.max_h(px(c.height)).overflow_hidden())
        });
        let cut_picks: Vec<usize> = folded.map(|c| (c.shown..count).filter(|i| picks[*i]).collect()).unwrap_or_default();
        let picked_after = (!cut_picks.is_empty())
            .then(|| div().flex().flex_wrap().gap(px(CHIP_GAP)).children(cut_picks.into_iter().map(|i| make(i, ("vm-cat-cut", i).into()))));
        let more = cut.is_some().then(|| {
            div()
                .id("vm-cats-more")
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::accent())
                .hover(|s| s.text_color(theme::text_bright()))
                .child(if all { tr!("Show fewer").to_string() } else { tr!("Show all ({count})", count = count) })
                .child(
                    icon(Icon::ExpandMore, px(14.), theme::accent())
                        .when(all, |i| i.with_transformation(Transformation::rotate(percentage(0.5)))),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.view_menu.cats_all = !this.view_menu.cats_all;
                    cx.notify();
                }))
        });
        let empty = entries.is_empty().then(|| {
            hint_text(tr!("Sort games your own way: pick them by hand, or let a smart category gather them by its rules.")).px(px(4.))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(d.head_gap))
            .child(section_head(tr!("Categories")).child(manage))
            .children(empty)
            .children(wrap)
            .children(picked_after)
            .children(more)
            .children(save)
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
        let pills = FILTER_BY.into_iter().enumerate().map(|(i, kind)| {
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
            .child(div().flex().flex_wrap().gap(px(6.)).children(pills).child(div().relative().flex_none().child(more).children(list)))
            .children(ages)
    }

    fn vm_filter_more_list(&self, cx: &mut Context<Self>) -> Deferred {
        let kinds = Kind::ALL.into_iter().filter(|k| !matches!(k, Kind::Only | Kind::Group | Kind::Category) && !FILTER_BY.contains(k));
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
        popover("vm-filter-more-list", list, |el| el.top(relative(1.)).left_0().pt(px(4.)), cx)
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
            list = list.child(
                menu_item("vm-manage-categories", false)
                    .child(icon(Icon::Label, px(16.), theme::text_muted()))
                    .child(tr!("Manage categories…"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        close(this);
                        this.open_manage_categories(window, cx);
                    })),
            );
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

/// The word over the "Filter" block.
pub(super) fn filter_block() -> &'static str {
    trc!("view menu block", "Filter")
}

/// The word over the "Organize" block.
pub(super) fn organize_block() -> &'static str {
    trc!("view menu block", "Organize")
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, px, size, Bounds, Pixels};

    use super::{rows_cut, RowsCut};

    fn chip(x: f32, y: f32, w: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(30.)))
    }

    /// Three chips a row, `rows` rows, 6px apart, from y = 100.
    fn grid(rows: usize) -> Vec<Bounds<Pixels>> {
        (0..rows).flat_map(|r| (0..3).map(move |c| chip(c as f32 * 90., 100. + r as f32 * 36., 84.))).collect()
    }

    #[test]
    fn rows_cut_ends_after_the_last_row_shown() {
        assert_eq!(rows_cut(&[], 5), None);
        assert_eq!(rows_cut(&grid(5), 5), None, "five rows fit");
        assert_eq!(rows_cut(&grid(7), 5), Some(RowsCut { height: 4. * 36. + 30., shown: 15 }));
        assert_eq!(rows_cut(&grid(2), 0), None);
    }

    #[test]
    fn rows_cut_counts_a_short_last_row_and_rounding() {
        // Sub-pixel tops in one row are one row; the cut row is partly full.
        let mut b = grid(5);
        b[1].origin.y += px(0.4);
        b.truncate(13);
        b.push(chip(0., 100. + 5. * 36., 84.));
        assert_eq!(rows_cut(&b, 5), Some(RowsCut { height: 4. * 36. + 30., shown: 13 }));
        assert_eq!(rows_cut(&b, 6), None);
    }
}
