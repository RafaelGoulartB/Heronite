//! Game list rows and cards, column layout and the sticky group header.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_images::Variant;

use super::vm::{CardVm, ListItem};
use super::{card, list, CollectionView};
use crate::theme;

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

    pub(super) fn cover_h(&self) -> f32 {
        theme::CARD_W * self.cover_aspect
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
                let header = list::group_header(g, false).id(SharedString::from(format!("group-{gid}"))).cursor_pointer().on_click(
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
                div().pl(px(13.6)).pr(px(16.)).child(header).into_any_element()
            }
            ListItem::Row { group, start } => {
                let cards: Vec<Arc<CardVm>> = {
                    let g = &self.groups[group];
                    g.cards[start..(start + self.columns).min(g.cards.len())].to_vec()
                };
                let last_row = start + cards.len() == self.groups[group].cards.len();
                let mut row = div()
                    .flex()
                    .pl(px(13.6))
                    .pr(px(16.))
                    .gap(px(theme::CARD_GAP_X))
                    .pt(px(if start == 0 { 7.2 } else { 0. }))
                    .pb(px(if last_row { 5.6 + 13.6 } else { theme::CARD_GAP_Y }));
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
        let cover = self.cover(&c.cover_key, theme::CARD_W, cover_h, scale, 100);
        let no_art = cover.is_none() && self.images.is_failed(&cover_id(&c.cover_key, theme::CARD_W, cover_h, scale));
        let (k1, k2, k3, k4) = (key.clone(), key.clone(), key.clone(), key.clone());
        let el = card::card(
            card::CardState {
                vm: &c,
                cover,
                no_art,
                cover_h,
                selected: self.selected.as_ref() == Some(&key),
                hovered: self.hovered.as_ref() == Some(&key),
                leaving: self.leaving.as_ref() == Some(&key),
                grey_uninstalled: self.grey_uninstalled,
                epic: self.epic_card(&key, cx),
            },
            cx.listener(move |this, _, _, cx| {
                this.select(k1.clone(), true);
                cx.notify();
            }),
            cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(k2.clone());
                } else if this.hovered.as_ref() == Some(&k2) {
                    this.hovered = None;
                    this.leaving = Some(k2.clone());
                }
                cx.notify();
            }),
            cx.listener(move |this, _, _, cx| {
                this.primary_action(&k3, cx);
                cx.notify();
            }),
        );
        div()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.open_context_menu(k4.clone(), ev.position, cx);
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
        let sticky = self.sticky().map(|(g, shift)| {
            div().absolute().top(px(shift)).left(px(13.6)).right(px(16.)).child(list::group_header(&self.groups[g], true))
        });
        div()
            .relative()
            .w(px(list_w))
            .h_full()
            .flex_none()
            .bg(linear_gradient(
                90.,
                linear_color_stop(theme::rgb_a(7, 9, 12, 0.28), 0.),
                linear_color_stop(theme::rgb_a(7, 9, 12, 0.1), 0.72),
            ))
            .child(list_el)
            .child(wheel_layer)
            .children(sticky)
            .children(list_scrollbar)
    }
}

/// Image id of a cover at a size (one texture per size).
fn cover_id(key: &SharedString, w: f32, h: f32, scale: f32) -> SharedString {
    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
    format!("c:{key}:{pw}x{ph}").into()
}
