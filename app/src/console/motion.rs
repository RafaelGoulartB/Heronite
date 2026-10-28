//! Motion for console mode: smooth scrolling that follows the focus
//! ([`Glide`]), entrances of screens / panels ([`enter`]) and the focus
//! "pop" of rows and covers ([`press_epoch`], [`lift`]).
//!
//! Everything runs only while something moves (springs and one-shot
//! animations request frames until they settle); an idle screen draws no
//! frames. `HERONITE_NO_ANIMATIONS=1` turns it all off (jumps instead).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::kit::u;
use crate::theme;

pub fn on() -> bool {
    theme::animations() && !(super::active() && super::look::motion() == heronite_data::console_theme::Motion::Off)
}

/// The console theme's "Reduced" motion: fades stay, things no longer slide.
pub fn reduced() -> bool {
    super::active() && super::look::motion() == heronite_data::console_theme::Motion::Reduced
}

/// Scrolling: critically damped, ~90% of the way in 170ms, keeps its speed
/// when the target moves again (held directions).
const SCROLL: SpringConfig = SpringConfig::new(380., 39., 1.);
/// Focus lift of covers: a touch of overshoot.
pub const LIFT: SpringConfig = SpringConfig::new(520., 36., 1.);
/// Fades between focused / unfocused states.
pub const SOFT: SpringConfig = SpringConfig::new(300., 34.6, 1.);
/// Slower and critically damped (~300 ms, no overshoot): sizes that change
/// as the focus moves along a row.
pub const GENTLE: SpringConfig = SpringConfig::new(170., 26.1, 1.);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    X,
    Y,
}

/// Where a [`Glide`] scrolls to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Goal {
    /// Child `ix` at the start of the area.
    Start(usize),
    /// The least scroll that shows child `ix` with `margin` design px of
    /// what is around it.
    Reveal { ix: usize, margin: f32 },
    /// A scroll position (px from the start).
    Offset(f32),
    /// Like `Reveal`, for a stretch of the content known by its place
    /// (`start..end`, design px from the start of the content) instead of a
    /// child index: for areas whose children are not one per item.
    Span { start: f32, end: f32, margin: f32 },
}

#[derive(Default)]
struct GlideState {
    goal: Option<Goal>,
    snap: bool,
    velocity: f32,
    last: Option<Instant>,
    /// Frames spent waiting for the target child to be laid out.
    waits: u8,
}

/// A scroll area that glides to the focused child. Put
/// [`Glide::driver`] as the **last** child of the area (it is absolute and
/// takes no room); it moves the offset during prepaint, from the child
/// bounds of the last frame.
#[derive(Clone)]
pub struct Glide {
    handle: ScrollHandle,
    axis: Axis,
    st: Rc<RefCell<GlideState>>,
}

impl Glide {
    pub fn new(axis: Axis) -> Self {
        Self { handle: ScrollHandle::new(), axis, st: Rc::default() }
    }

    pub fn handle(&self) -> &ScrollHandle {
        &self.handle
    }

    /// Glides to `goal`.
    pub fn go(&self, goal: Goal) {
        if !on() {
            return self.jump(goal);
        }
        let mut st = self.st.borrow_mut();
        st.goal = Some(goal);
        st.waits = 0;
    }

    /// Jumps to `goal`: now when the area was laid out, else on the next
    /// frame.
    pub fn jump(&self, goal: Goal) {
        let current = -self.get(self.handle.offset());
        if let Some(t) = self.target(goal, current) {
            self.set(t);
            *self.st.borrow_mut() = GlideState::default();
            return;
        }
        let mut st = self.st.borrow_mut();
        st.goal = Some(goal);
        st.snap = true;
        st.waits = 0;
        st.velocity = 0.;
    }

    /// Glides `delta` px further (right stick on long text); repeated
    /// steps add up to where the last one was heading.
    pub fn by(&self, delta: f32) {
        let from = match self.st.borrow().goal {
            Some(Goal::Offset(t)) => t,
            _ => -self.get(self.handle.offset()),
        };
        let max = self.get(self.handle.max_offset()).max(0.);
        self.go(Goal::Offset((from + delta).clamp(0., max)));
    }

    /// Back to the start at once.
    pub fn reset(&self) {
        *self.st.borrow_mut() = GlideState::default();
        self.handle.set_offset(point(px(0.), px(0.)));
    }

    fn get(&self, p: Point<Pixels>) -> f32 {
        f32::from(match self.axis {
            Axis::X => p.x,
            Axis::Y => p.y,
        })
    }

