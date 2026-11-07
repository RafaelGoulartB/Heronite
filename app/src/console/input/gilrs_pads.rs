//! Controllers through `gilrs` (evdev on Linux): a thread per listener that
//! blocks between events (no polling).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

use super::{PadButton, PadMsg, RawEvent, StickAxis, Translator};

/// The running thread; dropping it asks the thread to stop.
pub(super) struct Source {
    stop: Arc<AtomicBool>,
}

impl Drop for Source {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// `None` when the thread cannot start. When gilrs cannot (no udev, ...) the
/// thread ends at once and the receiver with it.
pub(super) fn start() -> Option<(Source, UnboundedReceiver<PadMsg>)> {
    let (tx, rx) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new().name("gamepad".into()).spawn(move || run(tx, flag)).map_err(|e| log::warn!("gamepad thread: {e}")).ok()?;
    Some((Source { stop }, rx))
}

/// Devices that only look like gamepads (the "joystick" interface of some
/// wireless keyboards) have no south (A) button.
fn is_gamepad(pad: &gilrs::Gamepad<'_>) -> bool {
    pad.button_code(gilrs::Button::South).is_some()
}

fn button(b: gilrs::Button) -> Option<PadButton> {
    use gilrs::Button as B;
    Some(match b {
        B::South => PadButton::South,
        B::East => PadButton::East,
        B::West => PadButton::West,
        B::North => PadButton::North,
        B::LeftTrigger => PadButton::LB,
        B::RightTrigger => PadButton::RB,
        B::LeftTrigger2 => PadButton::LT,
        B::RightTrigger2 => PadButton::RT,
        B::Start => PadButton::Start,
        B::Mode => PadButton::Guide,
        B::Select => PadButton::Select,
        B::DPadUp => PadButton::Up,
        B::DPadDown => PadButton::Down,
        B::DPadLeft => PadButton::Left,
        B::DPadRight => PadButton::Right,
        _ => return None,
    })
}

fn axis(a: gilrs::Axis) -> Option<StickAxis> {
    use gilrs::Axis as A;
    Some(match a {
        A::LeftStickX => StickAxis::LeftX,
        A::LeftStickY => StickAxis::LeftY,
        A::RightStickX => StickAxis::RightX,
        A::RightStickY => StickAxis::RightY,
        // Hat d-pads that are not mapped to buttons.
        A::DPadX => StickAxis::DPadX,
        A::DPadY => StickAxis::DPadY,
        _ => return None,
    })
}

fn run(tx: UnboundedSender<PadMsg>, stop: Arc<AtomicBool>) {
    // No force feedback (nothing here rumbles): gilrs runs it on a thread that
    // never ends. It outlived each console mode session holding the
    // controllers open, and once a controller was unplugged it logged
    // "Failed to modify effect ... ENODEV" every 50 ms until the app quit.
    let mut gilrs = match gilrs::GilrsBuilder::new().with_force_feedback(false).build() {
        Ok(g) => g,
        Err(err) => {
            log::warn!("controllers unavailable: {err}");
            return;
        }
    };
    let mut tr = Translator::default();
    let send = |m: PadMsg| tx.unbounded_send(m).is_ok();
    for (_, pad) in gilrs.gamepads().filter(|(_, p)| is_gamepad(p)) {
        log::info!("controller: {}", pad.name());
        send(PadMsg::Connected(pad.name().to_string(), false));
    }
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        // Wake for the next repeat, or at least once a second to see `stop`.
        let timeout = tr.wait(now).unwrap_or(Duration::from_secs(1)).min(Duration::from_secs(1));
        let Some(ev) = gilrs.next_event_blocking(Some(timeout)) else {
            if let Some(m) = tr.tick(Instant::now()) {
                if !send(m) {
                    break;
                }
            }
            continue;
        };
        if !matches!(ev.event, gilrs::EventType::Disconnected) && !is_gamepad(&gilrs.gamepad(ev.id)) {
            continue;
        }
        use gilrs::EventType as E;
        let raw = match ev.event {
            E::ButtonPressed(b, _) => button(b).map(|b| RawEvent::Button(b, true)),
            E::ButtonReleased(b, _) => button(b).map(|b| RawEvent::Button(b, false)),
            E::AxisChanged(a, v, _) => axis(a).map(|a| RawEvent::Axis(a, v)),
            E::Connected => {
                let name = gilrs.gamepad(ev.id).name().to_string();
                log::info!("controller connected: {name}");
                Some(RawEvent::Connected { name, plugged: true })
            }
            E::Disconnected => {
                let remaining = gilrs.gamepads().find(|(_, p)| p.is_connected() && is_gamepad(p)).map(|(_, p)| p.name().to_string());
                log::info!("controller disconnected; left: {remaining:?}");
                Some(RawEvent::Disconnected { remaining })
            }
            _ => None,
        };
        let Some(raw) = raw else { continue };
        let connection = matches!(raw, RawEvent::Connected { .. } | RawEvent::Disconnected { .. });
        // The theme is read per press, so a change applies at once.
        let swap = matches!(raw, RawEvent::Button(..)) && super::swap_ab();
        if let Some(m) = tr.event(raw, Instant::now(), swap) {
            // A listener that went away ends the thread; connection news are
            // not worth stopping for (the next press finds out).
            if !send(m) && !connection {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gilrs_buttons_keep_their_presses() {
        use gilrs::Button as B;
        assert_eq!(button(B::South), Some(PadButton::South));
        assert_eq!(button(B::LeftTrigger), Some(PadButton::LB));
        assert_eq!(button(B::LeftTrigger2), Some(PadButton::LT));
        assert_eq!(button(B::Mode), Some(PadButton::Guide));
        assert_eq!(button(B::DPadLeft), Some(PadButton::Left));
        assert_eq!(button(B::C), None);
        assert_eq!(axis(gilrs::Axis::DPadY), Some(StickAxis::DPadY));
        assert_eq!(axis(gilrs::Axis::LeftZ), None);
    }
}
