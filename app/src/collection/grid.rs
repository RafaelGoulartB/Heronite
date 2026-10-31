//! Grid view of the Collection: the covers across the whole window, the
//! selected game in a glass drawer on the right (`grid_drawer.rs`). The
//! header's Focus / Grid switch (`view_mode.rs`) picks it; the choice, the
//! cover size and the titles under the covers are remembered in
//! `ui-state.json`, as is whether the Focus list shows covers or rows.

use gpui_kit::component::slider::SliderState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::FocusCoverSize;

use super::vm::ListItem;
use super::CollectionView;
use crate::theme;

const GRID_PAD: f32 = 22.;
const GRID_GAP_X: f32 = 16.;
const GRID_GAP_Y: f32 = 20.;
/// Default drawer width; narrow windows get less (see [`drawer_width`]). The
/// user can drag it wider, or a little narrower (`resize.rs`).
const DRAWER_W: f32 = 480.;
/// Space around the drawer (and between it and the grid).
pub(super) const DRAWER_INSET: f32 = 10.;

#[derive(Default)]
pub(super) struct GridUi {
    /// The drawer was closed (Esc, its close button) while a game stays
    /// selected; selecting a game opens it again.
    pub drawer_closed: bool,
    /// The cover size slider of the view options, created the first time it shows.
    pub size: Option<Entity<SliderState>>,
    /// The pointer went down on the active view mode while its menu was
    /// open (`view_mode.rs`).
    pub menu_press: bool,
    /// Scroll the selected card into view once the new columns are laid out.
    pub reveal: bool,
}

/// Paddings and gaps of the list rows.
#[derive(Clone, Copy)]
pub(super) struct ListMetrics {
    pub pad_l: f32,
    pub pad_r: f32,
    pub gap_x: f32,
    pub gap_y: f32,
}

/// Columns and card width of a grid `width` wide whose cards are at least
/// `min_card` wide, `gap` apart: the cards share the room left over, so a
/// row always reaches both edges.
pub(super) fn grid_columns(width: f32, min_card: f32, gap: f32) -> (usize, f32) {
    let avail = (width - 2. * GRID_PAD).max(min_card);
    let cols = (((avail + gap) / (min_card + gap)).floor() as usize).max(1);
    let card = ((avail - gap * (cols - 1) as f32) / cols as f32).floor();
    (cols, card)
}

/// Default width of the drawer.
pub(super) fn drawer_width(viewport_w: f32) -> f32 {
    DRAWER_W.min((viewport_w * 0.47).max(360.))
}

/// Narrowest the user can drag the drawer: 5% under the default, to the
/// nearest 10 px (480 → 460, 360 → 340).
pub(super) fn drawer_min_width(viewport_w: f32) -> f32 {
    (drawer_width(viewport_w) * 0.095).round() * 10.
}

impl CollectionView {
    /// Settings → Appearance → Density "Compact" halves the gaps.
    pub(super) fn list_metrics(&self) -> ListMetrics {
        let k = if theme::appearance().density == heronite_data::appearance::Density::Compact { 0.5 } else { 1. };
        if self.ui.grid_view {
            ListMetrics { pad_l: GRID_PAD, pad_r: GRID_PAD, gap_x: GRID_GAP_X * k, gap_y: GRID_GAP_Y * k }
        } else {
            ListMetrics { pad_l: 13.6, pad_r: 16., gap_x: theme::CARD_GAP_X * k, gap_y: theme::CARD_GAP_Y * k }
        }
    }

    pub(super) fn drawer_open(&self) -> bool {
        // While selecting several games the dock acts on them: the grid takes the whole width.
        self.ui.grid_view && self.focus.is_some() && !self.grid.drawer_closed && !self.sel.active()
    }

    /// Width of the list column and its columns for this window; sets the
    /// card width.
    pub(super) fn list_layout(&mut self, viewport_w: f32) -> (f32, usize) {
        if !self.ui.grid_view {
            let (w, cols) = self.focus_list_layout(viewport_w);
            let m = self.list_metrics();
            if self.ui.focus_list {
                self.card_w = w - m.pad_l - m.pad_r + 2. * super::list_view::ROW_INSET;
                return (w, cols);
            }
            let (cols, card) = self.focus_cards(w, cols);
            self.card_w = card;
            return (w, cols);
        }
        let w = if self.drawer_open() {
            viewport_w - self.panel_width(super::resize::Panel::Drawer, viewport_w) - 2. * DRAWER_INSET
        } else {
            viewport_w
        };
        let (cols, card) = grid_columns(w, self.ui.grid_cover_w * self.card_widen(), self.list_metrics().gap_x);
        self.card_w = card;
        (w, cols)
    }

    /// Columns and card width of the Focus list's covers in a column `w`
    /// wide that fits `cols` covers of the default width. The default size
    /// and shape keep `theme::CARD_W`; otherwise the covers share the row.
    fn focus_cards(&self, w: f32, cols: usize) -> (usize, f32) {
        let m = self.list_metrics();
        let room = w - m.pad_l - m.pad_r;
        let widen = self.card_widen();
        let base =
            if (widen - 1.).abs() < 0.01 { cols } else { (((room + m.gap_x) / (theme::CARD_W * widen + m.gap_x)).floor() as usize).max(1) };
        let cols = match self.ui.focus_cover_size {
            FocusCoverSize::Small => base + 1,
            FocusCoverSize::Medium => base,
            FocusCoverSize::Large => base.saturating_sub(1).max(1),
        };
        if cols == base && (widen - 1.).abs() < 0.01 {
            return (cols, theme::CARD_W);
        }
        (cols, ((room - m.gap_x * (cols - 1) as f32) / cols as f32).floor())
    }

