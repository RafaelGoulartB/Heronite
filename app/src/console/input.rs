//! Controller input for console mode: a `gilrs` thread turns gamepad events
//! into [`Pad`] presses (with key-repeat for held directions and the
//! sticks) and hands them to the UI over a channel. The thread only runs while
//! console mode is on, and blocks between events (no polling).
//!
//! The keyboard drives console mode too ([`pad_from_key`]), so it works
//! without a controller.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gpui_kit::Keystroke;

use super::Pad;

/// What the thread sends.
#[derive(Clone, Debug, PartialEq)]
pub enum PadMsg {
    Press(Pad),
    /// A controller is present: its name, and whether it was just plugged in
    /// (`false` for the ones found at start).
    Connected(String, bool),
    Disconnected,
}

/// Devices that only look like gamepads (the "joystick" interface of some
/// wireless keyboards) have no south (A) button.
fn is_gamepad(pad: &gilrs::Gamepad<'_>) -> bool {
    pad.button_code(gilrs::Button::South).is_some()
}

/// First repeat after this long, then every [`REPEAT_EVERY`].
const REPEAT_AFTER: Duration = Duration::from_millis(380);
const REPEAT_EVERY: Duration = Duration::from_millis(85);
/// Right stick scrolling repeats faster.
const SCROLL_EVERY: Duration = Duration::from_millis(60);
/// Stick deflection that counts as a press, and the one that releases it.
const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;

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

fn button_pad(b: gilrs::Button) -> Option<Pad> {
    use gilrs::Button as B;
    Some(match b {
        B::South => Pad::A,
        B::East => Pad::B,
        B::West => Pad::X,
        B::North => Pad::Y,
        B::LeftTrigger => Pad::LB,
        B::RightTrigger => Pad::RB,
        B::LeftTrigger2 => Pad::LT,
        B::RightTrigger2 => Pad::RT,
        B::Start | B::Mode => Pad::Start,
        B::Select => Pad::Select,
        B::DPadUp => Pad::Up,
        B::DPadDown => Pad::Down,
        B::DPadLeft => Pad::Left,
        B::DPadRight => Pad::Right,
        _ => return None,
    })
}

fn is_direction(p: Pad) -> bool {
    matches!(p, Pad::Up | Pad::Down | Pad::Left | Pad::Right)
}

/// The running controller thread; dropping it asks the thread to stop.
pub struct PadThread {
    stop: Arc<AtomicBool>,
}

impl Drop for PadThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts reading controllers. `None` when gilrs cannot start (no udev, ...).
pub fn start() -> Option<(PadThread, UnboundedReceiver<PadMsg>)> {
    let (tx, rx) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new().name("gamepad".into()).spawn(move || run(tx, flag)).map_err(|e| log::warn!("gamepad thread: {e}")).ok()?;
    Some((PadThread { stop }, rx))
}

fn run(tx: UnboundedSender<PadMsg>, stop: Arc<AtomicBool>) {
    let mut gilrs = match gilrs::Gilrs::new() {
        Ok(g) => g,
        Err(err) => {
            log::warn!("controllers unavailable: {err}");
            return;
        }
    };
    for (_, pad) in gilrs.gamepads().filter(|(_, p)| is_gamepad(p)) {
        log::info!("controller: {}", pad.name());
        tx.unbounded_send(PadMsg::Connected(pad.name().to_string(), false)).ok();
    }
    let mut rep = Repeater::default();
    // Sticks report X and Y separately; keep both.
    let (mut lx, mut ly, mut rx, mut ry) = (0f32, 0f32, 0f32, 0f32);
    let send = |p: Pad| tx.unbounded_send(PadMsg::Press(p)).is_ok();
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        // Wake for the next repeat, or at least once a second to see `stop`.
        let timeout = rep.wait(now).unwrap_or(Duration::from_secs(1)).min(Duration::from_secs(1));
        let Some(ev) = gilrs.next_event_blocking(Some(timeout)) else {
            if let Some(p) = rep.tick(Instant::now()) {
                if !send(p) {
                    break;
                }
            }
            continue;
        };
        let now = Instant::now();
        if !matches!(ev.event, gilrs::EventType::Disconnected) && !is_gamepad(&gilrs.gamepad(ev.id)) {
            continue;
        }
        use gilrs::{Axis, EventType as E};
        let out = match ev.event {
            E::ButtonPressed(b, _) => button_pad(b).map(|p| if is_direction(p) { rep.press(p, now) } else { p }),
            E::ButtonReleased(b, _) => {
                if let Some(p) = button_pad(b) {
                    rep.release(p);
                }
                None
            }
            E::AxisChanged(axis, v, _) => {
                match axis {
                    Axis::LeftStickX => lx = v,
                    Axis::LeftStickY => ly = v,
                    Axis::RightStickX => rx = v,
                    Axis::RightStickY => ry = v,
                    // Hat d-pads that are not mapped to buttons.
                    Axis::DPadX => lx = v,
                    Axis::DPadY => ly = v,
                    _ => {}
                }
                match axis {
                    Axis::RightStickX | Axis::RightStickY => rep.stick(true, rx, ry, now),
                    Axis::LeftStickX | Axis::LeftStickY | Axis::DPadX | Axis::DPadY => rep.stick(false, lx, ly, now),
                    _ => None,
                }
            }
            E::Connected => {
                let name = gilrs.gamepad(ev.id).name().to_string();
                log::info!("controller connected: {name}");
                tx.unbounded_send(PadMsg::Connected(name, true)).ok();
                None
            }
            E::Disconnected => {
                tx.unbounded_send(PadMsg::Disconnected).ok();
                None
            }
            _ => None,
        };
        if let Some(p) = out {
            if !send(p) {
                break;
            }
        }
    }
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
    use super::{parse_script, Repeater, REPEAT_AFTER, REPEAT_EVERY};
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
    fn script_steps() {
        let s = parse_script("down, a,wait:1000,b,bogus");
        assert_eq!(s.iter().map(|(_, p)| *p).collect::<Vec<_>>(), [Pad::Down, Pad::A, Pad::B]);
        assert_eq!(s[2].0, Duration::from_millis(1350));
    }
}
