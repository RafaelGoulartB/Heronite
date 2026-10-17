//! "Group by" and "Sort" of the View menu: the three fields used most as
//! segments, then "More", which opens the other fields under the row and,
//! once one is picked, shows its name. Also the names, icons and directions
//! of the fields, shared with the command palette.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{GroupBy, SortMode};
use heronite_i18n::tr;

use super::super::data::group_icon;
use super::super::CollectionView;
use super::render::{control, control_hover, hint_text, section_head, seg_item, seg_track, Density};
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel};
use crate::ui::icon;

/// Which row's "More" list is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::collection) enum More {
    Group,
    Sort,
}

/// Segments before "More".
const QUICK_GROUPS: [GroupBy; 3] = [GroupBy::None, GroupBy::Status, GroupBy::Store];
const QUICK_SORTS: [SortMode; 3] = [SortMode::Title, SortMode::LastPlayed, SortMode::Playtime];

pub(in crate::collection) fn sort_label(mode: SortMode) -> &'static str {
    match mode {
        SortMode::Title => tr!("Title"),
        SortMode::LastPlayed => tr!("Last played"),
        SortMode::Playtime => tr!("Playtime"),
        SortMode::DateAdded => tr!("Date added"),
        SortMode::TimesPlayed => tr!("Times played"),
        SortMode::InstallSize => tr!("Install size"),
        SortMode::ReleaseDate => tr!("Release date"),
        SortMode::Rating => tr!("Your rating"),
        SortMode::CriticScore => tr!("Critic score"),
    }
}

/// The name in a segment, where room is short.
fn sort_short(mode: SortMode) -> &'static str {
    match mode {
        SortMode::LastPlayed => tr!("Last"),
        SortMode::DateAdded => tr!("Added"),
        SortMode::TimesPlayed => tr!("Sessions"),
        SortMode::InstallSize => tr!("Size"),
        SortMode::ReleaseDate => tr!("Release"),
        SortMode::Rating => tr!("Rating"),
        SortMode::CriticScore => tr!("Score"),
        other => sort_label(other),
    }
}

pub(in crate::collection) fn sort_icon(mode: SortMode) -> Icon {
    match mode {
        SortMode::Title => Icon::Label,
        SortMode::LastPlayed => Icon::History,
        SortMode::Playtime => Icon::AccessTime,
        SortMode::DateAdded => Icon::Inbox,
        SortMode::TimesPlayed => Icon::Replay,
        SortMode::InstallSize => Icon::FaHardDrive,
        SortMode::ReleaseDate => Icon::Event,
        SortMode::Rating => Icon::Star,
        SortMode::CriticScore => Icon::BarChart,
    }
}

/// What the order is in words ("A → Z", "Newest first"...): each field has
/// its own first.
pub(in crate::collection) fn sort_direction(mode: SortMode, descending: bool) -> &'static str {
    let (natural, turned) = match mode {
        SortMode::Title => (tr!("A → Z"), tr!("Z → A")),
        SortMode::LastPlayed => (tr!("Recent first"), tr!("Oldest first")),
        SortMode::Playtime | SortMode::TimesPlayed => (tr!("Most first"), tr!("Least first")),
        SortMode::DateAdded | SortMode::ReleaseDate => (tr!("Newest first"), tr!("Oldest first")),
        SortMode::InstallSize => (tr!("Largest first"), tr!("Smallest first")),
        SortMode::Rating => (tr!("Best first"), tr!("Worst first")),
        SortMode::CriticScore => (tr!("Highest first"), tr!("Lowest first")),
    };
    if descending {
        turned
    } else {
        natural
    }
}

fn sort_section(mode: SortMode) -> &'static str {
    match mode {
        SortMode::DateAdded | SortMode::TimesPlayed | SortMode::InstallSize => tr!("Library"),
        _ => tr!("Game info"),
    }
}

pub(in crate::collection) fn group_label(by: GroupBy) -> &'static str {
    match by {
        GroupBy::None => tr!("None"),
        GroupBy::Status => tr!("Status"),
        GroupBy::Category => tr!("Category"),
        GroupBy::Store => tr!("Store"),
        GroupBy::Platform => tr!("Platform"),
        GroupBy::Genre => tr!("Genre"),
        GroupBy::Developer => tr!("Developer"),
        GroupBy::Series => tr!("Series"),
        GroupBy::Year => tr!("Release year"),
        GroupBy::Playtime => tr!("Playtime"),
        GroupBy::InstallSize => tr!("Install size"),
    }
}

