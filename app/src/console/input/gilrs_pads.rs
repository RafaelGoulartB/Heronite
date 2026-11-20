//! Controllers through `gilrs` (evdev on Linux): a thread per listener that
//! blocks between events (no polling).

use std::collections::HashSet;
use std::path::Path;
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
pub(super) fn start(tr: Translator) -> Option<(Source, UnboundedReceiver<PadMsg>)> {
    let (tx, rx) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("gamepad".into())
        .spawn(move || run(tx, flag, tr))
        .map_err(|e| log::warn!("gamepad thread: {e}"))
        .ok()?;
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

/// Kernel drivers that report the Xbox X and Y buttons by their letters, as
/// `BTN_X` and `BTN_Y`, which the kernel also names `BTN_NORTH` and
/// `BTN_WEST`. For a controller SDL's mapping database knows, the mapping
/// puts them right; for one it does not (a GameSir G7 SE), gilrs reads the
/// kernel's names and the top and left buttons come out swapped.
const LETTER_DRIVERS: [&str; 1] = ["xpad"];

/// The pad's top and left buttons arrive swapped (see [`LETTER_DRIVERS`]).
fn swaps_north_west(pad: &gilrs::Gamepad<'_>) -> bool {
    pad.mapping_source() == gilrs::MappingSource::Driver
        && pad
            .vendor_id()
            .zip(pad.product_id())
            .and_then(|(v, p)| driver_of(Path::new("/sys/class/input"), v, p))
            .is_some_and(|d| LETTER_DRIVERS.contains(&d.as_str()))
}

/// The kernel driver of the input device with this USB vendor and product
/// id, from sysfs (`<root>/<entry>/device/id/{vendor,product}` and the
/// `device/device/driver` link).
fn driver_of(root: &Path, vendor: u16, product: u16) -> Option<String> {
    std::fs::read_dir(root).ok()?.flatten().find_map(|entry| {
        let dev = entry.path().join("device");
        let id = |f: &str| std::fs::read_to_string(dev.join("id").join(f)).ok().and_then(|s| u16::from_str_radix(s.trim(), 16).ok());
        if id("vendor") != Some(vendor) || id("product") != Some(product) {
            return None;
        }
        let driver = std::fs::read_link(dev.join("device").join("driver")).ok()?;
        Some(driver.file_name()?.to_string_lossy().into_owned())
    })
}

/// A button of a pad whose top and left buttons arrive swapped, put back.
fn face(b: PadButton, swapped: bool) -> PadButton {
    match (b, swapped) {
        (PadButton::North, true) => PadButton::West,
        (PadButton::West, true) => PadButton::North,
        _ => b,
    }
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

fn run(tx: UnboundedSender<PadMsg>, stop: Arc<AtomicBool>, mut tr: Translator) {
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
    let send = |m: PadMsg| tx.unbounded_send(m).is_ok();
    // The pads whose top and left buttons arrive swapped.
    let mut swapped: HashSet<gilrs::GamepadId> = HashSet::new();
    for (id, pad) in gilrs.gamepads().filter(|(_, p)| is_gamepad(p)) {
        log::info!("controller: {}", pad.name());
        if swaps_north_west(&pad) {
            log::info!("controller {}: X and Y read as the kernel names them, swapped back", pad.name());
            swapped.insert(id);
        }
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
        let fixed = swapped.contains(&ev.id);
        let raw = match ev.event {
            E::ButtonPressed(b, _) => button(b).map(|b| RawEvent::Button(face(b, fixed), true)),
            E::ButtonReleased(b, _) => button(b).map(|b| RawEvent::Button(face(b, fixed), false)),
            E::AxisChanged(a, v, _) => axis(a).map(|a| RawEvent::Axis(a, v)),
            E::Connected => {
                let pad = gilrs.gamepad(ev.id);
                let name = pad.name().to_string();
                log::info!("controller connected: {name}");
                if swaps_north_west(&pad) {
                    log::info!("controller {name}: X and Y read as the kernel names them, swapped back");
                    swapped.insert(ev.id);
                }
                Some(RawEvent::Connected { name, plugged: true })
            }
            E::Disconnected => {
                swapped.remove(&ev.id);
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

    #[test]
    fn swapped_pads_get_their_top_and_left_buttons_back() {
        assert_eq!(face(PadButton::North, true), PadButton::West);
        assert_eq!(face(PadButton::West, true), PadButton::North);
        assert_eq!(face(PadButton::South, true), PadButton::South);
        assert_eq!(face(PadButton::North, false), PadButton::North);
    }

    #[test]
    fn the_driver_comes_from_sysfs() {
        let root = tempfile::tempdir().unwrap();
        let drivers = root.path().join("drivers");
        std::fs::create_dir_all(drivers.join("xpad")).unwrap();
        let add = |name: &str, vendor: &str, product: &str| {
            let dev = root.path().join("class").join(name).join("device");
            std::fs::create_dir_all(dev.join("id")).unwrap();
            std::fs::write(dev.join("id/vendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(dev.join("id/product"), format!("{product}\n")).unwrap();
            std::fs::create_dir_all(dev.join("device")).unwrap();
            std::os::unix::fs::symlink(drivers.join("xpad"), dev.join("device/driver")).unwrap();
        };
        add("event24", "3537", "1010");
        let class = root.path().join("class");
        assert_eq!(driver_of(&class, 0x3537, 0x1010).as_deref(), Some("xpad"));
        assert_eq!(driver_of(&class, 0x045e, 0x028e), None);
        assert_eq!(driver_of(&root.path().join("missing"), 0x3537, 0x1010), None);
    }
}
