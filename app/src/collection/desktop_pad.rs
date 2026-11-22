//! The controller on the desktop (outside console mode). The desktop keeps
//! its look: the controller types the keys it already takes and moves a
//! pointer, so every screen keeps one way of handling input.
//!
//! - D-pad: the arrows (with repeats). A: Enter. B: Esc. Y: the palette
//!   (Ctrl K), which reaches every page, command and game. X: the selected
//!   game's actions in the palette. LB / RB: the page before / after (the
//!   ☰ menu's order), or the palette's filters while it is open; nothing
//!   while LB + RB is the button held to enter console mode.
//! - The right stick moves a pointer drawn over the window, RT clicks with
//!   it and LT right-clicks (a game's menu): what the keys cannot reach
//!   (dialog buttons, switches) is a click away. The pointer hides when the
//!   mouse moves or a button that types a key is pressed.
//! - The left stick scrolls what is under the pointer (or the mouse, else
//!   the middle of the window).
//! - Guide, Start and Back are left to the shortcut to console mode and the
//!   in-game overlay (`console::summon`).
//!
//! Nothing runs per frame while the sticks rest: the reader blocks between
//! events (`console::input::start_desktop`), and the pointer and scrolling
//! move on a timer only while a stick is tilted. On Windows the reader also
//! stops while the window is in the background (SDL reads open controllers
//! every 8 ms), and everywhere while a game runs. Presses only count while
//! the window is in front, no game runs and "Use the controller on the
//! desktop" is on.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use futures::future::{self, Either};
use futures::StreamExt;
use gpui_kit::*;

use super::CollectionView;
use crate::console::input::{self, PadMsg, PadThread};
use crate::console::{look, pointer, Pad};
use crate::pages::{self, Page};

/// How often the pointer and scrolling move while a stick is tilted.
const TICK: Duration = Duration::from_millis(16);
/// The longest step a late tick moves by (a hitch does not jump).
const MAX_STEP: Duration = Duration::from_millis(50);
/// Pointer speed at full tilt, px/s (a 1920 px window in about 1.4 s).
const POINTER_SPEED: f32 = 1400.;
/// Scrolling at full tilt, px/s.
const SCROLL_SPEED: f32 = 2400.;
/// The drawn pointer's diameter, px.
const POINTER_D: f32 = 18.;

/// What the reader, its pump and the drawn pointer share.
#[derive(Default)]
struct Shared {
    /// Where each stick points (left, right), as [`PadMsg::Stick`] gives it.
    sticks: Cell<[(f32, f32); 2]>,
    /// The pointer's place in the window, while it is drawn.
    pointer: Cell<Option<Point<Pixels>>>,
    /// Where the pointer was last put by this module (its own moves are not
    /// the mouse's).
    placed: Cell<Option<Point<Pixels>>>,
    /// Where the system last said the mouse is: a move to the same place
    /// (the window coming to the front under it) is not the mouse moving.
    mouse: Cell<Option<Point<Pixels>>>,
}

impl Shared {
    fn tilted(&self) -> bool {
        self.sticks.get().iter().any(|&(x, y)| x != 0. || y != 0.)
    }

    fn rest(&self) {
        self.sticks.set([(0., 0.); 2]);
    }
}

#[derive(Default)]
pub(super) struct DesktopPad {
    reader: Option<(PadThread, Task<()>)>,
    /// The reader could not start: not tried again until it is wanted anew.
    failed: bool,
    shared: Rc<Shared>,
    /// Draws again when the window comes to the front or goes behind (the
    /// reader follows it on Windows).
    activation: Option<Subscription>,
}

/// The step of a stick's tilt (0..=1) into a speed: slow and precise near
/// the middle, fast at full tilt.
fn curve(v: f32) -> f32 {
    v * v.abs()
}

