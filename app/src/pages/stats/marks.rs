//! Charts made of hundreds of cells (the calendar, the hour grid, the bars
//! of each day) painted as one element. As divs, every cell cost layout and a
//! hover listener on every frame, and the page renders again on each scroll
//! and mouse move. The cell under the cursor is found from the rectangles
//! laid out for the paint.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{Hover, StatsPage};

/// One cell: a rectangle in px from the element's top-left corner.
#[derive(Clone)]
pub(super) struct Mark {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Hsla,
    radius: f32,
    /// A white ring around it (the cell under the cursor).
    ring: bool,
    hover: Option<Hover>,
}

impl Mark {
    pub fn new(x: f32, y: f32, w: f32, h: f32, color: impl Into<Hsla>) -> Self {
        Self { x, y, w, h, color: color.into(), radius: 0., ring: false, hover: None }
    }

    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }

    pub fn ring(mut self, on: bool) -> Self {
        self.ring = on;
        self
    }

    /// What hovering the cell shows (the tooltip of [`super::tip`]).
    pub fn hover(mut self, h: Option<Hover>) -> Self {
        self.hover = h;
        self
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What the first cell with a [`Hover`] under `(x, y)` shows.
fn hit(marks: &[Mark], x: f32, y: f32) -> Option<Hover> {
    marks.iter().find(|m| m.hover.is_some() && m.contains(x, y)).and_then(|m| m.hover)
}

/// The cells `layout(width, height)` places, painted in the canvas' bounds
/// (no hover: decoration).
pub(super) fn painted(layout: impl FnOnce(f32, f32) -> Vec<Mark> + 'static) -> Canvas<Vec<Mark>> {
    canvas(move |bounds, _, _| layout(f32::from(bounds.size.width), f32::from(bounds.size.height)), paint)
}

fn paint(bounds: Bounds<Pixels>, marks: Vec<Mark>, window: &mut Window, _: &mut App) {
    for m in &marks {
        let b = Bounds { origin: bounds.origin + point(px(m.x), px(m.y)), size: size(px(m.w), px(m.h)) };
        window.paint_quad(fill(b, m.color).corner_radii(px(m.radius)));
        if m.ring {
            window.paint_quad(quad(b, px(m.radius), transparent_black(), px(2.), gpui_kit::white(), BorderStyle::default()));
        }
    }
}

/// The cells `layout(width, height)` places, painted over the element (give
/// the returned div its size). Hovering a cell that has a [`Hover`] sets the
/// page's hover; leaving the chart clears it.
pub(super) fn marks(
    id: impl Into<ElementId>,
    layout: impl FnOnce(f32, f32) -> Vec<Mark> + 'static,
    cx: &mut Context<StatsPage>,
) -> Stateful<Div> {
    let laid: Rc<RefCell<(Point<Pixels>, Vec<Mark>)>> = Rc::default();
    let (set, on_move, on_leave) = (laid.clone(), laid.clone(), laid);
    let page = cx.entity().downgrade();
    let leave_page = page.clone();
    let chart = canvas(
        move |bounds, _, _| {
            let marks = layout(f32::from(bounds.size.width), f32::from(bounds.size.height));
            *set.borrow_mut() = (bounds.origin, marks.clone());
            marks
        },
        paint,
    )
    .absolute()
    .inset_0();
    div()
        .id(id)
        .relative()
        .child(chart)
        .on_mouse_move(move |ev: &MouseMoveEvent, _, cx| {
            let hit = {
                let (origin, marks) = &*on_move.borrow();
                let p = ev.position - *origin;
                hit(marks, f32::from(p.x), f32::from(p.y))
            };
            page.update(cx, |this, cx| {
                if this.hover != hit {
                    this.hover = hit;
                    cx.notify();
                }
            })
            .ok();
        })
        .on_hover(move |on: &bool, _, cx| {
            if *on {
                return;
            }
            let mine: Vec<Hover> = on_leave.borrow().1.iter().filter_map(|m| m.hover).collect();
            leave_page
                .update(cx, |this, cx| {
                    if this.hover.is_some_and(|h| mine.contains(&h)) {
                        this.hover = None;
                        cx.notify();
                    }
                })
                .ok();
        })
}

#[cfg(test)]
mod tests {
    use super::{hit, Mark};
    use crate::pages::stats::Hover;
    use gpui_kit::transparent_black;

    #[test]
    fn the_cell_under_the_cursor_answers() {
        let cell = |x: f32, h: Option<Hover>| Mark::new(x, 0., 10., 10., transparent_black()).hover(h);
        // A bar without hover over a column that has one (the day bars).
        let marks = [cell(0., Some(Hover::Bar(0))), cell(0., None), cell(12., Some(Hover::Bar(1)))];
        assert_eq!(hit(&marks, 5., 5.), Some(Hover::Bar(0)));
        assert_eq!(hit(&marks, 15., 9.9), Some(Hover::Bar(1)));
        // The gap between cells and outside them answer nothing.
        assert_eq!(hit(&marks, 11., 5.), None);
        assert_eq!(hit(&marks, 5., 10.), None);
    }
}
