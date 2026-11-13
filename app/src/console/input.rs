//! Controller input for console mode: a thread turns controller events into
//! [`Pad`] presses (with key-repeat for held directions and the sticks) and
//! hands them to the UI over a channel. It only runs while something listens
//! (console mode, or the prompt to enter it).
//!
//! Where the events come from depends on the system: `gilrs` (evdev) on
//! Linux, SDL on Windows (`heronite_platform::controllers`), where
//! Windows.Gaming.Input missed PlayStation and Nintendo controllers. Both
//! report a [`RawEvent`]; the [`Translator`] makes the presses the same way
//! for both.
//!
//! The keyboard drives console mode too ([`pad_from_key`]), so it works
//! without a controller.
//!
//! [`start_buttons`] reads the buttons as they go down and up instead (a
//! button held, for the shortcut to console mode: `console::summon`).

use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::Keystroke;

use super::Pad;

#[cfg(not(windows))]
#[path = "input/gilrs_pads.rs"]
mod backend;
#[cfg(windows)]
#[path = "input/sdl_pads.rs"]
mod backend;

/// What the thread sends.
#[derive(Clone, Debug, PartialEq)]
pub enum PadMsg {
    Press(Pad),
    /// A button went down (`true`) or up, from [`start_buttons`] only.
    Button(PadButton, bool),
    /// A controller is present: its name, and whether it was just plugged in
    /// (`false` for the ones found at start).
    Connected(String, bool),
    /// A controller went away. `remaining` is the name of another one that is
    /// still connected (`None` when it was the last).
    Disconnected {
        remaining: Option<String>,
    },
}

/// First repeat after this long, then every [`REPEAT_EVERY`].
const REPEAT_AFTER: Duration = Duration::from_millis(380);
const REPEAT_EVERY: Duration = Duration::from_millis(85);
/// Right stick scrolling repeats faster.
const SCROLL_EVERY: Duration = Duration::from_millis(60);
/// Stick deflection that counts as a press, and the one that releases it.
const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;
/// Trigger travel that counts as a press, and the one that releases it (what
/// gilrs uses to turn analog triggers into buttons).
const TRIGGER_ON: f32 = 0.75;
const TRIGGER_OFF: f32 = 0.65;

/// A controller button by its position, whatever reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    LB,
    RB,
    /// A trigger that reports a button (gilrs; SDL's analog triggers are
    /// [`RawEvent::Trigger`]).
    #[cfg_attr(windows, allow(dead_code))]
    LT,
    #[cfg_attr(windows, allow(dead_code))]
    RT,
    Start,
    /// Xbox, PS or Home: opens the menu like Start.
    Guide,
    Select,
    Up,
    Down,
    Left,
    Right,
}

/// Stick and hat axes; positive Y points up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StickAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    /// A hat d-pad not mapped to buttons: moves like the left stick (gilrs;
    /// SDL always reports d-pad buttons).
    #[cfg_attr(windows, allow(dead_code))]
    DPadX,
    #[cfg_attr(windows, allow(dead_code))]
    DPadY,
}

/// What a backend reads from the controllers.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RawEvent {
    Button(PadButton, bool),
    Axis(StickAxis, f32),
    /// An analog trigger, 0..=1 (SDL; gilrs reports trigger buttons).
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    Trigger {
        right: bool,
        value: f32,
    },
    /// `plugged`: just connected (`false` for the ones found at start).
    Connected {
        name: String,
        plugged: bool,
    },
    Disconnected {
        remaining: Option<String>,
    },
}

/// Turns held directions into repeated presses. Pure, so it is tested
/// without a controller.
#[derive(Default)]
pub struct Repeater {
    /// The held direction and when it fires next.
    held: Option<(Pad, Instant)>,
    /// Where each stick points now (`Some` while past the threshold).
    left: Option<Pad>,
    right: Option<Pad>,
}

impl Repeater {
    /// A direction went down: fires at once, then repeats.
    pub fn press(&mut self, pad: Pad, now: Instant) -> Pad {
        self.held = Some((pad, now + REPEAT_AFTER));
        pad
    }

