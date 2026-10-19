//! Grid view of the Collection: the covers across the whole window, the
//! selected game in a glass drawer on the right (`grid_drawer.rs`). The
//! header's Focus / Grid switch picks it; the choice, the cover size and the
//! titles under the covers are remembered in `ui-state.json`.

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::ui_state::GRID_COVER_W;

use super::vm::ListItem;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tool_button_base};
use heronite_i18n::tr;

const GRID_PAD: f32 = 22.;
const GRID_GAP_X: f32 = 16.;
const GRID_GAP_Y: f32 = 20.;
/// Default drawer width; narrow windows get less (see [`drawer_width`]). The
/// user can drag it wider (`resize.rs`).
const DRAWER_W: f32 = 460.;
/// Space around the drawer (and between it and the grid).
pub(super) const DRAWER_INSET: f32 = 10.;

#[derive(Default)]
pub(super) struct GridUi {
    /// The drawer was closed (Esc, its close button) while a game stays
    /// selected; selecting a game opens it again.
    pub drawer_closed: bool,
    /// The header's cover size slider, created the first time it shows.
    size: Option<Entity<SliderState>>,
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
/// `min_card` wide: the cards share the room left over, so a row always
/// reaches both edges.
pub(super) fn grid_columns(width: f32, min_card: f32) -> (usize, f32) {
    let avail = (width - 2. * GRID_PAD).max(min_card);
    let cols = (((avail + GRID_GAP_X) / (min_card + GRID_GAP_X)).floor() as usize).max(1);
    let card = ((avail - GRID_GAP_X * (cols - 1) as f32) / cols as f32).floor();
    (cols, card)
}

/// Default width of the drawer, also its narrowest.
pub(super) fn drawer_width(viewport_w: f32) -> f32 {
    DRAWER_W.min((viewport_w * 0.45).max(340.))
}

impl CollectionView {
    pub(super) fn list_metrics(&self) -> ListMetrics {
        if self.ui.grid_view {
            ListMetrics { pad_l: GRID_PAD, pad_r: GRID_PAD, gap_x: GRID_GAP_X, gap_y: GRID_GAP_Y }
        } else {
            ListMetrics { pad_l: 13.6, pad_r: 16., gap_x: theme::CARD_GAP_X, gap_y: theme::CARD_GAP_Y }
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
            self.card_w = theme::CARD_W;
            return self.focus_list_layout(viewport_w);
        }
        let w = if self.drawer_open() {
            viewport_w - self.panel_width(super::resize::Panel::Drawer, viewport_w) - 2. * DRAWER_INSET
        } else {
            viewport_w
        };
        let (cols, card) = grid_columns(w, self.ui.grid_cover_w);
        self.card_w = card;
        (w, cols)
    }