    fn set(&self, v: f32) {
        let o = self.handle.offset();
        self.handle.set_offset(match self.axis {
            Axis::X => point(px(-v), o.y),
            Axis::Y => point(o.x, px(-v)),
        });
    }

    /// Scroll position (positive) that satisfies `goal`, from the last layout.
    fn target(&self, goal: Goal, current: f32) -> Option<f32> {
        let max = self.get(self.handle.max_offset()).max(0.);
        let view = f32::from(match self.axis {
            Axis::X => self.handle.bounds().size.width,
            Axis::Y => self.handle.bounds().size.height,
        });
        let child = |ix: usize| {
            let (first, item) = (self.handle.bounds_for_item(0)?, self.handle.bounds_for_item(ix)?);
            Some(match self.axis {
                Axis::X => (f32::from(item.left() - first.left()), f32::from(item.right() - first.left())),
                Axis::Y => (f32::from(item.top() - first.top()), f32::from(item.bottom() - first.top())),
            })
        };
        let t = match goal {
            Goal::Offset(v) => return Some(v.clamp(0., max)),
            Goal::Start(ix) => child(ix)?.0,
            Goal::Reveal { ix, margin } => {
                let (start, end) = child(ix)?;
                reveal(start, end, f32::from(u(margin)), view, current)
            }
            Goal::Span { start, end, margin } => {
                // Waits for the first layout, as the child goals do.
                self.handle.bounds_for_item(0)?;
                reveal(f32::from(u(start)), f32::from(u(end)), f32::from(u(margin)), view, current)
            }
        };
        Some(t.clamp(0., max))
    }

    fn step(&self, window: &mut Window) {
        let mut st = self.st.borrow_mut();
        let Some(goal) = st.goal else { return };
        let current = -self.get(self.handle.offset());
        let Some(target) = self.target(goal, current) else {
            // Not laid out yet: try again next frame (a few times).
            st.waits += 1;
            if st.waits > 8 {
                st.goal = None;
            } else {
                window.request_animation_frame();
            }
            return;
        };
        let now = Instant::now();
        if st.snap {
            self.set(target);
            *st = GlideState::default();
            window.request_animation_frame();
            return;
        }
        let dt = st.last.map_or(1. / 60., |l| now.duration_since(l).as_secs_f32()).clamp(0.001, 0.05);
        let next = SCROLL.step(SpringState { position: current, velocity: st.velocity }, target, dt);
        if (next.position - target).abs() < 0.5 && next.velocity.abs() < 20. {
            self.set(target);
            *st = GlideState::default();
        } else {
            self.set(next.position);
            st.velocity = next.velocity;
            st.last = Some(now);
        }
        // The new offset shows on the next frame.
        window.request_animation_frame();
    }

    /// The element that runs the glide (last child of the scroll area).
    pub fn driver(&self) -> impl IntoElement {
        let me = self.clone();
        canvas(move |_, window, _| me.step(window), |_, _, _, _| {}).absolute().top_0().left_0().size_0()
    }
}

/// The least scroll that shows `start..end` with `margin` around it, in a
/// view of `view` from `current`; the start when it does not fit.
fn reveal(start: f32, end: f32, margin: f32, view: f32, current: f32) -> f32 {
    if end - start + 2. * margin >= view || start - margin < current {
        start - margin
    } else if end + margin > current + view {
        end + margin - view
    } else {
        current
    }
}

/// A number that follows its target on a spring, stepped by its owner once
/// a frame ([`Follow::step`]): one value can then drive a whole row (every
/// icon's size and place), where a spring per element could not share it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Follow {
    pub value: f32,
    velocity: f32,
    target: f32,
}

impl Follow {
    pub fn new(value: f32) -> Self {
        Self { value, velocity: 0., target: value }
    }

    /// Heads for `target` (at once without animations).
    pub fn go(&mut self, target: f32) {
        self.target = target;
        if !on() {
            self.snap(target);
        }
    }

    pub fn snap(&mut self, value: f32) {
        *self = Self::new(value);
    }

    /// Moves `dt` seconds along; `true` while it has not settled.
    pub fn step(&mut self, cfg: SpringConfig, dt: f32) -> bool {
        if self.value == self.target && self.velocity == 0. {
            return false;
        }
        let next = cfg.step(SpringState { position: self.value, velocity: self.velocity }, self.target, dt);
        if (next.position - self.target).abs() < 0.0005 && next.velocity.abs() < 0.01 {
            self.snap(self.target);
            return false;
        }
        self.value = next.position;
        self.velocity = next.velocity;
        true
    }
}

