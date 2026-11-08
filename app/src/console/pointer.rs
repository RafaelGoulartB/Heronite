//! The mouse in console mode and the in-game overlay: it drives them with the
//! same [`Pad`] presses a controller makes, so every screen keeps one way of
//! handling input.
//!
//! - A click on something that takes the focus sends the presses that bring
//!   the focus there (`route`), then A when the click is for activating it
//!   (rows of a list) or when it already had the focus (covers: the first
//!   click shows the game, the second opens it).
//! - A click on a button hint sends that button ([`kit::hint_bar`](super::kit::hint_bar)).
//! - The wheel moves the focus (one press per notch), the right and back
//!   buttons go back (B). [`layer`] catches them for the whole window, before
//!   the scroll areas, which follow the focus.
//!
//! Each window that takes presses registers where they go with
//! [`set_router`]; elements send them through [`press`] (deferred, so a
//! handler never runs inside an update of the view that drew it). Layers over
//! a screen (dialogs, menus, keyboards) occlude what is under them, so a click
//! never reaches a screen that does not have the focus.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::*;

use super::Pad;

type Router = Rc<dyn Fn(Pad, &mut Window, &mut App)>;

thread_local! {
    static ROUTERS: RefCell<HashMap<WindowId, Router>> = RefCell::new(HashMap::new());
    static WHEEL: Cell<Wheel> = const { Cell::new(Wheel { sum: 0., last: None }) };
}

/// Presses from the mouse in `window` go to `route` from now on (the
/// routes of windows that closed are dropped).
pub fn set_router(window: &Window, cx: &App, route: impl Fn(Pad, &mut Window, &mut App) + 'static) {
    let id = window.window_handle().window_id();
    let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
    ROUTERS.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|w, _| open.contains(w));
        r.insert(id, Rc::new(route));
    });
}

/// `el` with a click that tells where along its width it landed, from 0 at
/// the left edge to 1 at the right one (a volume bar).
pub fn on_click_at(el: Div, f: impl Fn(f32, &mut Window, &mut App) + 'static) -> Div {
    let bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let seen = bounds.clone();
    el.child(canvas(move |b, _, _| seen.set(Some(b)), |_, _, _, _| {}).absolute().inset_0()).on_mouse_down(
        MouseButton::Left,
        move |ev, window, cx| {
            let Some(b) = bounds.get().filter(|b| b.size.width > px(0.)) else { return };
            cx.stop_propagation();
            let k = f32::from(ev.position.x - b.origin.x) / f32::from(b.size.width);
            f(k.clamp(0., 1.), window, cx);
        },
    )
}

/// The mouse no longer presses anything in `window`.
pub fn clear_router(window: &Window) {
    let id = window.window_handle().window_id();
    ROUTERS.with(|r| r.borrow_mut().remove(&id));
}

fn router(window: &Window) -> Option<Router> {
    let id = window.window_handle().window_id();
    ROUTERS.with(|r| r.borrow().get(&id).cloned())
}

/// Sends `pads` to the window's router, in order, once the current event is
/// over.
pub fn press(pads: impl Into<Rc<[Pad]>>, window: &mut Window, cx: &mut App) {
    let pads = pads.into();
    if pads.is_empty() {
        return;
    }
    let Some(route) = router(window) else { return };
    window.defer(cx, move |window, cx| {
        for &pad in pads.iter() {
            route(pad, window, cx);
        }
    });
}

/// `n` presses of `pad`.
pub fn repeat(pad: Pad, n: usize) -> impl Iterator<Item = Pad> {
    std::iter::repeat_n(pad, n)
}

/// The presses that move a focus along a line from `from` to `to`: `back`
/// toward the start, `fwd` toward the end.
pub fn along(from: usize, to: usize, back: Pad, fwd: Pad) -> Vec<Pad> {
    if to >= from {
        repeat(fwd, to - from).collect()
    } else {
        repeat(back, from - to).collect()
    }
}

/// The presses for a click on an item `steps` away from the focus: the
/// steps, or A when it has the focus already (the first click shows it, the
/// second opens it).
pub fn focus_or_activate(steps: Vec<Pad>) -> Vec<Pad> {
    if steps.is_empty() {
        vec![Pad::A]
    } else {
        steps
    }
}

