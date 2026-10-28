//! Game list rows and cards, column layout and the sticky group header.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::CoverLabel;
use heronite_images::Variant;

use super::vm::{CardVm, ListItem};
use super::{card, list, list_row, CollectionView};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::icon;
use heronite_i18n::tr;

/// How far a Focus list row reaches past the covers' padding on each side:
/// its highlight has room around the thumbnail and the caption.
pub(super) const ROW_INSET: f32 = 4.;

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OnHover = Box<dyn Fn(&bool, &mut Window, &mut App)>;

impl CollectionView {
    pub(super) fn list_column_width(viewport_w: f32) -> (f32, usize) {
        if viewport_w <= 780. {
            (theme::LIST_COL_W_SM, 1)
        } else if viewport_w <= 1100. {
            (theme::LIST_COL_W_MD, 2)
        } else {
            (theme::LIST_COL_W, 3)
        }
    }

    /// The Focus list shows one game per row (`focusList`), not covers.
    pub(super) fn focus_rows(&self) -> bool {
        !self.ui.grid_view && self.ui.focus_list
    }

    pub(super) fn cover_h(&self) -> f32 {
        self.card_w * self.cover_aspect
    }

    /// Size the covers are requested at. The grid's cards change width with
    /// the window and the drawer: steps of 16px keep a resize from decoding
    /// every cover again for each pixel (the image scales to the card).
    fn cover_request(&self) -> (f32, f32) {
        if self.focus_rows() {
            return (list_row::THUMB_W, list_row::THUMB_W * self.cover_aspect);
        }
        if !self.ui.grid_view {
            return (self.card_w, self.cover_h());
        }
        let w = (self.card_w / 16.).ceil() * 16.;
        (w, w * self.cover_aspect)
    }
    /// Cover texture for `key` at `w`×`h` logical px (requested on demand).
    pub(super) fn cover(&mut self, key: &SharedString, w: f32, h: f32, scale: f32, priority: i32) -> Option<ImageSource> {
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id = cover_id(key, w, h, scale);
        let chains = &self.cover_chains;
        self.images
            .get_or_request(&id, || chains.get(key).cloned().unwrap_or_default(), pw, ph, Variant::Cover, priority)
            .map(ImageSource::Render)
    }