    /// Focus (list + details) or Grid.
    pub(super) fn set_grid_view(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.ui.grid_view == on {
            return;
        }
        self.ui.grid_view = on;
        self.save_ui();
        self.grid.drawer_closed = false;
        self.grid.reveal = true;
        self.focus_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// Header controls: in the grid the cover size and the titles switch,
    /// then the Focus / Grid switch.
    pub(super) fn view_mode_controls(&mut self, cx: &mut Context<Self>) -> Div {
        let grid = self.ui.grid_view;
        let mut row = div().flex().flex_none().items_center().gap(px(6.)).mr(px(6.));
        if grid {
            let slider = self.grid_size_slider(cx);
            let titles = self.ui.grid_titles;
            row = row
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon(Icon::GridView, px(14.), theme::tool_icon()))
                        .child(div().w(px(96.)).child(Slider::new(&slider).bg(theme::accent()).text_color(theme::accent()))),
                )
                .child(
                    tool_button_base("grid-titles", titles)
                        .w_auto()
                        .px(px(9.))
                        .text_size(px(13.))
                        .font_weight(theme::W_BOLD)
                        .text_color(if titles { white() } else { theme::tool_icon() })
                        .child("Aa")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ui.grid_titles = !this.ui.grid_titles;
                            this.save_ui();
                            cx.notify();
                        })),
                );
        }
        row.child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .p(px(3.))
                .rounded_full()
                .bg(theme::white_a(0.06))
                .child(
                    mode_button("view-focus", Icon::ViewSidebar, tr!("Focus"), !grid)
                        .on_click(cx.listener(|this, _, _, cx| this.set_grid_view(false, cx))),
                )
                .child(
                    mode_button("view-grid", Icon::GridView, tr!("Grid"), grid)
                        .on_click(cx.listener(|this, _, _, cx| this.set_grid_view(true, cx))),
                ),
        )
    }

    fn grid_size_slider(&mut self, cx: &mut Context<Self>) -> Entity<SliderState> {
        if let Some(s) = &self.grid.size {
            return s.clone();
        }
        let (min, max, _) = GRID_COVER_W;
        // `max` first: `min` is clamped against the default max of 100.
        let state = cx.new(|_| SliderState::new().max(max).min(min).step(2.).default_value(self.ui.grid_cover_w));
        cx.subscribe(&state, |this, _, ev: &SliderEvent, cx| {
            let (value, release) = match ev {
                SliderEvent::Change(v) => (v, false),
                SliderEvent::Release(v) => (v, true),
            };
            let SliderValue::Single(v) = *value else { return };
            this.ui.grid_cover_w = v;
            if release {
                this.save_ui();
            }
            cx.notify();
        })
        .detach();
        self.grid.size = Some(state.clone());
        state
    }

    /// Keys of the grid and the Focus list when nothing floats over them:
    /// arrows move the selection, Enter plays an installed game (in the grid
    /// it otherwise shows the drawer), Esc closes the drawer. Returns whether
    /// the key was used.
    pub(super) fn grid_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let free = self.page == crate::pages::Page::Collection
            && self.dialog.is_none()
            && self.context_menu.is_none()
            && self.open_menu.is_none()
            && !self.palette_open()
            && self.focus_handle.is_focused(window);
        if !free || ev.keystroke.modifiers.modified() {
            return false;
        }
        let step = match ev.keystroke.key.as_str() {
            "left" => Some((-1, 0)),
            "right" => Some((1, 0)),
            "up" => Some((0, -1)),
            "down" => Some((0, 1)),
            _ => None,
        };
        if let Some((dx, dy)) = step {
            if let Some(key) = self.grid_neighbour(dx, dy) {
                self.select(key, true);
                self.reveal_selected();
            }
            return true;
        }
        match ev.keystroke.key.as_str() {
            "enter" => {
                let Some(key) = self.selected.clone() else { return false };
                if self.focus.as_ref().is_some_and(|f| f.installed && !f.playing) {
                    self.install_or_play(&key, window, cx);
                } else if self.ui.grid_view {
                    self.grid.drawer_closed = false;
                }
                true
            }
            "escape" if self.drawer_open() => {
                self.grid.drawer_closed = true;
                true
            }
            _ => false,
        }
    }

    /// The card next to the selected one: `dx` walks the cards in order
    /// (wrapping to the next row), `dy` moves to the same column of the row
    /// above or below. Nothing selected: the first card.
    fn grid_neighbour(&self, dx: i32, dy: i32) -> Option<SharedString> {
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
            return (!rows.is_empty()).then(|| card(0, 0));
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
            let next = ri as i32 + dy;
            if next < 0 || next as usize >= rows.len() {
                return None;
            }
            let next = next as usize;
            (next, col.min(len(next) - 1))
        };
        Some(card(ri, col))
    }
}

/// One half of the Focus / Grid switch: white pill when active.
fn mode_button(id: &'static str, ic: Icon, label: &'static str, active: bool) -> Stateful<Div> {
    let fg: Hsla = if active { rgb(0x0b0e13).into() } else { theme::white_a(0.68) };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(11.))
        .rounded_full()
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(fg)
        .when(active, |d| d.bg(theme::white_a(0.9)))
        .when(!active, |d| d.hover(|s| s.bg(theme::white_a(0.08))))
        .child(icon(ic, px(15.), fg))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::{grid_columns, GRID_GAP_X, GRID_PAD};

    #[test]
    fn cards_fill_the_row() {
        let (cols, card) = grid_columns(1920., 150.);
        assert_eq!(cols, 11);
        assert!(card >= 150.);
        let used = card * cols as f32 + GRID_GAP_X * (cols - 1) as f32;
        assert!(1920. - 2. * GRID_PAD - used < cols as f32);
    }

    #[test]
    fn narrow_grid_keeps_one_column() {
        assert_eq!(grid_columns(100., 150.).0, 1);
    }
}
