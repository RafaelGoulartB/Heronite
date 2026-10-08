//! Draggable scrollbar overlay.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme;

/// Width of the mouse target of a draggable scrollbar (the visible thumb is
/// 3px wide, like Heronite's `::-webkit-scrollbar`).
const SCROLLBAR_HIT_W: f32 = 10.;

/// Thumb height and its distance from the track top, in px.
fn thumb_geometry(offset: f32, max: f32, viewport: f32) -> Option<(f32, f32)> {
    if max <= 0.5 || viewport <= 0. {
        return None;
    }
    let thumb = (viewport * viewport / (viewport + max)).max(24.).min(viewport);
    let top = (offset / max).clamp(0., 1.) * (viewport - thumb);
    Some((thumb, top))
}

/// Scroll distance (0..=max) for a thumb whose top edge is at `thumb_top`.
fn scroll_for_thumb_top(thumb_top: f32, thumb: f32, max: f32, viewport: f32) -> f32 {
    let room = viewport - thumb;
    if room <= 0. {
        return 0.;
    }
    (thumb_top / room).clamp(0., 1.) * max
}

/// Marker for the active drag of a scrollbar (`id` tells scrollbars apart).
struct ScrollbarDrag {
    id: &'static str,
}

thread_local! {
    /// Grab offset of each scrollbar's current drag. The scrollbar element is
    /// rebuilt every frame, so what the mouse-down learned has to live here.
    static GRABS: std::cell::RefCell<std::collections::HashMap<&'static str, std::rc::Rc<std::cell::Cell<f32>>>> =
        Default::default();
}

fn grab_cell(id: &'static str) -> std::rc::Rc<std::cell::Cell<f32>> {
    GRABS.with(|g| g.borrow_mut().entry(id).or_default().clone())
}

/// The view shown while dragging: nothing.
struct NoDragView;

impl Render for NoDragView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Vertical scrollbar whose thumb can be grabbed and dragged; a click on the
/// track jumps there. `scroll_to` receives the new scroll distance in
/// px (0 = top, `max` = bottom) and must apply it and repaint. `dragging`
/// (optional) is told when a drag starts (`true`) and ends (`false`).
/// Place it over the scrolling content, inside a `relative` parent.
pub fn scrollbar_draggable(
    id: &'static str,
    offset: Pixels,
    max: Pixels,
    viewport: Pixels,
    scroll_to: impl Fn(f32, &mut Window, &mut App) + 'static,
    dragging: Option<std::rc::Rc<dyn Fn(bool)>>,
) -> Option<Stateful<Div>> {
    use std::cell::Cell;
    use std::rc::Rc;

    let (offset, max_px, viewport_px) = (f32::from(-offset), f32::from(max), f32::from(viewport));
    let (thumb, top) = thumb_geometry(offset, max_px, viewport_px)?;

    let bounds: Rc<Cell<Bounds<Pixels>>> = Rc::new(Cell::new(Bounds::default()));
    // Pointer-to-thumb-top distance while dragging.
    let grab = grab_cell(id);
    let scroll_to = Rc::new(scroll_to);

    let on_down = {
        let (bounds, grab, scroll_to, dragging) = (bounds.clone(), grab.clone(), scroll_to.clone(), dragging.clone());
        move |ev: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            let y = f32::from(ev.position.y - bounds.get().origin.y);
            let on_thumb = y >= top && y <= top + thumb;
            grab.set(if on_thumb { y - top } else { thumb / 2. });
            if let Some(d) = &dragging {
                d(true);
            }
            if !on_thumb {
                scroll_to(scroll_for_thumb_top(y - thumb / 2., thumb, max_px, viewport_px), window, cx);
            }
        }
    };
    let on_up = {
        let dragging = dragging.clone();
        move |_: &MouseUpEvent, _: &mut Window, _: &mut App| {
            if let Some(d) = &dragging {
                d(false);
            }
        }
    };
    let on_move = {
        let (bounds, grab, scroll_to, own) = (bounds.clone(), grab.clone(), scroll_to.clone(), id);
        move |ev: &DragMoveEvent<ScrollbarDrag>, window: &mut Window, cx: &mut App| {
            if ev.drag(cx).id != own {
                return;
            }
            let y = f32::from(ev.event.position.y - bounds.get().origin.y) - grab.get();
            scroll_to(scroll_for_thumb_top(y, thumb, max_px, viewport_px), window, cx);
        }
    };

    Some(
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(SCROLLBAR_HIT_W))
            .block_mouse_except_scroll()
            .on_mouse_down(MouseButton::Left, on_down)
            .on_mouse_up(MouseButton::Left, on_up.clone())
            .on_mouse_up_out(MouseButton::Left, on_up)
            .on_drag(ScrollbarDrag { id }, |_, _, _, cx| cx.new(|_| NoDragView))
            .on_drag_move::<ScrollbarDrag>(on_move)
            .child(
                canvas(
                    {
                        let bounds = bounds.clone();
                        move |b, _, _| bounds.set(b)
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top(px(top))
                    .right(px(1.))
                    .w(px(3.))
                    .h(px(thumb))
                    .rounded_full()
                    .bg(theme::white_a(0.16)),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::{scroll_for_thumb_top, thumb_geometry};

    #[test]
    fn thumb_geometry_matches_the_static_scrollbar() {
        assert_eq!(thumb_geometry(0., 0., 500.), None);
        let (thumb, top) = thumb_geometry(0., 500., 500.).unwrap();
        assert_eq!((thumb, top), (250., 0.));
        let (thumb, top) = thumb_geometry(500., 500., 500.).unwrap();
        assert_eq!((thumb, top), (250., 250.));
        // Very long content: the thumb never gets smaller than 24px.
        assert_eq!(thumb_geometry(0., 1_000_000., 500.).unwrap().0, 24.);
    }

    #[test]
    fn dragging_maps_the_thumb_back_to_a_scroll_distance() {
        let (thumb, max, viewport) = (250., 500., 500.);
        assert_eq!(scroll_for_thumb_top(0., thumb, max, viewport), 0.);
        assert_eq!(scroll_for_thumb_top(125., thumb, max, viewport), 250.);
        assert_eq!(scroll_for_thumb_top(250., thumb, max, viewport), 500.);
        // Past either end it clamps.
        assert_eq!(scroll_for_thumb_top(-40., thumb, max, viewport), 0.);
        assert_eq!(scroll_for_thumb_top(900., thumb, max, viewport), 500.);
        // Round trip.
        let (t, top) = thumb_geometry(123., max, viewport).unwrap();
        assert!((scroll_for_thumb_top(top, t, max, viewport) - 123.).abs() < 0.01);
    }
}