/// The time since the last frame of a set of [`Follow`]s (1/60 s on the
/// first one after a rest, at most 1/20 s so a stall never jumps).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameClock {
    last: Option<Instant>,
}

impl FrameClock {
    pub fn dt(&mut self) -> f32 {
        let now = Instant::now();
        let dt = self.last.map_or(1. / 60., |l| now.duration_since(l).as_secs_f32()).clamp(0.001, 0.05);
        self.last = Some(now);
        dt
    }

    /// Ends a frame: the next one is asked for while something `moving`.
    pub fn done(&mut self, moving: bool, window: &mut Window) {
        if moving {
            window.request_animation_frame();
        } else {
            self.last = None;
        }
    }
}

static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Called on every controller press: the focused row replays its pop, so
/// a move or a change gives feedback.
pub fn press() {
    EPOCH.fetch_add(1, Ordering::Relaxed);
}

pub fn press_epoch() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}

/// Fades `el` in while it slides `dx`, `dy` design px into place (`el`
/// must be `relative()`, or absolute with its own `left` / `top`). `id`
/// restarts it: give a new id for each new content.
pub fn enter<E: Styled + IntoElement + 'static>(id: impl Into<ElementId>, el: E, ms: u64, dx: f32, dy: f32) -> AnyElement {
    enter_from(id, el, ms, dx, dy, 0.)
}

/// [`enter`] starting at `from` opacity (content that changes often, so a
/// fast scroll never blanks it).
pub fn enter_from<E: Styled + IntoElement + 'static>(id: impl Into<ElementId>, el: E, ms: u64, dx: f32, dy: f32, from: f32) -> AnyElement {
    if !on() {
        return el.into_any_element();
    }
    let (dx, dy) = if reduced() { (px(0.), px(0.)) } else { (u(dx), u(dy)) };
    el.with_animation(id, Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()), move |el, t| {
        let mut s = el.opacity(from + (1. - from) * t);
        if dx != px(0.) {
            s = s.left(dx * (1. - t));
        }
        if dy != px(0.) {
            s = s.top(dy * (1. - t));
        }
        s
    })
    .into_any_element()
}

/// A full-window layer (`absolute().inset_0()`) fading in while it rises
/// `dy` design px (the layer keeps its size).
pub fn layer_in(id: impl Into<ElementId>, child: impl IntoElement, ms: u64, dy: f32) -> AnyElement {
    let layer = div().absolute().inset_0().child(child);
    if !on() {
        return layer.into_any_element();
    }
    let dy = if reduced() { px(0.) } else { u(dy) };
    layer
        .with_animation(id, Animation::new(Duration::from_millis(ms)).with_easing(ease_out_quint()), move |el, t| {
            let off = dy * (1. - t);
            el.opacity(t).top(off).bottom(-off)
        })
        .into_any_element()
}

/// Fade only.
pub fn fade_in<E: Styled + IntoElement + 'static>(id: impl Into<ElementId>, el: E, ms: u64) -> AnyElement {
    enter(id, el, ms, 0., 0.)
}

/// `el` built at the spring position of `on` (0 = off, 1 = on); a new
/// element starts settled. Without animations it is built at 0 or 1.
pub fn spring<E: IntoElement + 'static>(
    id: impl Into<ElementId>,
    el: E,
    cfg: SpringConfig,
    on_: bool,
    build: impl FnOnce(E, f32) -> E + 'static,
) -> AnyElement {
    if !on() {
        return build(el, if on_ { 1. } else { 0. }).into_any_element();
    }
    el.with_spring(id, SpringAnimation::new(cfg).to(on_).with_epsilon(0.004), move |el, phase| build(el, phase.0)).into_any_element()
}

/// Linear mix of two colours.
pub fn mix(a: impl Into<Rgba>, b: impl Into<Rgba>, t: f32) -> Rgba {
    let (a, b) = (a.into(), b.into());
    let t = t.clamp(0., 1.);
    Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

#[cfg(test)]
mod tests {
    use super::reveal;
    // `gpui_kit::*` above brings gpui's own `test` macro.
    use core::prelude::v1::test;

    #[test]
    fn reveal_scrolls_only_as_far_as_needed() {
        // A 100 tall item with 20 around it, in a view of 500.
        assert_eq!(reveal(300., 400., 20., 500., 0.), 0., "in sight: stays");
        assert_eq!(reveal(600., 700., 20., 500., 0.), 220., "below: its end at the bottom edge");
        assert_eq!(reveal(300., 400., 20., 500., 400.), 280., "above: its start at the top edge");
        // Too big for the view: its start.
        assert_eq!(reveal(300., 800., 20., 500., 0.), 280.);
    }
}