    pub fn release(&mut self, pad: Pad) {
        if self.held.is_some_and(|(p, _)| p == pad) {
            self.held = None;
        }
    }

    /// Forgets the held direction and both sticks, so nothing keeps repeating
    /// after a controller disappears without sending its release events.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Stick moved: maps the deflection to a direction (with hysteresis)
    /// and returns a press when a new direction starts.
    pub fn stick(&mut self, right_stick: bool, x: f32, y: f32, now: Instant) -> Option<Pad> {
        let current = if right_stick { self.right } else { self.left };
        let threshold = |p: Option<Pad>, wanted: Pad| if p == Some(wanted) { STICK_OFF } else { STICK_ON };
        let dir = if right_stick {
            // Only vertical: scrolling.
            if y >= threshold(current, Pad::ScrollUp) {
                Some(Pad::ScrollUp)
            } else if y <= -threshold(current, Pad::ScrollDown) {
                Some(Pad::ScrollDown)
            } else {
                None
            }
        } else if x.abs() > y.abs() {
            if x >= threshold(current, Pad::Right) {
                Some(Pad::Right)
            } else if x <= -threshold(current, Pad::Left) {
                Some(Pad::Left)
            } else {
                None
            }
        } else if y >= threshold(current, Pad::Up) {
            Some(Pad::Up)
        } else if y <= -threshold(current, Pad::Down) {
            Some(Pad::Down)
        } else {
            None
        };
        if dir == current {
            return None;
        }
        if let Some(old) = current {
            self.release(old);
        }
        if right_stick {
            self.right = dir;
        } else {
            self.left = dir;
        }
        dir.map(|d| self.press(d, now))
    }

    /// How long the thread may sleep before [`Repeater::tick`] has work.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.held.map(|(_, at)| at.saturating_duration_since(now))
    }

    /// The repeat due at `now`, if any.
    pub fn tick(&mut self, now: Instant) -> Option<Pad> {
        let (pad, at) = self.held?;
        if now < at {
            return None;
        }
        let every = if matches!(pad, Pad::ScrollUp | Pad::ScrollDown) { SCROLL_EVERY } else { REPEAT_EVERY };
        self.held = Some((pad, now + every));
        Some(pad)
    }
}

/// The press a controller button makes. With `swap_ab` the east button
/// confirms and the south one goes back (`controls.swapAb` of the console
/// theme).
fn button_pad(b: PadButton, swap_ab: bool) -> Option<Pad> {
    use PadButton as B;
    Some(match b {
        B::South if swap_ab => Pad::B,
        B::East if swap_ab => Pad::A,
        B::South => Pad::A,
        B::East => Pad::B,
        B::West => Pad::X,
        B::North => Pad::Y,
        B::LB => Pad::LB,
        B::RB => Pad::RB,
        B::LT => Pad::LT,
        B::RT => Pad::RT,
        B::Start | B::Guide => Pad::Start,
        B::Select => Pad::Select,
        B::Up => Pad::Up,
        B::Down => Pad::Down,
        B::Left => Pad::Left,
        B::Right => Pad::Right,
    })
}

/// The theme in use, read per press so a change applies at once.
fn swap_ab() -> bool {
    super::look::theme().controls.swap_ab
}

fn is_direction(p: Pad) -> bool {
    matches!(p, Pad::Up | Pad::Down | Pad::Left | Pad::Right)
}

/// Turns a backend's [`RawEvent`]s into [`PadMsg`]s: repeats for held
/// directions, stick and trigger thresholds. Pure, so it is tested without a
/// controller.
#[derive(Default)]
pub(super) struct Translator {
    /// Sends the buttons as they are ([`PadMsg::Button`]), nothing else.
    buttons: bool,
    rep: Repeater,
    /// Sticks report X and Y separately; keep both.
    lx: f32,
    ly: f32,
    rx: f32,
    ry: f32,
    /// Analog triggers held past [`TRIGGER_ON`] (left, right).
    triggers: [bool; 2],
}