/// How far the pointer moves in `dt` for a stick at `x`, `y` (positive `y`
/// up; the window's `y` grows down).
fn pointer_step(x: f32, y: f32, dt: f32) -> (f32, f32) {
    (curve(x) * POINTER_SPEED * dt, -curve(y) * POINTER_SPEED * dt)
}

/// The wheel's delta in `dt` for a stick at `x`, `y`: up scrolls up (GPUI's
/// positive `y`), right scrolls right (GPUI's negative `x`).
fn scroll_step(x: f32, y: f32, dt: f32) -> (f32, f32) {
    (-curve(x) * SCROLL_SPEED * dt, curve(y) * SCROLL_SPEED * dt)
}

/// The key a press types, if it types one.
fn key_of(pad: Pad) -> Option<&'static str> {
    Some(match pad {
        Pad::Up => "up",
        Pad::Down => "down",
        Pad::Left => "left",
        Pad::Right => "right",
        Pad::A => "enter",
        Pad::B => "escape",
        Pad::Y => "ctrl-k",
        _ => return None,
    })
}

/// The pages LB and RB step through: the ☰ menu's, in its order (the stores
/// are one stop), then Settings.
fn nav_pages(stores: Page) -> Vec<Page> {
    [Page::Collection, Page::Downloads, Page::Statistics, stores, Page::Emulators, Page::WineManager, Page::Settings]
        .into_iter()
        .filter(|p| p.available())
        .collect()
}

/// The page `dir` steps from `current` to (no wrap), if any.
fn step_page(list: &[Page], current: Page, dir: isize) -> Option<Page> {
    let at = list.iter().position(|&p| p == current || (pages::stores::is_store_page(p) && pages::stores::is_store_page(current)))?;
    let to = at.checked_add_signed(dir)?;
    list.get(to).copied()
}

impl CollectionView {
    /// Starts or stops the reader as "Use the controller on the desktop",
    /// console mode and (on Windows) the window's focus ask. Every frame:
    /// cheap when nothing changes.
    pub(super) fn sync_desktop_pad(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.desktop_pad.activation.is_none() {
            // Here too, not only on the next frame: a minimized window draws none.
            self.desktop_pad.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
                this.sync_desktop_pad(window, cx);
                cx.notify();
            }));
        }
        // Not while a game runs: its sticks would wake the app all along.
        let want = look::controls().desktop
            && !crate::console::active()
            && self.runtime.running_apps().is_empty()
            && (!cfg!(windows) || window.is_window_active());
        let pad = &mut self.desktop_pad;
        if !want {
            pad.failed = false;
            if pad.reader.take().is_some() {
                pad.shared.rest();
                pad.shared.pointer.set(None);
            }
            return;
        }
        if pad.reader.is_some() || pad.failed {
            return;
        }
        let Some((thread, rx)) = input::start_desktop() else {
            pad.failed = true;
            return;
        };
        // The mouse is where it is: the window coming to the front over it is no move.
        self.desktop_pad.shared.mouse.set(Some(window.mouse_position()));
        let task = spawn_pump(rx, self.desktop_pad.shared.clone(), window, cx);
        self.desktop_pad.reader = Some((thread, task));
    }

    /// The controller may act now: on the desktop, its setting on, the
    /// window in front and no game running.
    fn desktop_pad_live(&self, window: &Window) -> bool {
        self.desktop_pad.reader.is_some()
            && look::controls().desktop
            && !crate::console::active()
            && window.is_window_active()
            && self.runtime.running_apps().is_empty()
    }

    /// The selected game's actions in the palette (X).
    fn open_game_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Collection || self.palette_open() {
            return;
        }
        if let Some(key) = self.selected.clone() {
            self.open_palette_on_game(&key, window, cx);
        }
    }

    /// LB / RB: the page before or after this one.
    fn step_desktop_page(&mut self, dir: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open(cx) {
            return;
        }
        if let Some(to) = step_page(&nav_pages(self.pages.stores_page()), self.page, dir) {
            self.navigate(to, window, cx);
        }
    }

    /// The pointer drawn over the window, and the watch that hides it when
    /// the mouse moves. Last child of the window's root.
    pub(super) fn render_desktop_pointer(&self) -> Option<AnyElement> {
        self.desktop_pad.reader.as_ref()?;
        let shared = self.desktop_pad.shared.clone();
        let el = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let watch = shared.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, _| {
                    if phase != DispatchPhase::Capture || watch.placed.get() == Some(ev.position) {
                        return;
                    }
                    // The mouse moved (not this module's own move, nor the
                    // window coming to the front under a still mouse): it takes over.
                    let moved = watch.mouse.replace(Some(ev.position)).is_some_and(|m| m != ev.position);
                    if moved && watch.pointer.take().is_some() {
                        window.refresh();
                    }
                });
                if let Some(at) = shared.pointer.get() {
                    paint_pointer(at, window);
                }
            },
        )
        .absolute()
        .size_0();
        // Over menus and popovers too.
        Some(deferred(el).with_priority(1000).into_any_element())
    }
}

