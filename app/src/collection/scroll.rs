//! Smooth mouse-wheel scrolling of the game list.

use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::CollectionView;

/// Pixels per wheel "line": Wayland sends 3 lines per notch, Chromium
/// scrolls ~100px per notch.
const WHEEL_PX_PER_LINE: f32 = 100. / 3.;

#[derive(Default)]
pub(super) struct WheelScroll {
    pending: f32,
    ticking: bool,
    last: Option<Instant>,
}

impl CollectionView {
    /// Mouse wheel over the list. Wheel notches (line deltas) scroll
    /// ~100px like Chromium, animated over ~120ms; touchpads (pixel deltas)
    /// are applied as they come.
    fn on_wheel(&mut self, delta: ScrollDelta, window: &mut Window, cx: &mut Context<Self>) {
        match delta {
            ScrollDelta::Pixels(p) => {
                self.list_state.scroll_by(-p.y);
                cx.notify();
            }
            ScrollDelta::Lines(l) => {
                self.wheel.pending += -l.y * WHEEL_PX_PER_LINE;
                if !self.wheel.ticking {
                    self.wheel.ticking = true;
                    self.wheel.last = Some(Instant::now());
                    cx.on_next_frame(window, |this, window, cx| this.tick_wheel(window, cx));
                }
            }
        }
    }

    fn tick_wheel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = self.wheel.last.map_or(0.016, |l| now.duration_since(l).as_secs_f32()).clamp(0.001, 0.05);
        self.wheel.last = Some(now);
        // Exponential ease-out (time constant 40ms): ~95% done in 120ms.
        let k = 1. - (-dt / 0.04).exp();
        let mut step = self.wheel.pending * k;
        if self.wheel.pending.abs() < 1. {
            step = self.wheel.pending;
        }
        self.wheel.pending -= step;
        // Stop at the edges instead of accumulating pending distance.
        let before = self.list_state.scroll_px_offset_for_scrollbar().y;
        self.list_state.scroll_by(px(step));
        let max = self.list_state.max_offset_for_scrollbar().y;
        if (step < 0. && f32::from(before) >= 0.) || (step > 0. && f32::from(-before) >= f32::from(max)) {
            self.wheel.pending = 0.;
        }
        cx.notify();
        if self.wheel.pending != 0. {
            cx.on_next_frame(window, |this, window, cx| this.tick_wheel(window, cx));
        } else {
            self.wheel.ticking = false;
        }
    }

    /// Captures wheel events over the list (before the list's own handler) to
    /// scroll farther and smoothly. It has its own hitbox so dialogs and menus
    /// drawn on top keep their scrolling.
    pub(super) fn wheel_layer(&self, cx: &mut Context<Self>) -> Canvas<Hitbox> {
        let weak_wheel = cx.entity().downgrade();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && hitbox.is_hovered(window) {
                        cx.stop_propagation();
                        let delta = ev.delta;
                        weak_wheel.update(cx, |this, cx| this.on_wheel(delta, window, cx)).ok();
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }
}