fn group_short(by: GroupBy) -> &'static str {
    match by {
        GroupBy::Year => tr!("Year"),
        GroupBy::InstallSize => tr!("Size"),
        other => group_label(other),
    }
}

fn group_section(by: GroupBy) -> &'static str {
    match by {
        GroupBy::Category | GroupBy::Platform => tr!("Library"),
        GroupBy::Playtime | GroupBy::InstallSize => tr!("Activity"),
        _ => tr!("Game info"),
    }
}

impl CollectionView {
    pub(in crate::collection) fn set_sort(&mut self, mode: SortMode, cx: &mut Context<Self>) {
        self.view_menu.more = None;
        if self.ui.sort != mode {
            self.ui.sort = mode;
            self.save_ui();
            self.refresh_groups();
        }
        cx.notify();
    }

    pub(in crate::collection) fn set_group_by(&mut self, by: GroupBy, cx: &mut Context<Self>) {
        self.view_menu.more = None;
        if self.ui.group_by != by {
            self.ui.group_by = by;
            self.save_ui();
            self.refresh_groups();
        }
        cx.notify();
    }

    fn toggle_more(&mut self, which: More, cx: &mut Context<Self>) {
        self.view_menu.more = if self.view_menu.more == Some(which) { None } else { Some(which) };
        self.view_menu.select_open = false;
        self.view_menu.more_hovered = false;
        cx.notify();
    }

    pub(super) fn vm_group(&self, d: Density, cx: &mut Context<Self>) -> Div {
        let current = self.ui.group_by;
        let compact = self.view_menu.compact;
        let quick = QUICK_GROUPS.into_iter().map(|by| {
            let on = current == by;
            seg_item(("vm-group", by as usize), on, d)
                .flex_auto()
                .children(quick_group_icon(by).filter(|_| !compact).map(|ic| icon(ic, px(15.), seg_color(on))))
                .child(group_label(by))
                .on_click(cx.listener(move |this, _, _, cx| this.set_group_by(by, cx)))
        });
        let quick: Vec<_> = quick.collect();
        let extra = (!QUICK_GROUPS.contains(&current)).then_some(current);
        let more = self.more_segment("vm-group-more", More::Group, extra.map(|by| (group_icon(by), group_short(by))), d, cx);
        let options = GroupBy::ALL.into_iter().filter(|by| !QUICK_GROUPS.contains(by)).map(|by| MoreOption {
            id: by as usize,
            label: group_label(by),
            icon: group_icon(by).unwrap_or(Icon::Flag),
            section: group_section(by),
            on: current == by,
        });
        let list = (self.view_menu.more == Some(More::Group)).then(|| {
            self.more_list("vm-group-list", options.collect(), cx, |this, id, cx| {
                if let Some(by) = GroupBy::ALL.into_iter().find(|b| *b as usize == id) {
                    this.set_group_by(by, cx);
                }
            })
        });
        let hint = current.many_per_game().then(|| hint_text(tr!("A game with several values shows in each of their groups.")).px(px(4.)));
        div()
            .flex()
            .flex_col()
            .gap(px(d.head_gap))
            .child(section_head(tr!("Group by")))
            .child(div().relative().child(seg_track("vm-group", d).children(quick).child(more)).children(list))
            .children(hint)
    }

