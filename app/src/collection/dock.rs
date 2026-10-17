//! The selection dock: a glass bar floating at the bottom of the Collection
//! while games are selected (`selection.rs`), with the batch changes. Status
//! and Categories open a popover above it; their rows are shared with the
//! context menu's flyouts for a selection.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};

use super::context_menu::item;
use super::selection::{DockPop, Step, Tri};
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::kbd;
use crate::ui::icon;

/// Height of the dock's buttons; the popovers open this far above them.
const BTN_H: f32 = 40.;

/// A round tick: filled when every selected game has it, a dash when only
/// some do, an empty ring when none does.
pub(super) fn tri_mark(t: Tri) -> Div {
    let d = div().size(px(18.)).flex_none().rounded_full().flex().items_center().justify_center().border_1();
    match t {
        Tri::All => d.bg(theme::accent()).border_color(theme::accent()).child(icon(Icon::Check, px(13.), theme::page_bg())),
        Tri::Some => d.border_color(theme::accent()).child(div().w(px(8.)).h(px(2.)).rounded(px(1.)).bg(theme::accent())),
        Tri::None => d.border_color(theme::white_a(0.32)),
    }
}

fn dock_button(id: &'static str, ic: Icon, label: Option<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(BTN_H))
        .px(px(if label.is_some() { 14. } else { 11. }))
        .rounded_full()
        .text_size(px(13.5))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .hover(|s| s.bg(theme::white_a(0.1)))
        .when(on, |d| d.bg(theme::white_a(0.1)))
        .child(icon(ic, px(18.), theme::text_muted()))
        .children(label)
}