/// A white disc with a dark ring, centred on `at` (window coordinates).
fn paint_pointer(at: Point<Pixels>, window: &mut Window) {
    let d = px(POINTER_D);
    let bounds = Bounds::new(point(at.x - d / 2., at.y - d / 2.), size(d, d));
    window.paint_quad(quad(bounds, d / 2., hsla(0., 0., 1., 0.92), px(2.), hsla(0., 0., 0., 0.65), BorderStyle::Solid));
    let dot = px(4.);
    let inner = Bounds::new(point(at.x - dot / 2., at.y - dot / 2.), size(dot, dot));
    window.paint_quad(quad(inner, dot / 2., hsla(0., 0., 0., 0.7), px(0.), transparent_black(), BorderStyle::Solid));
}

/// Reads the messages; while a stick is tilted, also wakes every [`TICK`]
/// to move the pointer or scroll.
fn spawn_pump(mut rx: UnboundedReceiver<PadMsg>, shared: Rc<Shared>, window: &mut Window, cx: &mut Context<CollectionView>) -> Task<()> {
    cx.spawn_in(window, async move |this, cx| {
        let mut last = Instant::now();
        loop {
            let was_tilted = shared.tilted();
            let msg = if was_tilted {
                let timer = cx.background_executor().timer(TICK);
                match future::select(rx.next(), timer).await {
                    Either::Left((msg, _)) => Some(msg),
                    Either::Right(_) => None,
                }
            } else {
                Some(rx.next().await)
            };
            let ok = match msg {
                // The reader ended.
                Some(None) => break,
                Some(Some(msg)) => on_msg(&this, &shared, msg, cx),
                None => true,
            };
            if !ok {
                break;
            }
            let now = Instant::now();
            if !was_tilted {
                // Tilted just now: the first step comes a tick later.
                last = now;
            } else if shared.tilted() && now.duration_since(last) >= TICK {
                let dt = now.duration_since(last).min(MAX_STEP).as_secs_f32();
                last = now;
                if !glide(&this, &shared, dt, cx) {
                    break;
                }
            }
        }
    })
}

/// One message. `false` once the view is gone.
fn on_msg(this: &WeakEntity<CollectionView>, shared: &Rc<Shared>, msg: PadMsg, cx: &mut AsyncWindowContext) -> bool {
    match msg {
        PadMsg::Stick { right, x, y } => {
            let mut sticks = shared.sticks.get();
            sticks[usize::from(right)] = (x, y);
            shared.sticks.set(sticks);
            true
        }
        PadMsg::Press(pad) => press(this, shared, pad, cx),
        PadMsg::Connected(name, _) => {
            // The confirm button follows the controller in use (Nintendo's A is east).
            look::set_controller(Some(&name));
            true
        }
        PadMsg::Disconnected { remaining } => {
            look::set_controller(remaining.as_deref());
            shared.rest();
            true
        }
        PadMsg::Button(..) => true,
    }
}