    pub(super) fn vm_sort(&self, d: Density, cx: &mut Context<Self>) -> Div {
        let current = self.ui.sort;
        let compact = self.view_menu.compact;
        let quick = QUICK_SORTS.into_iter().map(|mode| {
            let on = current == mode;
            seg_item(("vm-sort", mode as usize), on, d)
                .flex_auto()
                .when(!compact, |s| s.child(icon(sort_icon(mode), px(15.), seg_color(on))))
                .child(sort_short(mode))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(mode, cx)))
        });
        let quick: Vec<_> = quick.collect();
        let extra = (!QUICK_SORTS.contains(&current)).then_some(current);
        let more = self.more_segment("vm-sort-more", More::Sort, extra.map(|m| (Some(sort_icon(m)), sort_short(m))), d, cx);
        let options = SortMode::ALL.into_iter().filter(|m| !QUICK_SORTS.contains(m)).map(|m| MoreOption {
            id: m as usize,
            label: sort_label(m),
            icon: sort_icon(m),
            section: sort_section(m),
            on: current == m,
        });
        let list = (self.view_menu.more == Some(More::Sort)).then(|| {
            self.more_list("vm-sort-list", options.collect(), cx, |this, id, cx| {
                if let Some(mode) = SortMode::ALL.into_iter().find(|m| *m as usize == id) {
                    this.set_sort(mode, cx);
                }
            })
        });
        let descending = self.ui.sort_descending;
        let dir = div()
            .id("vm-sort-dir")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(d.row_h))
            .rounded_full()
            .map(control)
            .cursor_pointer()
            .hover(|s| s.bg(control_hover()))
            // Up for A → Z, down for "most / newest first", as the words say.
            .child(icon(
                if (current == SortMode::Title) == descending { Icon::ArrowDownward } else { Icon::ArrowUpward },
                px(16.),
                theme::text_primary(),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sort_direction(cx)));
        let head = section_head(tr!("Sort")).child(div().text_size(px(12.)).child(sort_direction(current, descending)));
        let hint = (current == SortMode::Title)
            .then(|| hint_text(tr!("Ignores “The”, “A” and “An” at the start; numbers go in order (VII before IX).")).px(px(4.)));
        div()
            .flex()
            .flex_col()
            .gap(px(d.head_gap))
            .child(head)
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().min_w_0().child(seg_track("vm-sort", d).children(quick).child(more)))
                    .child(dir)
                    .children(list),
            )
            .children(hint)
    }

    /// "More ▾", or the extra field picked with its icon.
    fn more_segment(
        &self,
        id: &'static str,
        which: More,
        extra: Option<(Option<Icon>, &'static str)>,
        d: Density,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let on = extra.is_some();
        let open = self.view_menu.more == Some(which);
        let compact = self.view_menu.compact;
        seg_item(id, on, d)
            .flex_auto()
            .when(open && !on, |s| s.bg(control_hover()).text_color(white()))
            .map(|s| match extra {
                Some((ic, label)) => s.children(ic.filter(|_| !compact).map(|ic| icon(ic, px(15.), seg_color(true)))).child(label),
                None => s.child(tr!("More")),
            })
            .child(icon(Icon::ExpandMore, px(16.), if on || open { theme::text_primary() } else { theme::text_dim() }))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_more(which, cx)))
    }

    /// The fields after the segments, by section, under the row's right end.
    fn more_list(
        &self,
        id: &'static str,
        options: Vec<MoreOption>,
        cx: &mut Context<Self>,
        on_pick: impl Fn(&mut Self, usize, &mut Context<Self>) + Copy + 'static,
    ) -> Deferred {
        let mut list = menu_panel(240.).w(px(240.));
        let mut section = None;
        for o in options {
            if section != Some(o.section) {
                section = Some(o.section);
                list = list.child(
                    div()
                        .px(px(10.))
                        .pt(px(6.))
                        .pb(px(2.))
                        .text_size(px(11.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::text_muted())
                        .child(o.section.to_uppercase()),
                );
            }
            let pick = o.id;
            list = list.child(
                menu_item((id, o.id), o.on)
                    .child(icon(o.icon, px(16.), if o.on { theme::accent() } else { theme::text_muted() }))
                    .child(div().flex_1().min_w_0().truncate().child(o.label))
                    .when(o.on, |d| d.child(icon(Icon::Check, px(15.), theme::accent())))
                    .on_click(cx.listener(move |this, _, _, cx| on_pick(this, pick, cx))),
            );
        }
        deferred(
            div()
                .id(id)
                .absolute()
                .top(relative(1.))
                .right_0()
                .pt(px(4.))
                .occlude()
                .on_hover(cx.listener(|this, hovered: &bool, _, _| this.view_menu.more_hovered = *hovered))
                .child(list),
        )
        .with_priority(40)
    }
}

struct MoreOption {
    id: usize,
    label: &'static str,
    icon: Icon,
    section: &'static str,
    on: bool,
}

/// Icon of a quick "Group by" segment: Status gets the flag of the Browse
/// row (one colour, as the other segments).
fn quick_group_icon(by: GroupBy) -> Option<Icon> {
    match by {
        GroupBy::Status => Some(Icon::Flag),
        other => group_icon(other),
    }
}

fn seg_color(on: bool) -> Hsla {
    if on {
        white()
    } else {
        theme::seg_text().into()
    }
}