/// Elements a click (the left button) can press through.
pub trait Clickable: InteractiveElement + Sized {
    /// A left click sends `pads` (nothing when empty).
    fn on_pads(self, pads: impl Into<Rc<[Pad]>>) -> Self {
        let pads: Rc<[Pad]> = pads.into();
        if pads.is_empty() {
            return self;
        }
        self.on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            press(pads.clone(), window, cx);
        })
    }

    /// Takes the clicks so nothing under it sees them (a panel over a screen).
    fn eats_clicks(self) -> Self {
        self.occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}

impl<E: InteractiveElement> Clickable for E {}

/// Which presses the wheel makes: ↑ ↓ for lists, ← → for a row of covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WheelAxis {
    Vertical,
    Horizontal,
    /// Up and down scroll long text (the right stick's presses): a game page.
    Text,
}

#[derive(Clone, Copy)]
struct Wheel {
    /// Lines turned and not pressed yet (positive = up / left).
    sum: f32,
    last: Option<Instant>,
}

/// A pause that starts a new turn of the wheel (what is left of the last one
/// is dropped).
const WHEEL_PAUSE: Duration = Duration::from_millis(300);
/// Pixels of a touchpad's scroll that count as a line.
const PX_PER_LINE: f32 = 50.;
/// Presses one event may make (a fast flick of a touchpad).
const MAX_PRESSES: usize = 4;

impl Wheel {
    /// Adds `lines` (positive = up / left) and returns the whole lines to
    /// press, keeping the rest.
    fn turn(&mut self, lines: f32, now: Instant) -> i32 {
        let fresh = self.last.is_none_or(|t| now.duration_since(t) > WHEEL_PAUSE);
        // A new turn, or the other way: start from zero.
        if fresh || self.sum.signum() != lines.signum() {
            self.sum = 0.;
        }
        self.last = Some(now);
        self.sum += lines;
        let whole = self.sum.trunc();
        self.sum -= whole;
        (whole as i32).clamp(-(MAX_PRESSES as i32), MAX_PRESSES as i32)
    }
}

/// The presses for a turn of the wheel: `delta` in lines (pixels already
/// divided), positive y = up and positive x = left, as GPUI reports them.
fn wheel_presses(dx: f32, dy: f32, axis: WheelAxis, now: Instant) -> Vec<Pad> {
    // The stronger axis wins; a sideways wheel (or a touchpad) moves sideways.
    let (lines, sideways) = if dx.abs() > dy.abs() { (dx, true) } else { (dy, false) };
    if lines == 0. {
        return Vec::new();
    }
    let n = WHEEL.with(|w| {
        let mut wheel = w.get();
        let n = wheel.turn(lines, now);
        w.set(wheel);
        n
    });
    let (back, fwd) = match axis {
        _ if sideways => (Pad::Left, Pad::Right),
        WheelAxis::Horizontal => (Pad::Left, Pad::Right),
        WheelAxis::Text => (Pad::ScrollUp, Pad::ScrollDown),
        WheelAxis::Vertical => (Pad::Up, Pad::Down),
    };
    repeat(if n > 0 { back } else { fwd }, n.unsigned_abs() as usize).collect()
}

/// Over the whole window: turns the wheel into presses along `axis` and the
/// right / back mouse buttons into B, before anything under the pointer sees
/// them (scroll areas follow the focus instead of the wheel). Draw it as the
/// first child of the window's root.
pub fn layer(axis: WheelAxis) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || router(window).is_none() {
                    return;
                }
                cx.stop_propagation();
                let (dx, dy) = match ev.delta {
                    ScrollDelta::Lines(l) => (l.x, l.y),
                    ScrollDelta::Pixels(p) => (f32::from(p.x) / PX_PER_LINE, f32::from(p.y) / PX_PER_LINE),
                };
                press(wheel_presses(dx, dy, axis, Instant::now()), window, cx);
            });
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
                let back = matches!(ev.button, MouseButton::Right | MouseButton::Navigate(NavigationDirection::Back));
                if phase != DispatchPhase::Capture || !back || router(window).is_none() {
                    return;
                }
                cx.stop_propagation();
                press([Pad::B], window, cx);
            });
        },
    )
    .absolute()
    .size_0()
}

/// A step of `HERONITE_DEBUG_MOUSE`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// A left click at a point, in fractions of the window (0..=1).
    Click(f32, f32),
    /// The wheel turned by lines (positive = up) at the window's middle.
    Wheel(f32),
    /// The right button (back).
    Right,
}

