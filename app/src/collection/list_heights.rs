//! Heights the list gives the rows it has not drawn yet.
//!
//! GPUI counts a row it has not measured as 0px, and it only measures the
//! rows on screen: the scrollbar knew a few rows, a long library looked
//! short and dragging the thumb to the bottom stopped near the top (#196).
//! Every row it has not drawn now counts as the average height of the
//! list's items, worked out from the height of one drawn row, so the total
//! (the thumb's size and how far it goes) is right from the first frame.

use gpui_kit::*;

use super::vm::{GroupVm, ListItem};
use super::{list, list_row, CollectionView};

/// Height of the padding after the last item (2rem).
pub(super) const END_H: f32 = 32.;

/// How the rows are laid out, for their heights.
#[derive(Clone, Copy)]
pub(super) struct RowLayout {
    /// The Focus list's one game per row (`focusList`).
    pub focus_rows: bool,
    pub columns: usize,
    /// Space between two rows of covers.
    pub gap_y: f32,
}

/// Padding above and below the row of a group of `len` games that starts
/// at its game `start`: the first row has a little more room above, the
/// last one more below.
pub(super) fn row_padding(l: RowLayout, start: usize, len: usize) -> (f32, f32) {
    if l.focus_rows {
        let last = start + 1 >= len;
        (if start == 0 { 2. } else { 0. }, if last { 12. } else { 2. })
    } else {
        let last = start + l.columns.max(1) >= len;
        (if start == 0 { 7.2 } else { 0. }, if last { 5.6 + 13.6 } else { l.gap_y })
    }
}

/// Height of `item` when a row's cards (or game row) are `core` tall.
fn item_height(l: RowLayout, item: ListItem, groups: &[GroupVm], core: f32) -> f32 {
    match item {
        ListItem::Header { .. } => list::GROUP_HEADER_H,
        ListItem::Row { group, start } => {
            let (top, bottom) = row_padding(l, start, groups.get(group).map_or(0, |g| g.cards.len()));
            core + top + bottom
        }
        ListItem::End => END_H,
    }
}

/// The height every item not drawn yet counts as: the average of them all.
pub(super) fn average_height(l: RowLayout, items: &[ListItem], groups: &[GroupVm], core: f32) -> f32 {
    if items.is_empty() {
        return 0.;
    }
    items.iter().map(|it| item_height(l, *it, groups, core)).sum::<f32>() / items.len() as f32
}

/// The cards' (or game row's) height in a drawn row `measured` tall.
fn core_of(l: RowLayout, item: ListItem, groups: &[GroupVm], measured: f32) -> Option<f32> {
    let ListItem::Row { group, start } = item else { return None };
    let (top, bottom) = row_padding(l, start, groups.get(group)?.cards.len());
    Some(measured - top - bottom)
}

/// What the list knows of its rows' height.
#[derive(Default)]
pub(super) struct RowHeights {
    /// Height of a row's cards (or game row) as last drawn; an estimate
    /// until a row is drawn.
    core: Option<f32>,
    /// A drawn row confirmed the height the hints were made from.
    settled: bool,
    /// Frames asked for to find a drawn row since the hints were made.
    tries: u8,
    /// The list's width when the hints were given: GPUI drops them all
    /// when the list gets another width (its first layout too).
    hinted_width: Option<f32>,
    /// The column's width, the cards' width and shape and the kind of list
    /// the hints were checked for: other cards are another height.
    checked_for: Option<(f32, f32, f32, bool)>,
}

impl CollectionView {
    fn row_layout(&self) -> RowLayout {
        RowLayout { focus_rows: self.focus_rows(), columns: self.columns, gap_y: self.list_metrics().gap_y }
    }

    /// Padding above and below the row of `group` that starts at its game `start`.
    pub(super) fn row_padding(&self, group: usize, start: usize) -> (f32, f32) {
        row_padding(self.row_layout(), start, self.groups.get(group).map_or(0, |g| g.cards.len()))
    }

    /// The height an item not drawn yet counts as (see the module).
    fn row_hint(&self) -> f32 {
        // Before a row is drawn: the cover and the lines under it.
        let estimate = || if self.focus_rows() { list_row::ROW_H } else { self.cover_h() + 44. };
        average_height(self.row_layout(), &self.items, &self.groups, self.row_heights.core.unwrap_or_else(estimate))
    }

    fn list_width(&self) -> Option<f32> {
        Some(f32::from(self.list_state.viewport_bounds().size.width)).filter(|w| *w > 0.)
    }

    /// Resets the list to its items, each one not drawn yet counting as the
    /// average height.
    pub(super) fn reset_list(&mut self) {
        let hint = self.row_hint();
        if hint > 0. {
            self.list_state.reset_with_uniform_height(self.items.len(), px(hint));
        } else {
            self.list_state.reset(self.items.len());
        }
        self.row_heights.hinted_width = self.list_width();
        self.row_heights.settled = false;
        self.row_heights.tries = 0;
    }