impl Translator {
    /// For [`start_buttons`].
    pub fn buttons() -> Self {
        Self { buttons: true, ..Self::default() }
    }

    /// Made by [`Translator::buttons`]: nothing repeats, a slower read does.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn buttons_only(&self) -> bool {
        self.buttons
    }

    pub fn event(&mut self, ev: RawEvent, now: Instant, swap_ab: bool) -> Option<PadMsg> {
        if self.buttons {
            return match ev {
                RawEvent::Button(b, down) => Some(PadMsg::Button(b, down)),
                RawEvent::Connected { name, plugged } => Some(PadMsg::Connected(name, plugged)),
                RawEvent::Disconnected { remaining } => Some(PadMsg::Disconnected { remaining }),
                RawEvent::Axis(..) | RawEvent::Trigger { .. } => None,
            };
        }
        match ev {
            RawEvent::Button(b, true) => {
                button_pad(b, swap_ab).map(|p| PadMsg::Press(if is_direction(p) { self.rep.press(p, now) } else { p }))
            }
            RawEvent::Button(b, false) => {
                if let Some(p) = button_pad(b, swap_ab) {
                    self.rep.release(p);
                }
                None
            }
            RawEvent::Axis(axis, v) => {
                match axis {
                    StickAxis::LeftX | StickAxis::DPadX => self.lx = v,
                    StickAxis::LeftY | StickAxis::DPadY => self.ly = v,
                    StickAxis::RightX => self.rx = v,
                    StickAxis::RightY => self.ry = v,
                }
                let right = matches!(axis, StickAxis::RightX | StickAxis::RightY);
                let (x, y) = if right { (self.rx, self.ry) } else { (self.lx, self.ly) };
                self.rep.stick(right, x, y, now).map(PadMsg::Press)
            }
            RawEvent::Trigger { right, value } => {
                let held = &mut self.triggers[usize::from(right)];
                if !*held && value >= TRIGGER_ON {
                    *held = true;
                    Some(PadMsg::Press(if right { Pad::RT } else { Pad::LT }))
                } else {
                    if *held && value <= TRIGGER_OFF {
                        *held = false;
                    }
                    None
                }
            }
            RawEvent::Connected { name, plugged } => Some(PadMsg::Connected(name, plugged)),
            RawEvent::Disconnected { remaining } => {
                // The release events never arrive; the state is not per pad, so
                // it is all dropped.
                self.rep.reset();
                (self.lx, self.ly, self.rx, self.ry) = (0.0, 0.0, 0.0, 0.0);
                self.triggers = [false; 2];
                Some(PadMsg::Disconnected { remaining })
            }
        }
    }

    /// How long the thread may sleep before [`Translator::tick`] has work.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.rep.wait(now)
    }

    /// The repeat due at `now`, if any.
    pub fn tick(&mut self, now: Instant) -> Option<PadMsg> {
        self.rep.tick(now).map(PadMsg::Press)
    }
}

/// Reading controllers for one listener; dropping it stops (the receiver
/// then ends).
pub struct PadThread {
    _source: backend::Source,
}

/// Starts reading controllers. `None` when the reader cannot start or
/// `HERONITE_NO_GAMEPAD` is set (test copies on a hidden workspace must not
/// follow the controller of the app in use).
pub fn start() -> Option<(PadThread, UnboundedReceiver<PadMsg>)> {
    if crate::env::flag("HERONITE_NO_GAMEPAD") {
        return None;
    }
    let (source, rx) = backend::start(Translator::default())?;
    Some((PadThread { _source: source }, rx))
}

/// [`start`] for the buttons as they go down and up ([`PadMsg::Button`]),
/// without repeats, sticks or triggers. Reads with the app in the
/// background too.
pub fn start_buttons() -> Option<(PadThread, UnboundedReceiver<PadMsg>)> {
    if crate::env::flag("HERONITE_NO_GAMEPAD") {
        return None;
    }
    let (source, rx) = backend::start(Translator::buttons())?;
    Some((PadThread { _source: source }, rx))
}