impl CollectionView {
    /// The selected games' status: its dot and name when they share one.
    fn selection_status_label(&self, games: &[Arc<CollectionGame>]) -> AnyElement {
        let lib = self.lib.as_ref();
        let first = games.first().map(|g| g.status_id.as_str());
        let shared = first.filter(|id| games.iter().all(|g| g.status_id == *id));
        match shared.and_then(|id| lib?.statuses.iter().find(|s| s.id == id)) {
            Some(st) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().size(px(9.)).rounded_full().bg(theme::status_color(&st.slug)))
                .child(heronite_data::status::status_label(&st.name).to_string())
                .into_any_element(),
            None => div().italic().text_color(theme::text_muted()).child(tr!("Mixed")).into_any_element(),
        }
    }

    /// Status rows for the selection: a dot, the name, how many of them have
    /// it when only some do, a tick when all do. A click sets it on all.
    pub(super) fn batch_status_rows(&self, games: &[Arc<CollectionGame>], prefix: &str, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let Some(lib) = self.lib.clone() else { return Vec::new() };
        let total = games.len();
        lib.statuses
            .iter()
            .map(|st| {
                let n = games.iter().filter(|g| g.status_id == st.id).count();
                let id = st.id.clone();
                item(SharedString::from(format!("{prefix}-st-{}", st.id)))
                    .py(px(8.))
                    .child(div().size(px(10.)).flex_none().rounded_full().bg(theme::status_color(&st.slug)))
                    .child(div().min_w_0().truncate().child(heronite_data::status::status_label(&st.name).to_string()))
                    .child(div().flex_1())
                    .when(n > 0 && n < total, |d| d.child(div().text_size(px(12.)).text_color(theme::text_dim()).child(n.to_string())))
                    .when(n == total, |d| d.child(icon(Icon::Check, px(18.), theme::accent())))
                    .on_click(cx.listener(move |this, _, _, cx| this.batch_status(id.clone(), cx)))
            })
            .collect()
    }

    /// Category rows for the selection: a round tick (all / some / none),
    /// the name, "3 of 7" when only some have it, and under the pointer what
    /// a click does next. Clicks keep the list open.
    pub(super) fn batch_category_rows(&self, games: &[Arc<CollectionGame>], prefix: &str, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let total = games.len();
        self.categories
            .names()
            .enumerate()
            .map(|(i, name)| {
                let (tri, n, next) = self.category_tri(name, games);
                let group = SharedString::from(format!("{prefix}-cat-{i}"));
                let next = match next {
                    Step::All => tr!("add to all"),
                    Step::None => tr!("remove from all"),
                    Step::AsWas => tr!("as it was"),
                };
                let name = name.to_string();
                item(group.clone())
                    .group(group.clone())
                    .py(px(8.))
                    .child(tri_mark(tri))
                    .child(div().min_w_0().truncate().child(name.clone()))
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(theme::accent())
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .child(next),
                    )
                    .when(tri == Tri::Some, |d| {
                        d.child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("{n} of {total}", n, total)))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.batch_category(name.clone(), cx)))
            })
            .collect()
    }

    /// The categories list with its help line and "Manage categories…".
    pub(super) fn batch_categories_body(&self, games: &[Arc<CollectionGame>], prefix: &str, cx: &mut Context<Self>) -> Div {
        let mut list =
            div().id(SharedString::from(format!("{prefix}-cat-list"))).flex().flex_col().gap(px(2.)).overflow_y_scroll().max_h(px(300.));
        if self.categories.is_empty() {
            list = list
                .child(div().px(px(12.)).py(px(6.)).text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No categories yet.")));
        }
        list = list.children(self.batch_category_rows(games, prefix, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(list)
            .when(!self.categories.is_empty(), |d| {
                d.child(
                    div()
                        .mt(px(4.))
                        .px(px(12.))
                        .py(px(8.))
                        .border_t_1()
                        .border_color(theme::glass_border())
                        .text_size(px(11.5))
                        .line_height(px(16.))
                        .text_color(theme::text_dim())
                        .child(tr!("A dash means only some of them have it. Each click: add to all, remove from all, back as it was.")),
                )
            })
            .child(
                item(SharedString::from(format!("{prefix}-cat-manage")))
                    .py(px(8.))
                    .child(icon(Icon::Add, px(18.), theme::text_muted()))
                    .child(tr!("Manage categories…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sel.pop = None;
                        this.open_manage_categories(window, cx);
                    })),
            )
    }

    fn dock_popover(&self, pop: DockPop, games: &[Arc<CollectionGame>], cx: &mut Context<Self>) -> Div {
        let paper = crate::ui::glass::glass_menu_surface(theme::R_MENU).occlude().flex().flex_col().gap(px(2.)).p(px(6.));
        let body = match pop {
            DockPop::Status => paper.min_w(px(240.)).child(
                div()
                    .id("dock-st-list")
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .max_h(px(380.))
                    .overflow_y_scroll()
                    .children(self.batch_status_rows(games, "dock", cx)),
            ),
            DockPop::Categories => paper.w(px(320.)).child(self.batch_categories_body(games, "dock", cx)),
        };
        div().absolute().bottom(px(BTN_H + 12.)).left(px(-6.)).child(body)
    }

    fn dock_pop_button(
        &self,
        pop: DockPop,
        id: &'static str,
        ic: Icon,
        label: AnyElement,
        games: &[Arc<CollectionGame>],
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.sel.pop == Some(pop);
        let button = div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(BTN_H))
            .pl(px(14.))
            .pr(px(10.))
            .rounded_full()
            .text_size(px(13.5))
            .text_color(theme::text_primary())
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.1)))
            .when(open, |d| d.bg(theme::white_a(0.1)))
            .child(icon(ic, px(18.), theme::text_muted()))
            .child(label)
            .child(icon(Icon::ExpandMore, px(18.), theme::text_dim()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sel.pop = if this.sel.pop == Some(pop) { None } else { Some(pop) };
                cx.notify();
            }));
        div().relative().child(button).when(open, |d| d.child(self.dock_popover(pop, games, cx)))
    }

    /// The dock and, while a popover is open, a click catcher closing it.
    pub(super) fn render_dock(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.sel.active() || self.page != crate::pages::Page::Collection {
            return None;
        }
        let games = self.selected_games();
        if games.is_empty() {
            return None;
        }
        let n = games.len();
        // Narrow windows: the plain actions keep only their icons.
        let compact = f32::from(window.viewport_size().width) < 1180.;
        let label = |s: &str| (!compact).then(|| SharedString::from(s.to_string()));
        let shown = self.shown_keys();
        let all_shown = shown.iter().all(|k| self.sel.contains(k));
        let all_fav = games.iter().all(|g| g.favourite);
        let all_hidden = games.iter().all(|g| g.hidden);

        let count = div()
            .flex()
            .flex_col()
            .flex_none()
            .pl(px(4.))
            .pr(px(10.))
            .child(div().text_size(px(13.5)).font_weight(theme::W_500).child(trn!(n, "{n} game selected", "{n} games selected")))
            .when(!all_shown, |d| {
                d.child(
                    div()
                        .id("dock-all")
                        .text_size(px(11.5))
                        .text_color(theme::accent())
                        .cursor_pointer()
                        .hover(|s| s.text_color(white()))
                        .child(tr!("Select all {n}", n = shown.len()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_all_shown();
                            cx.notify();
                        })),
                )
            });
        let close = div()
            .id("dock-close")
            .size(px(BTN_H))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.1)))
            .child(icon(Icon::Close, px(18.), theme::text_muted()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.clear_selection();
                cx.notify();
            }));
        let status_label = self.selection_status_label(&games);
        let bar = crate::ui::glass::glass_menu_surface(26.)
            .occlude()
            .flex()
            .items_center()
            .gap(px(3.))
            .p(px(6.))
            .child(close)
            .child(count)
            .child(div().w(px(1.)).h(px(24.)).mx(px(4.)).flex_none().bg(theme::glass_border()))
            .child(self.dock_pop_button(DockPop::Status, "dock-status", Icon::Flag, status_label, &games, cx))
            .child(self.dock_pop_button(DockPop::Categories, "dock-cats", Icon::Label, tr!("Categories").into_any_element(), &games, cx))
            .child(
                dock_button(
                    "dock-fav",
                    if all_fav { Icon::Favorite } else { Icon::FavoriteBorder },
                    label(if all_fav { tr!("Unfavourite") } else { tr!("Favourite") }),
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.batch_favourite(cx))),
            )
            .child(
                dock_button(
                    "dock-hide",
                    if all_hidden { Icon::Visibility } else { Icon::VisibilityOff },
                    label(if all_hidden { tr!("Unhide") } else { tr!("Hide") }),
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.batch_hide(cx))),
            )
            .child(
                dock_button("dock-meta", Icon::CloudDownload, label(tr!("Metadata")), false)
                    .on_click(cx.listener(|this, _, _, cx| this.batch_metadata(cx))),
            )
            .child(
                dock_button("dock-edit", Icon::Edit, label(tr!("Edit")), false)
                    .child(kbd("F3"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_batch_edit(window, cx))),
            );
        let catcher = self.sel.pop.map(|_| {
            div().id("dock-catcher").absolute().inset_0().occlude().on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.sel.pop = None;
                    cx.notify();
                }),
            )
        });
        Some(
            div()
                .absolute()
                .inset_0()
                .children(catcher)
                .child(div().absolute().bottom(px(18.)).left_0().right_0().flex().justify_center().child(bar))
                .into_any_element(),
        )
    }
}