    /// Focus (list + details) or Grid.
    pub(super) fn set_grid_view(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.show_grid_view(on, cx) {
            self.save_ui();
        }
    }

    /// [`Self::set_grid_view`] without saving it as the last view; `false`
    /// when it already is the view.
    pub(super) fn show_grid_view(&mut self, on: bool, cx: &mut Context<Self>) -> bool {
        if self.ui.grid_view == on {
            return false;
        }
        self.ui.grid_view = on;
        self.grid.drawer_closed = !self.ui.grid_open_on_select;
        self.grid.reveal = true;
        self.focus_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
        true
    }

    /// The Focus list as rows or as covers.
    pub(super) fn set_focus_list(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.ui.focus_list == on {
            return;
        }
        self.ui.focus_list = on;
        self.save_ui();
        // The rows change height even when the columns stay the same.
        self.rebuild_items();
        self.grid.reveal = true;
        cx.notify();
    }

    /// Keys of the grid and the Focus list when nothing floats over them
    /// (the moves are `keys.rs`'s): Enter does what the game's big button
    /// does, Play or Install (a running game: the grid shows its drawer),
    /// Esc closes the drawer. Returns whether the key was used.
    pub(super) fn grid_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.list_free() || !self.focus_handle.is_focused(window) || ev.keystroke.modifiers.modified() {
            return false;
        }
        match ev.keystroke.key.as_str() {
            "enter" => {
                let Some(key) = self.selected.clone() else { return false };
                // In the grid the drawer shows the game the key acts on.
                if self.ui.grid_view {
                    self.grid.drawer_closed = false;
                }
                self.enter_primary(&key, window, cx);
                true
            }
            "escape" if self.drawer_open() => {
                self.grid.drawer_closed = true;
                true
            }
            _ => false,
        }
    }

    /// Enter on a game: what its big button does, Play or Install. Never
    /// Stop, nor cancelling a launch being prepared. True when it acted.
    pub(super) fn enter_primary(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(app) = self.lib.as_ref().and_then(|l| l.get(key)).map(|g| g.app_name.clone()) else { return false };
        if self.playing.contains_key(&app) || self.preparing.contains_key(&app) {
            return false;
        }
        self.install_or_play(key, window, cx);
        true
    }

    /// The card next to the selected one: `dx` walks the cards in order
    /// (wrapping to the next row), `dy` moves to the same column of the row
    /// `dy` rows above or below (a jump of several rows stops at the first or
    /// last one). Nothing selected: the first card. The selected game in a
    /// folded group: the nearest card after or before the group.
    pub(super) fn grid_neighbour(&self, dx: i32, dy: i32) -> Option<SharedString> {
        let rows: Vec<(usize, usize)> = self
            .items
            .iter()
            .filter_map(|it| match *it {
                ListItem::Row { group, start } => Some((group, start)),
                _ => None,
            })
            .collect();
        let len = |ri: usize| {
            let (g, s) = rows[ri];
            (self.groups[g].cards.len() - s).min(self.columns)
        };
        let card = |ri: usize, col: usize| {
            let (g, s) = rows[ri];
            self.groups[g].cards[s + col].key.clone()
        };
        let here = self.selected.as_ref().and_then(|sel| {
            (0..rows.len()).find_map(|ri| {
                let (g, s) = rows[ri];
                self.groups[g].cards[s..s + len(ri)].iter().position(|c| &c.key == sel).map(|col| (ri, col))
            })
        });
        let Some((ri, col)) = here else {
            let folded = self.selected.as_ref().and_then(|sel| self.groups.iter().position(|g| g.cards.iter().any(|c| &c.key == sel)));
            return match folded {
                Some(g) if dx > 0 || dy > 0 => rows.iter().position(|r| r.0 > g).map(|ri| card(ri, 0)),
                Some(g) => rows.iter().rposition(|r| r.0 < g).map(|ri| card(ri, len(ri) - 1)),
                None => (!rows.is_empty()).then(|| card(0, 0)),
            };
        };
        let (ri, col) = if dx > 0 {
            if col + 1 < len(ri) {
                (ri, col + 1)
            } else if ri + 1 < rows.len() {
                (ri + 1, 0)
            } else {
                return None;
            }
        } else if dx < 0 {
            if col > 0 {
                (ri, col - 1)
            } else if ri > 0 {
                (ri - 1, len(ri - 1) - 1)
            } else {
                return None;
            }
        } else {
            let mut next = ri as i32 + dy;
            if dy.abs() > 1 {
                next = next.clamp(0, rows.len() as i32 - 1);
            }
            if next < 0 || next as usize >= rows.len() || next as usize == ri {
                return None;
            }
            let next = next as usize;
            (next, col.min(len(next) - 1))
        };
        Some(card(ri, col))
    }
}

#[cfg(test)]
mod tests {
    use super::{grid_columns, GRID_GAP_X, GRID_PAD};

    #[test]
    fn cards_fill_the_row() {
        let (cols, card) = grid_columns(1920., 150., GRID_GAP_X);
        assert_eq!(cols, 11);
        assert!(card >= 150.);
        let used = card * cols as f32 + GRID_GAP_X * (cols - 1) as f32;
        assert!(1920. - 2. * GRID_PAD - used < cols as f32);
    }

    #[test]
    fn narrow_grid_keeps_one_column() {
        assert_eq!(grid_columns(100., 150., GRID_GAP_X).0, 1);
    }
}