/// Whether the controller may act now; `None` once the view is gone.
fn live(this: &WeakEntity<CollectionView>, cx: &mut AsyncWindowContext) -> Option<bool> {
    let this = this.clone();
    cx.update(|window, cx| this.upgrade().map(|v| v.read(cx).desktop_pad_live(window))).ok().flatten()
}

fn press(this: &WeakEntity<CollectionView>, shared: &Rc<Shared>, pad: Pad, cx: &mut AsyncWindowContext) -> bool {
    let Some(live) = live(this, cx) else { return false };
    if !live {
        return true;
    }
    if let Some(key) = key_of(pad) {
        // Stepping with the keys: the pointer steps aside, and so does the mouse's.
        shared.pointer.set(None);
        pointer::hide_cursor();
        // Outside the view's update: the keys reach it as typed ones do.
        let ks = Keystroke::parse(key).expect("a valid keystroke");
        return cx.update(|window, cx| window.dispatch_keystroke(ks, cx)).is_ok();
    }
    match pad {
        // Held together they enter console mode: theirs then.
        Pad::LB | Pad::RB if cx.update(|_, cx| crate::console::summon::holds_bumpers(cx)).unwrap_or(false) => true,
        Pad::X | Pad::LB | Pad::RB => {
            shared.pointer.set(None);
            pointer::hide_cursor();
            this.update_in(cx, |v, window, cx| match pad {
                Pad::X => v.open_game_actions(window, cx),
                // The palette's filters, else the pages.
                _ if v.palette_open() => {
                    let ks = Keystroke::parse(if pad == Pad::LB { "shift-tab" } else { "tab" }).expect("a valid keystroke");
                    window.defer(cx, move |window, cx| {
                        window.dispatch_keystroke(ks, cx);
                    });
                }
                _ => v.step_desktop_page(if pad == Pad::LB { -1 } else { 1 }, window, cx),
            })
            .is_ok()
        }
        Pad::LT | Pad::RT => cx.update(|window, cx| click(shared, pad == Pad::RT, window, cx)).is_ok(),
        _ => true,
    }
}

/// Where the pointer starts when it shows: where the mouse was, if it was
/// in the window, else the middle.
fn pointer_origin(window: &Window) -> Point<Pixels> {
    let size = window.viewport_size();
    let at = window.mouse_position();
    let inside = at.x > px(0.) && at.y > px(0.) && at.x < size.width && at.y < size.height;
    if inside {
        at
    } else {
        point(size.width / 2., size.height / 2.)
    }
}

/// Puts the pointer at `at` and tells the window, as the mouse moving there would.
fn place(shared: &Shared, at: Point<Pixels>, window: &mut Window, cx: &mut App) {
    shared.pointer.set(Some(at));
    shared.placed.set(Some(at));
    pointer::hide_cursor();
    let ev = MouseMoveEvent { position: at, pressed_button: None, modifiers: Modifiers::default() };
    window.dispatch_event(PlatformInput::MouseMove(ev), cx);
    window.refresh();
}

/// RT (left) or LT (right): a click at the pointer. A hidden pointer shows
/// first, without a click, so nothing is clicked unseen.
fn click(shared: &Shared, left: bool, window: &mut Window, cx: &mut App) {
    let Some(at) = shared.pointer.get() else {
        place(shared, pointer_origin(window), window, cx);
        return;
    };
    let button = if left { MouseButton::Left } else { MouseButton::Right };
    let modifiers = Modifiers::default();
    let down = MouseDownEvent { button, position: at, modifiers, click_count: 1, first_mouse: false };
    window.dispatch_event(PlatformInput::MouseDown(down), cx);
    window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button, position: at, modifiers, click_count: 1 }), cx);
}