/// Keyboard equivalents in console mode: arrows, Enter/Space (A), Escape /
/// Backspace (B), X, Y, Q / E (LB / RB), Page Up / Down (scroll), M or F1
/// (Start, the menu), Tab (Select).
pub fn pad_from_key(ks: &Keystroke) -> Option<Pad> {
    let m = &ks.modifiers;
    if m.control || m.alt || m.platform {
        return None;
    }
    Some(match ks.key.as_str() {
        "up" => Pad::Up,
        "down" => Pad::Down,
        "left" => Pad::Left,
        "right" => Pad::Right,
        "enter" | "space" => Pad::A,
        "escape" | "backspace" => Pad::B,
        "x" => Pad::X,
        "y" => Pad::Y,
        "q" => Pad::LB,
        "e" => Pad::RB,
        "pageup" => Pad::ScrollUp,
        "pagedown" => Pad::ScrollDown,
        "m" | "f1" => Pad::Start,
        "tab" => Pad::Select,
        _ => return None,
    })
}

/// `HERONITE_DEBUG_PAD`: a scripted sequence of presses for tests, e.g.
/// `down,down,a,wait:1500,b`. Each step waits 350 ms; `wait:<ms>` waits
/// longer.
pub fn parse_script(text: &str) -> Vec<(Duration, Pad)> {
    let mut out = Vec::new();
    let mut extra = Duration::ZERO;
    for step in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(ms) = step.strip_prefix("wait:").and_then(|n| n.parse::<u64>().ok()) {
            extra += Duration::from_millis(ms);
            continue;
        }
        let pad = match step.to_ascii_lowercase().as_str() {
            "up" => Pad::Up,
            "down" => Pad::Down,
            "left" => Pad::Left,
            "right" => Pad::Right,
            "a" => Pad::A,
            "b" => Pad::B,
            "x" => Pad::X,
            "y" => Pad::Y,
            "lb" => Pad::LB,
            "rb" => Pad::RB,
            "lt" => Pad::LT,
            "rt" => Pad::RT,
            "start" => Pad::Start,
            "select" => Pad::Select,
            "sup" => Pad::ScrollUp,
            "sdown" => Pad::ScrollDown,
            other => {
                log::warn!("HERONITE_DEBUG_PAD: unknown step {other:?}");
                continue;
            }
        };
        out.push((Duration::from_millis(350) + extra, pad));
        extra = Duration::ZERO;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        button_pad, parse_script, PadButton as Button, PadMsg, RawEvent, Repeater, StickAxis, Translator, REPEAT_AFTER, REPEAT_EVERY,
    };
    use crate::console::Pad;
    use std::time::{Duration, Instant};

    #[test]
    fn held_direction_repeats_after_a_delay() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        assert_eq!(r.press(Pad::Down, t0), Pad::Down);
        assert_eq!(r.tick(t0 + Duration::from_millis(100)), None);
        assert_eq!(r.tick(t0 + REPEAT_AFTER), Some(Pad::Down));
        assert_eq!(r.tick(t0 + REPEAT_AFTER + Duration::from_millis(10)), None);
        assert_eq!(r.tick(t0 + REPEAT_AFTER + REPEAT_EVERY), Some(Pad::Down));
        r.release(Pad::Down);
        assert_eq!(r.tick(t0 + Duration::from_secs(5)), None);
        assert_eq!(r.wait(t0), None);
    }

    #[test]
    fn reset_stops_repeats_and_releases_the_sticks() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        r.press(Pad::Left, t0);
        r.reset();
        assert_eq!(r.wait(t0), None);
        assert_eq!(r.tick(t0 + Duration::from_secs(5)), None);

        // A stick held past the threshold presses again after a reset.
        assert_eq!(r.stick(false, 0.0, 0.9, t0), Some(Pad::Up));
        r.reset();
        assert_eq!(r.tick(t0 + Duration::from_secs(5)), None);
        assert_eq!(r.stick(false, 0.0, 0.9, t0), Some(Pad::Up));
        assert_eq!(r.stick(true, 0.0, 0.9, t0), Some(Pad::ScrollUp));
        r.reset();
        assert_eq!(r.stick(true, 0.0, 0.9, t0), Some(Pad::ScrollUp));
    }

    #[test]
    fn stick_has_hysteresis_and_one_press_per_direction() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        assert_eq!(r.stick(false, 0.0, 0.4, t0), None);
        assert_eq!(r.stick(false, 0.0, 0.7, t0), Some(Pad::Up));
        // Still up while above the release threshold, no new press.
        assert_eq!(r.stick(false, 0.1, 0.4, t0), None);
        assert_eq!(r.stick(false, 0.0, 0.2, t0), None);
        assert_eq!(r.tick(t0 + Duration::from_secs(1)), None);
        assert_eq!(r.stick(false, 0.9, 0.1, t0), Some(Pad::Right));
        assert_eq!(r.stick(false, -0.9, 0.1, t0), Some(Pad::Left));
        assert_eq!(r.stick(true, 0.0, -0.8, t0), Some(Pad::ScrollDown));
    }

    #[test]
    fn south_confirms_and_east_goes_back_unless_swapped() {
        assert_eq!(button_pad(Button::South, false), Some(Pad::A));
        assert_eq!(button_pad(Button::East, false), Some(Pad::B));
        assert_eq!(button_pad(Button::South, true), Some(Pad::B));
        assert_eq!(button_pad(Button::East, true), Some(Pad::A));
        // Everything else ignores the swap.
        assert_eq!(button_pad(Button::West, true), Some(Pad::X));
        assert_eq!(button_pad(Button::North, true), Some(Pad::Y));
        assert_eq!(button_pad(Button::Up, true), Some(Pad::Up));
        // Xbox, PS and Home open the menu like Start.
        assert_eq!(button_pad(Button::Guide, false), Some(Pad::Start));
        assert_eq!(button_pad(Button::Select, false), Some(Pad::Select));
    }

    fn press(p: Pad) -> Option<PadMsg> {
        Some(PadMsg::Press(p))
    }

    #[test]
    fn the_buttons_translator_sends_each_button_down_and_up_only() {
        let t0 = Instant::now();
        let mut t = Translator::buttons();
        assert_eq!(t.event(RawEvent::Button(Button::Guide, true), t0, true), Some(PadMsg::Button(Button::Guide, true)));
        assert_eq!(t.event(RawEvent::Button(Button::Down, true), t0, false), Some(PadMsg::Button(Button::Down, true)));
        assert_eq!(t.wait(t0), None, "no repeats");
        assert_eq!(t.event(RawEvent::Button(Button::Guide, false), t0, false), Some(PadMsg::Button(Button::Guide, false)));
        assert_eq!(t.event(RawEvent::Axis(StickAxis::LeftY, 0.9), t0, false), None, "no sticks");
        assert_eq!(t.event(RawEvent::Trigger { right: true, value: 1.0 }, t0, false), None);
        let plugged = RawEvent::Connected { name: "Pad".into(), plugged: true };
        assert_eq!(t.event(plugged, t0, false), Some(PadMsg::Connected("Pad".into(), true)));
        assert!(t.buttons_only() && !Translator::default().buttons_only());
    }

    #[test]
    fn translator_presses_buttons_and_repeats_directions() {
        let t0 = Instant::now();
        let mut t = Translator::default();
        assert_eq!(t.event(RawEvent::Button(Button::South, true), t0, false), press(Pad::A));
        assert_eq!(t.event(RawEvent::Button(Button::South, false), t0, false), None);
        assert_eq!(t.event(RawEvent::Button(Button::East, true), t0, true), press(Pad::A));
        assert_eq!(t.wait(t0), None);
        assert_eq!(t.event(RawEvent::Button(Button::Down, true), t0, false), press(Pad::Down));
        assert_eq!(t.wait(t0), Some(REPEAT_AFTER));
        assert_eq!(t.tick(t0 + REPEAT_AFTER), press(Pad::Down));
        assert_eq!(t.event(RawEvent::Button(Button::Down, false), t0, false), None);
        assert_eq!(t.tick(t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn translator_sticks_keep_both_axes_and_the_hat_moves_like_the_left_stick() {
        let t0 = Instant::now();
        let mut t = Translator::default();
        assert_eq!(t.event(RawEvent::Axis(StickAxis::LeftX, 0.2), t0, false), None);
        assert_eq!(t.event(RawEvent::Axis(StickAxis::LeftY, 0.9), t0, false), press(Pad::Up));
        // X alone changes but Y still dominates: no new press.
        assert_eq!(t.event(RawEvent::Axis(StickAxis::LeftX, 0.3), t0, false), None);
        assert_eq!(t.event(RawEvent::Axis(StickAxis::LeftY, 0.0), t0, false), None);
        assert_eq!(t.event(RawEvent::Axis(StickAxis::DPadX, -1.0), t0, false), press(Pad::Left));
        assert_eq!(t.event(RawEvent::Axis(StickAxis::DPadX, 0.0), t0, false), None);
        assert_eq!(t.event(RawEvent::Axis(StickAxis::RightY, -0.8), t0, false), press(Pad::ScrollDown));
        // The right stick's X does not scroll.
        assert_eq!(t.event(RawEvent::Axis(StickAxis::RightY, 0.0), t0, false), None);
        assert_eq!(t.event(RawEvent::Axis(StickAxis::RightX, 1.0), t0, false), None);
    }

    #[test]
    fn translator_triggers_press_once_with_hysteresis() {
        let t0 = Instant::now();
        let mut t = Translator::default();
        let lt = |value| RawEvent::Trigger { right: false, value };
        assert_eq!(t.event(lt(0.5), t0, false), None);
        assert_eq!(t.event(lt(0.8), t0, false), press(Pad::LT));
        assert_eq!(t.event(lt(1.0), t0, false), None);
        // Above the release point: still held.
        assert_eq!(t.event(lt(0.7), t0, false), None);
        assert_eq!(t.event(lt(0.9), t0, false), None);
        assert_eq!(t.event(lt(0.1), t0, false), None);
        assert_eq!(t.event(lt(0.8), t0, false), press(Pad::LT));
        assert_eq!(t.event(RawEvent::Trigger { right: true, value: 1.0 }, t0, false), press(Pad::RT));
        // Triggers never repeat.
        assert_eq!(t.wait(t0), None);
    }

    #[test]
    fn translator_disconnect_drops_held_state() {
        let t0 = Instant::now();
        let mut t = Translator::default();
        assert_eq!(
            t.event(RawEvent::Connected { name: "Pad".into(), plugged: true }, t0, false),
            Some(PadMsg::Connected("Pad".into(), true))
        );
        t.event(RawEvent::Button(Button::Right, true), t0, false);
        t.event(RawEvent::Axis(StickAxis::LeftY, 0.9), t0, false);
        t.event(RawEvent::Trigger { right: false, value: 1.0 }, t0, false);
        assert_eq!(t.event(RawEvent::Disconnected { remaining: None }, t0, false), Some(PadMsg::Disconnected { remaining: None }));
        assert_eq!(t.wait(t0), None);
        assert_eq!(t.tick(t0 + Duration::from_secs(5)), None);
        // Held past the thresholds again: new presses.
        assert_eq!(t.event(RawEvent::Axis(StickAxis::LeftY, 0.9), t0, false), press(Pad::Up));
        assert_eq!(t.event(RawEvent::Trigger { right: false, value: 1.0 }, t0, false), press(Pad::LT));
    }

    #[test]
    fn script_steps() {
        let s = parse_script("down, a,wait:1000,b,bogus");
        assert_eq!(s.iter().map(|(_, p)| *p).collect::<Vec<_>>(), [Pad::Down, Pad::A, Pad::B]);
        assert_eq!(s[2].0, Duration::from_millis(1350));
    }
}