    fn render_item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.items.get(ix).copied() else {
            return div().into_any_element();
        };
        match item {
            ListItem::Header { group } => {
                let g = &self.groups[group];
                let gid = g.id.clone();
                let pick = self.group_pick(&gid, cx);
                let header = list::group_header(g).children(pick).id(SharedString::from(format!("group-{gid}"))).cursor_pointer().on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.ui.toggle_group(&gid);
                        this.save_ui();
                        if let Some(g) = this.groups.iter_mut().find(|g| g.id == gid) {
                            g.collapsed = !g.collapsed;
                        }
                        this.rebuild_items();
                        cx.notify();
                    }),
                );
                let m = self.list_metrics();
                // Pinned: the copy over the list stands in for it, otherwise
                // both show, a few pixels apart, while it scrolls under.
                let stuck = self.stuck_group == Some(group);
                div().pl(px(m.pad_l)).pr(px(m.pad_r)).when(stuck, |d| d.opacity(0.)).child(header).into_any_element()
            }
            ListItem::Row { group, start } if self.focus_rows() => {
                let c = self.groups[group].cards[start].clone();
                let last_row = start + 1 == self.groups[group].cards.len();
                let m = self.list_metrics();
                div()
                    .pl(px(m.pad_l - ROW_INSET))
                    .pr(px(m.pad_r - ROW_INSET))
                    .pt(px(if start == 0 { 2. } else { 0. }))
                    .pb(px(if last_row { 12. } else { 2. }))
                    .child(self.render_row(c, window.scale_factor(), cx))
                    .into_any_element()
            }
            ListItem::Row { group, start } => {
                let cards: Vec<Arc<CardVm>> = {
                    let g = &self.groups[group];
                    g.cards[start..(start + self.columns).min(g.cards.len())].to_vec()
                };
                let last_row = start + cards.len() == self.groups[group].cards.len();
                let m = self.list_metrics();
                let mut row = div()
                    .flex()
                    .pl(px(m.pad_l))
                    .pr(px(m.pad_r))
                    .gap(px(m.gap_x))
                    .pt(px(if start == 0 { 7.2 } else { 0. }))
                    .pb(px(if last_row { 5.6 + 13.6 } else { m.gap_y }));
                let scale = window.scale_factor();
                for c in cards {
                    row = row.child(self.render_card(c, scale, cx));
                }
                row.into_any_element()
            }
            ListItem::End => div().h(px(32.)).into_any_element(),
        }
    }

    fn render_card(&mut self, c: Arc<CardVm>, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let key = c.key.clone();
        let cover_h = self.cover_h();
        let (rw, rh) = self.cover_request();
        let cover = self.cover(&c.cover_key, rw, rh, scale, 100);
        let no_art = cover.is_none() && self.images.is_failed(&cover_id(&c.cover_key, rw, rh, scale));
        let a = theme::appearance();
        let look = card_look();
        let (on_select, on_hover, on_play) = self.card_listeners(&key, cx);
        let el = card::card(
            card::CardState {
                vm: &c,
                cover,
                no_art,
                width: self.card_w,
                cover_h,
                title: self.ui.grid_view && self.ui.grid_titles || a.cover_label == CoverLabel::Title,
                selected: self.selected.as_ref() == Some(&key),
                hovered: self.hovered.as_ref() == Some(&key),
                leaving: self.leaving.as_ref() == Some(&key),
                grey_uninstalled: self.grey_uninstalled,
                pick: self.sel.active().then(|| self.sel.contains(&key)),
                epic: self.epic_card(&key, cx),
                look,
            },
            on_select,
            on_hover,
            on_play,
        );
        self.with_context_menu(key, el, cx)
    }

    fn render_row(&mut self, c: Arc<CardVm>, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let key = c.key.clone();
        let (rw, rh) = self.cover_request();
        let cover = self.cover(&c.cover_key, rw, rh, scale, 100);
        let (on_select, on_hover, on_play) = self.card_listeners(&key, cx);
        let el = list_row::row(
            list_row::RowState {
                vm: &c,
                cover,
                width: self.card_w,
                thumb_h: rh,
                selected: self.selected.as_ref() == Some(&key),
                hovered: self.hovered.as_ref() == Some(&key),
                grey_uninstalled: self.grey_uninstalled,
                pick: self.sel.active().then(|| self.sel.contains(&key)),
                epic: self.epic_card(&key, cx),
                look: card_look(),
            },
            on_select,
            on_hover,
            on_play,
        );
        self.with_context_menu(key, el, cx)
    }

    /// What a card (or a row) of the game under `key` does when clicked,
    /// hovered and when its play button is clicked.
    fn card_listeners(&self, key: &SharedString, cx: &mut Context<Self>) -> (OnClick, OnHover, OnClick) {
        let (k1, k2, k3) = (key.clone(), key.clone(), key.clone());
        (
            Box::new(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                this.card_clicked(k1.clone(), ev);
                cx.notify();
            })),
            Box::new(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(k2.clone());
                } else if this.hovered.as_ref() == Some(&k2) {
                    this.hovered = None;
                    this.leaving = Some(k2.clone());
                }
                cx.notify();
            })),
            Box::new(cx.listener(move |this, _, _, cx| {
                this.primary_action(&k3, cx);
                cx.notify();
            })),
        )
    }

    /// `el` with the game's context menu on a right click.
    fn with_context_menu(&self, key: SharedString, el: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        div()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.open_context_menu(key.clone(), ev.position, cx);
                }),
            )
            .child(el)
            .into_any_element()
    }

    /// Group whose header should be pinned at the top of the list, and how
    /// far it is pushed up by the next group's header.
    pub(super) fn sticky(&self) -> Option<(usize, f32)> {
        let top = self.list_state.logical_scroll_top();
        let item = self.items.get(top.item_ix)?;
        let group = match *item {
            ListItem::Header { group } => {
                if top.offset_in_item <= px(0.) {
                    return None; // header itself is at the top, not stuck
                }
                group
            }
            ListItem::Row { group, .. } => group,
            ListItem::End => return None,
        };
        if !self.groups[group].show_header {
            return None;
        }
        let next_header =
            self.items[top.item_ix + 1..].iter().position(|it| matches!(it, ListItem::Header { .. })).map(|p| top.item_ix + 1 + p);
        let mut shift = 0.;
        if let Some(nh) = next_header {
            let vp = self.list_state.viewport_bounds();
            if let Some(b) = self.list_state.bounds_for_item(nh) {
                let dist = f32::from(b.origin.y - vp.origin.y);
                if dist < list::GROUP_HEADER_H {
                    shift = dist - list::GROUP_HEADER_H;
                }
            }
        }
        Some((group, shift))
    }

    /// The list column: rows, smooth-wheel layer, pinned group header and
    /// scrollbar.
    pub(super) fn render_list_column(&mut self, list_w: f32, cx: &mut Context<Self>) -> Div {
        // The list's padding lives inside each row: GPUI clips list content to
        // the list bounds, and the selection ring sits 2px outside the cover.
        let list_el = div().size_full().child(list(self.list_state.clone(), cx.processor(Self::render_item)).size_full());
        let wheel_layer = self.wheel_layer(cx);
        let list_scrollbar = crate::ui::scrollbar_draggable(
            "list-scrollbar",
            self.list_state.scroll_px_offset_for_scrollbar().y,
            self.list_state.max_offset_for_scrollbar().y,
            self.list_state.viewport_bounds().size.height,
            {
                let (state, weak) = (self.list_state.clone(), cx.entity().downgrade());
                move |distance, _, cx| {
                    state.set_offset_from_scrollbar(point(px(0.), px(-distance)));
                    weak.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            Some(Rc::new({
                let state = self.list_state.clone();
                move |started| {
                    if started {
                        state.scrollbar_drag_started();
                    } else {
                        state.scrollbar_drag_ended();
                    }
                }
            })),
        );
        // Games exist, but the search and the filters leave none.
        let no_match = self.lib.as_ref().is_some_and(|lib| !lib.games.is_empty()) && self.groups.iter().all(|g| g.cards.is_empty());
        let m = self.list_metrics();
        let grid = self.ui.grid_view;
        let rows = self.focus_rows();
        let sticky = self.sticky();
        self.stuck_group = sticky.map(|(g, _)| g);
        let sticky = sticky.map(|(g, shift)| {
            let pick = self.group_pick(&self.groups[g].id.clone(), cx);
            div()
                .absolute()
                .top(px(shift))
                .left_0()
                .right_0()
                .pl(px(m.pad_l))
                .pr(px(m.pad_r))
                .child(if rows { list::stuck_scrim_rows() } else { list::stuck_scrim(grid) })
                .child(list::group_header(&self.groups[g]).children(pick))
        });
        div()
            .relative()
            .w(px(list_w))
            .h_full()
            .flex_none()
            // A pinned header pushed up by the next one stays in the column
            // (the list already clips its rows here).
            .overflow_hidden()
            // The grid sits on the stage's own scrims.
            .when(!grid, |d| {
                d.bg(linear_gradient(
                    90.,
                    linear_color_stop(theme::stage_ink_a(0.28), 0.),
                    linear_color_stop(theme::stage_ink_a(0.1), 0.72),
                ))
            })
            .child(list_el)
            .child(wheel_layer)
            .children(sticky)
            .children(list_scrollbar)
            .when(no_match, |d| d.child(self.render_no_match(cx)))
    }

    /// While selecting: the group header's "Select group" (the whole group
    /// in, or out once all of it is in).
    fn group_pick(&self, gid: &SharedString, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (n, total) = self.sel.active().then(|| self.group_selection(gid)).flatten()?;
        let tri = if n == 0 {
            super::selection::Tri::None
        } else if n == total {
            super::selection::Tri::All
        } else {
            super::selection::Tri::Some
        };
        let gid = gid.clone();
        Some(
            div()
                .id(SharedString::from(format!("group-pick-{gid}")))
                .relative()
                .ml(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(22.))
                .pl(px(3.))
                .pr(px(9.))
                .rounded_full()
                .bg(theme::ink_a(0.05))
                .text_size(px(12.))
                .font_weight(theme::W_REGULAR)
                .text_color(theme::text_muted())
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.1)).text_color(theme::text_bright()))
                .child(super::dock::tri_mark(tri).size(px(16.)))
                .child(if n == total { tr!("Unselect group") } else { tr!("Select group") })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_group_selection(&gid);
                    cx.notify();
                })),
        )
    }

    /// The list when the search and the filters hide every game.
    fn render_no_match(&self, cx: &mut Context<Self>) -> Div {
        let filtered = self.ui.active_filters() > 0;
        let hint = if filtered { tr!("Try another search, or clear the filters.") } else { tr!("Try another name.") };
        div()
            .absolute()
            .top(px(48.))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .px(px(24.))
            .child(icon(Icon::FaSearch, px(26.), theme::text_dim()))
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(15.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(tr!("No games match")),
            )
            .child(div().text_center().text_size(px(13.)).text_color(theme::text_muted()).child(hint))
            .child(div().mt(px(6.)).child(button_sm("no-match-clear", ButtonKind::Glass, tr!("Show all games")).on_click(cx.listener(
                |this, _, window, cx| {
                    this.search.clear();
                    this.search_input.update(cx, |s, cx| s.set_value("", window, cx));
                    this.ui.clear_filters();
                    this.save_ui();
                    this.refresh_groups();
                    cx.notify();
                },
            ))))
    }
}

/// How every card looks, from Settings → Appearance → Covers.
fn card_look() -> card::CardLook {
    let a = theme::appearance();
    card::CardLook {
        radius: a.cover_corners.radius(),
        hover: a.cover_hover,
        badges: a.cover_badges,
        caption: !matches!(a.cover_label, CoverLabel::Title | CoverLabel::Nothing),
    }
}

/// Image id of a cover at a size (one texture per size).
fn cover_id(key: &SharedString, w: f32, h: f32, scale: f32) -> SharedString {
    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
    format!("c:{key}:{pw}x{ph}").into()
}