/// `HERONITE_DEBUG_MOUSE`: a scripted sequence of mouse events for tests,
/// e.g. `click:0.5:0.4,wait:800,wheel:-1,right` (points as fractions of the
/// window). Each step waits 400 ms; `wait:<ms>` waits longer.
pub fn parse_script(text: &str) -> Vec<(Duration, Step)> {
    let mut out = Vec::new();
    let mut extra = Duration::ZERO;
    for step in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = step.split(':').collect();
        let num = |i: usize| parts.get(i).and_then(|n| n.parse::<f32>().ok());
        let step = match parts[0] {
            "wait" => {
                extra += Duration::from_millis(num(1).unwrap_or(0.) as u64);
                continue;
            }
            "click" => match (num(1), num(2)) {
                (Some(x), Some(y)) => Step::Click(x, y),
                _ => continue,
            },
            "wheel" => Step::Wheel(num(1).unwrap_or(-1.)),
            "right" => Step::Right,
            other => {
                log::warn!("HERONITE_DEBUG_MOUSE: unknown step {other:?}");
                continue;
            }
        };
        out.push((Duration::from_millis(400) + extra, step));
        extra = Duration::ZERO;
    }
    out
}

/// Plays `step` in `window` as the system would send it.
pub fn play(step: Step, window: &mut Window, cx: &mut App) {
    let size = window.viewport_size();
    let at = |x: f32, y: f32| point(size.width * x, size.height * y);
    let (position, button) = match step {
        Step::Click(x, y) => (at(x, y), MouseButton::Left),
        Step::Right => (at(0.5, 0.5), MouseButton::Right),
        Step::Wheel(lines) => {
            let ev = ScrollWheelEvent {
                position: at(0.5, 0.5),
                delta: ScrollDelta::Lines(point(0., lines)),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            };
            window.dispatch_event(PlatformInput::ScrollWheel(ev), cx);
            return;
        }
    };
    let modifiers = Modifiers::default();
    window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers }), cx);
    let down = MouseDownEvent { button, position, modifiers, click_count: 1, first_mouse: false };
    window.dispatch_event(PlatformInput::MouseDown(down), cx);
    window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count: 1 }), cx);
}

#[cfg(test)]
mod tests {
    use super::{along, focus_or_activate, parse_script, Step, Wheel, WHEEL_PAUSE};
    use crate::console::Pad;
    use std::time::{Duration, Instant};

    #[test]
    fn routes_step_toward_the_target() {
        assert_eq!(along(2, 5, Pad::Left, Pad::Right), [Pad::Right; 3]);
        assert_eq!(along(4, 1, Pad::Up, Pad::Down), [Pad::Up; 3]);
        assert!(along(3, 3, Pad::Up, Pad::Down).is_empty());
        assert_eq!(focus_or_activate(Vec::new()), [Pad::A]);
        assert_eq!(focus_or_activate(vec![Pad::Down]), [Pad::Down]);
    }

    #[test]
    fn script_steps() {
        let s = parse_script("click:0.5:0.25, wait:600,wheel:-2,right,bogus,click:1");
        assert_eq!(s.iter().map(|(_, p)| *p).collect::<Vec<_>>(), [Step::Click(0.5, 0.25), Step::Wheel(-2.), Step::Right]);
        assert_eq!(s[1].0, Duration::from_millis(1000));
    }

    #[test]
    fn the_wheel_presses_once_per_line_and_keeps_the_rest() {
        let t0 = Instant::now();
        let mut w = Wheel { sum: 0., last: None };
        assert_eq!(w.turn(1., t0), 1);
        assert_eq!(w.turn(0.4, t0), 0);
        assert_eq!(w.turn(0.7, t0), 1);
        // The other way drops what was left.
        assert_eq!(w.turn(-0.5, t0), 0);
        assert_eq!(w.turn(-0.6, t0), -1);
        // A pause starts again from zero.
        assert_eq!(w.turn(-0.5, t0), 0);
        assert_eq!(w.turn(-0.6, t0 + WHEEL_PAUSE + Duration::from_millis(1)), 0);
        // A flick presses a few times at most.
        assert_eq!(w.turn(30., t0), 4);
    }
}
