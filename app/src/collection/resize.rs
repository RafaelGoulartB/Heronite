//! Wider side panels: the Focus list column and the grid's drawer grow when
//! their inner edge is dragged. The default width for the window is the
//! narrowest they get; dragging back to it (or a double click on the edge)
//! returns to the default. `ui-state.json` keeps the widths.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::grid::{drawer_width, DRAWER_INSET};
use super::CollectionView;
use crate::theme;
use crate::ui::NoDragView;

/// Mouse target over an edge, in px.
const HANDLE_W: f32 = 8.;
/// How far the target reaches into the panel; the rest lies outside, clear
/// of the scrollbar on the panel's own edge.
const HANDLE_IN: f32 = 2.;
/// Room the focus panel keeps next to a widened list.
const FOCUS_MIN_W: f32 = 560.;
/// Room the grid keeps next to a widened drawer.
const GRID_MIN_W: f32 = 320.;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Panel {
    List,
    Drawer,
}

/// Marker for the active drag of a panel edge.
struct EdgeDrag(Panel);

/// `wanted` (`None` = the default) kept between the default `base` and `max`.
fn widened(base: f32, wanted: Option<f32>, max: f32) -> f32 {
    wanted.map_or(base, |w| w.min(max).max(base))
}

impl CollectionView {
    /// Default and widest width of `panel` in a window `vw` wide.
    fn panel_range(&self, panel: Panel, vw: f32) -> (f32, f32) {
        let (base, max) = match panel {
            Panel::List => (Self::list_column_width(vw).0, vw - FOCUS_MIN_W),
            Panel::Drawer => (drawer_width(vw), vw - 2. * DRAWER_INSET - GRID_MIN_W),
        };
        (base, max.max(base))
    }

    fn wanted_width(&mut self, panel: Panel) -> &mut Option<f32> {
        match panel {
            Panel::List => &mut self.ui.list_column_w,
            Panel::Drawer => &mut self.ui.grid_drawer_w,
        }
    }

    pub(super) fn panel_width(&self, panel: Panel, vw: f32) -> f32 {
        let (base, max) = self.panel_range(panel, vw);
        let wanted = match panel {
            Panel::List => self.ui.list_column_w,
            Panel::Drawer => self.ui.grid_drawer_w,
        };
        widened(base, wanted, max)
    }

    /// Width and columns of the Focus list: a widened list fits more covers.
    pub(super) fn focus_list_layout(&self, vw: f32) -> (f32, usize) {
        let (base, cols) = Self::list_column_width(vw);
        let w = self.panel_width(Panel::List, vw);
        if w <= base {
            return (base, cols);
        }
        let m = self.list_metrics();
        let fit = ((w - m.pad_l - m.pad_r + m.gap_x) / (theme::CARD_W + m.gap_x)).floor() as usize;
        (w, cols.max(fit))
    }

    /// Window x of the edge the user drags.
    fn edge_x(&self, panel: Panel, vw: f32) -> f32 {
        match panel {
            Panel::List => self.panel_width(panel, vw),
            Panel::Drawer => vw - DRAWER_INSET - self.panel_width(panel, vw),
        }
    }

    /// Moves the edge of `panel` to window x `x`; at the default width or
    /// below the panel goes back to following the window.
    fn drag_edge(&mut self, panel: Panel, x: f32, vw: f32) {
        let w = match panel {
            Panel::List => x,
            Panel::Drawer => vw - DRAWER_INSET - x,
        };
        let (base, max) = self.panel_range(panel, vw);
        *self.wanted_width(panel) = (w > base).then(|| w.min(max));
    }

    fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resizing.take().is_some() {
            self.save_ui();
            cx.notify();
        }
    }

    /// The draggable edge of `panel`, laid over the body (absolute, full height).
    pub(super) fn resize_handle(&self, panel: Panel, vw: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = match panel {
            Panel::List => "list-resize",
            Panel::Drawer => "drawer-resize",
        };
        let edge = self.edge_x(panel, vw);
        let left = match panel {
            Panel::List => edge - HANDLE_IN,
            Panel::Drawer => edge - HANDLE_W + HANDLE_IN,
        };
        // The drawer's edge stays clear of its rounded corners.
        let (top, bottom) = match panel {
            Panel::List => (0., 0.),
            Panel::Drawer => (theme::R_DRAWER, DRAWER_INSET + theme::R_DRAWER),
        };
        div()
            .id(id)
            .absolute()
            .top(px(top))
            .bottom(px(bottom))
            .left(px(left))
            .w(px(HANDLE_W))
            .cursor_col_resize()
            .block_mouse_except_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if ev.click_count >= 2 {
                        this.resizing = None;
                        *this.wanted_width(panel) = None;
                        this.save_ui();
                        cx.notify();
                        return;
                    }
                    let vw = f32::from(window.viewport_size().width);
                    this.resizing = Some((panel, f32::from(ev.position.x) - this.edge_x(panel, vw)));
                }),
            )
            .on_drag(EdgeDrag(panel), |_, _, _, cx| cx.new(|_| NoDragView))
            .on_drag_move::<EdgeDrag>(cx.listener(move |this, ev: &DragMoveEvent<EdgeDrag>, window, cx| {
                if ev.drag(cx).0 != panel {
                    return;
                }
                let Some((_, grab)) = this.resizing else { return };
                let vw = f32::from(window.viewport_size().width);
                this.drag_edge(panel, f32::from(ev.event.position.x) - grab, vw);
                cx.notify();
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_resize(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.end_resize(cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::widened;

    #[test]
    fn default_width_is_the_narrowest() {
        assert_eq!(widened(432., None, 1000.), 432.);
        assert_eq!(widened(432., Some(300.), 1000.), 432.);
        assert_eq!(widened(432., Some(600.), 1000.), 600.);
        assert_eq!(widened(432., Some(1400.), 1000.), 1000.);
    }
}