/// A tick of tilted sticks: the right one moves the pointer, the left one
/// scrolls. `false` once the view is gone.
fn glide(this: &WeakEntity<CollectionView>, shared: &Rc<Shared>, dt: f32, cx: &mut AsyncWindowContext) -> bool {
    let Some(live) = live(this, cx) else { return false };
    if !live {
        shared.rest();
        return true;
    }
    let [(lx, ly), (rx, ry)] = shared.sticks.get();
    cx.update(|window, cx| {
        if rx != 0. || ry != 0. {
            let from = shared.pointer.get().unwrap_or_else(|| pointer_origin(window));
            let (dx, dy) = pointer_step(rx, ry, dt);
            let size = window.viewport_size();
            let to = point((from.x + px(dx)).clamp(px(0.), size.width - px(1.)), (from.y + px(dy)).clamp(px(0.), size.height - px(1.)));
            // Pushed against an edge it stays: nothing to draw again.
            if shared.pointer.get() != Some(to) {
                place(shared, to, window, cx);
            }
        }
        if lx != 0. || ly != 0. {
            let (dx, dy) = scroll_step(lx, ly, dt);
            let position = shared.pointer.get().unwrap_or_else(|| pointer_origin(window));
            let ev = ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            };
            window.dispatch_event(PlatformInput::ScrollWheel(ev), cx);
        }
    })
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{key_of, pointer_step, scroll_step, step_page, POINTER_SPEED};
    use crate::console::Pad;
    use crate::pages::Page;
    use gpui_kit::Keystroke;

    #[test]
    fn the_sticks_are_slow_near_the_middle_and_fast_at_full_tilt() {
        let (x, y) = pointer_step(1., 0., 1.);
        assert_eq!((x, y), (POINTER_SPEED, 0.));
        let (slow, _) = pointer_step(0.25, 0., 1.);
        assert!(slow < POINTER_SPEED / 10., "{slow}");
        // Up on the stick is up on the screen; scrolling up is GPUI's positive y.
        assert!(pointer_step(0., 1., 0.1).1 < 0.);
        assert!(scroll_step(0., 1., 0.1).1 > 0.);
        assert!(scroll_step(1., 0., 0.1).0 < 0.);
        assert_eq!(pointer_step(-0.5, 0., 1.).0, -pointer_step(0.5, 0., 1.).0);
    }

    #[test]
    fn keys_for_the_presses_that_type_one() {
        assert_eq!(key_of(Pad::A), Some("enter"));
        assert_eq!(key_of(Pad::B), Some("escape"));
        assert_eq!(key_of(Pad::Y), Some("ctrl-k"));
        assert_eq!(key_of(Pad::Down), Some("down"));
        // Left to console mode's shortcut and the overlay.
        assert_eq!(key_of(Pad::Start), None);
        assert_eq!(key_of(Pad::Select), None);
        for pad in [Pad::Up, Pad::Down, Pad::Left, Pad::Right, Pad::A, Pad::B, Pad::Y] {
            assert!(Keystroke::parse(key_of(pad).unwrap()).is_ok(), "{pad:?}");
        }
    }

    #[test]
    fn the_bumpers_step_through_the_pages_without_wrapping() {
        let list = [Page::Collection, Page::Downloads, Page::Statistics, Page::Steam, Page::Settings];
        assert_eq!(step_page(&list, Page::Collection, 1), Some(Page::Downloads));
        assert_eq!(step_page(&list, Page::Collection, -1), None);
        assert_eq!(step_page(&list, Page::Settings, 1), None);
        // Any store page is the stores' stop.
        assert_eq!(step_page(&list, Page::Gog, 1), Some(Page::Settings));
        assert_eq!(step_page(&list, Page::Gog, -1), Some(Page::Statistics));
        // A page off the list (the Wine Manager where it is missing) steps nowhere.
        assert_eq!(step_page(&list, Page::WineManager, 1), None);
    }
}