    /// Keeps the hints right, once per frame before the list is laid out:
    /// gives them back when the list's new width dropped them, and checks
    /// them against a row on screen; rows of another height (the first
    /// frame, other cards) reset the list with hints of the drawn height,
    /// keeping its place.
    pub(super) fn sync_row_heights(&mut self, list_w: f32, window: &mut Window) {
        let width = self.list_width();
        if width.is_some() && width != self.row_heights.hinted_width {
            self.row_heights.hinted_width = width;
            let hint = self.row_hint();
            if hint > 0. {
                // Only the rows without a height take it.
                self.list_state.clone().with_uniform_item_height(px(hint));
            }
        }
        let cards = Some((list_w, self.card_w, self.card_aspect, self.focus_rows()));
        if self.row_heights.checked_for != cards {
            // This frame's layout may drop the hints: look again on the next.
            self.row_heights.checked_for = cards;
            self.row_heights.settled = false;
            self.row_heights.tries = 0;
        }
        if self.row_heights.settled || self.list_state.is_scrollbar_dragging() {
            return;
        }
        let l = self.row_layout();
        let first = self.list_state.logical_scroll_top().item_ix;
        let drawn = (first..self.items.len()).take(12).find_map(|ix| {
            let bounds = self.list_state.bounds_for_item(ix)?;
            core_of(l, self.items[ix], &self.groups, f32::from(bounds.size.height))
        });
        match drawn {
            Some(core) if self.row_heights.core.is_none_or(|c| (c - core).abs() > 0.5) => {
                self.row_heights.core = Some(core);
                self.rebuild_items();
                window.request_animation_frame();
            }
            Some(_) if width == self.row_heights.hinted_width => self.row_heights.settled = true,
            // Not drawn yet (or just given back): look again on the next
            // frames, a few times (the list may not be on screen at all).
            _ if self.row_heights.tries < 4 && self.items.iter().any(|it| matches!(it, ListItem::Row { .. })) => {
                self.row_heights.tries += 1;
                window.request_animation_frame();
            }
            _ => self.row_heights.settled = true,
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` would stand in for the built-in one.
    use super::{average_height, core_of, item_height, list, row_padding, GroupVm, ListItem, RowLayout, END_H};

    const GRID: RowLayout = RowLayout { focus_rows: false, columns: 3, gap_y: 10. };
    const ROWS: RowLayout = RowLayout { focus_rows: true, columns: 1, gap_y: 10. };

    #[test]
    fn the_first_and_last_rows_get_their_own_padding() {
        // 7 games in rows of 3: rows start at 0, 3 and 6.
        assert_eq!(row_padding(GRID, 0, 7), (7.2, 10.));
        assert_eq!(row_padding(GRID, 3, 7), (0., 10.));
        assert_eq!(row_padding(GRID, 6, 7), (0., 5.6 + 13.6));
        // One row that is both the first and the last.
        assert_eq!(row_padding(GRID, 0, 2), (7.2, 5.6 + 13.6));
        assert_eq!(row_padding(ROWS, 0, 3), (2., 2.));
        assert_eq!(row_padding(ROWS, 1, 3), (0., 2.));
        assert_eq!(row_padding(ROWS, 2, 3), (0., 12.));
    }

    #[test]
    fn a_drawn_row_gives_back_the_cards_height() {
        let groups = test_groups(&[7]);
        let row = |start| ListItem::Row { group: 0, start };
        assert_eq!(core_of(GRID, row(0), &groups, 217.2), Some(200.));
        assert_eq!(core_of(GRID, row(3), &groups, 210.), Some(200.));
        assert_eq!(core_of(GRID, ListItem::End, &groups, 32.), None);
        assert_eq!(core_of(GRID, ListItem::Header { group: 0 }, &groups, 28.8), None);
    }

    #[test]
    fn the_hints_add_up_to_the_whole_list() {
        let groups = test_groups(&[7, 2]);
        let items = super::super::vm::build_items(&groups, GRID.columns);
        let core = 200.;
        let total: f32 = items.iter().map(|it| item_height(GRID, *it, &groups, core)).sum();
        // Two headers, rows of 7 games (first, middle, last) and of 2 (first
        // and last at once), the end padding.
        let expected =
            2. * list::GROUP_HEADER_H + (core + 7.2 + 10.) + (core + 10.) + (core + 5.6 + 13.6) + (core + 7.2 + 5.6 + 13.6) + END_H;
        assert!((total - expected).abs() < 0.01, "{total} vs {expected}");
        let avg = average_height(GRID, &items, &groups, core);
        assert!((avg * items.len() as f32 - expected).abs() < 0.01);
        assert_eq!(average_height(GRID, &[], &groups, core), 0.);
    }

    fn test_groups(lens: &[usize]) -> Vec<GroupVm> {
        lens.iter()
            .enumerate()
            .map(|(i, &n)| {
                let keys: Vec<String> = (0..n).map(|k| format!("g{i}-{k}")).collect();
                super::super::vm::test_group(&format!("g{i}"), &keys.iter().map(String::as_str).collect::<Vec<_>>(), false)
            })
            .collect()
    }
}
