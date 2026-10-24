//! Console mode: a controller-first TV interface. It is an extra mode: the
//! desktop UI does not change. While it is on, the Collection draws the
//! console shell (`collection/console/`) and every
//! page / dialog renders its console layout (`render_console`) and takes
//! [`Pad`] presses through [`PadTarget`].
//!
//! Building blocks: [`kit`] (look), [`list`] (focusable rows with values
//! changed by ◀ ▶), [`osk`] (on-screen keyboard), [`browser`] (folder /
//! file picker), [`input`] (controller thread + keyboard mapping).

pub mod browser;
pub mod input;
pub mod kit;
pub mod layers;
pub mod list;
pub mod look;
pub mod motion;
pub mod osk;
pub mod registry;
pub mod sheet;
pub mod themes;

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether console mode is on (process-wide, like the appearance).
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

/// One controller press (or its keyboard equivalent).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Pad {
    Up,
    Down,
    Left,
    Right,
    /// Confirm (south button).
    A,
    /// Back (east button).
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    /// Menu (opens the sidebar).
    Start,
    Select,
    /// Right stick: scroll long text.
    ScrollUp,
    ScrollDown,
}

/// Button glyph in the hint bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    /// "LB RB" pair.
    Bumpers,
    LT,
    Start,
    /// "◀ ▶".
    LeftRight,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub btn: Btn,
    pub label: SharedString,
}

pub fn hint(btn: Btn, label: impl Into<SharedString>) -> Hint {
    Hint { btn, label: label.into() }
}

/// A view that takes controller presses in console mode.
pub trait PadTarget: Sized + 'static {
    /// Handles `pad`; `false` lets the shell use it (B on a top screen
    /// opens the menu, ...).
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool;
    /// Button hints for the footer.
    fn hints(&self, cx: &App) -> Vec<Hint>;
}

type PadFn = Rc<dyn Fn(Pad, &mut Window, &mut App) -> bool>;
type HintsFn = Rc<dyn Fn(&App) -> Vec<Hint>>;

/// Type-erased handle on a [`PadTarget`] entity, for hosts that keep views
/// as `AnyView` (pages, dialogs, overlays).
#[derive(Clone)]
pub struct PadHook {
    pad: PadFn,
    hints: HintsFn,
}

impl PadHook {
    pub fn new<V: PadTarget>(view: &Entity<V>) -> Self {
        let (a, b) = (view.downgrade(), view.downgrade());
        Self {
            pad: Rc::new(move |pad, window, cx| a.update(cx, |v, cx| v.on_pad(pad, window, cx)).unwrap_or(false)),
            hints: Rc::new(move |cx| b.upgrade().map(|v| v.read(cx).hints(cx)).unwrap_or_default()),
        }
    }

    pub fn pad(&self, pad: Pad, window: &mut Window, cx: &mut App) -> bool {
        (self.pad)(pad, window, cx)
    }

    pub fn hints(&self, cx: &App) -> Vec<Hint> {
        (self.hints)(cx)
    }
}

/// An overlay entity (keyboard, picker, confirm sheet) with its pad hook.
#[derive(Clone)]
pub struct Overlay {
    pub view: AnyView,
    pub hook: PadHook,
}

impl Overlay {
    pub fn new<V: PadTarget + Render>(view: &Entity<V>) -> Self {
        Self { view: view.clone().into(), hook: PadHook::new(view) }
    }
}

/// An overlay owned by a page or dialog (its own sheets, keyboard,
/// picker): drawn on top of the owner's console layout, gets the presses
/// first.
#[derive(Default)]
pub struct Layer {
    overlay: Option<Overlay>,
    sub: Option<Subscription>,
}

impl Layer {
    pub fn open<V: PadTarget + Render>(&mut self, view: &Entity<V>, sub: Subscription) {
        self.overlay = Some(Overlay::new(view));
        self.sub = Some(sub);
    }

    pub fn close(&mut self) {
        self.overlay = None;
        self.sub = None;
    }

    pub fn is_open(&self) -> bool {
        self.overlay.is_some()
    }

    /// `true` when the layer took `pad` (B on an overlay that ignores it
    /// closes the layer).
    pub fn pad(&mut self, pad: Pad, window: &mut Window, cx: &mut App) -> bool {
        let Some(o) = self.overlay.clone() else { return false };
        if !o.hook.pad(pad, window, cx) && pad == Pad::B {
            self.close();
        }
        true
    }

    pub fn hints(&self, cx: &App) -> Option<Vec<Hint>> {
        self.overlay.as_ref().map(|o| o.hook.hints(cx))
    }

    pub fn view(&self) -> Option<AnyView> {
        self.overlay.as_ref().map(|o| o.view.clone())
    }
}
