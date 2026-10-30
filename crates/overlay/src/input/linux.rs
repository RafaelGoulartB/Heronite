//! The input thread, on evdev. It runs only while a game launched by the
//! app is running and the overlay is on. It sleeps in `poll` between events;
//! with no input it wakes every [`RESCAN`] to notice new controllers.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use super::core::{Core, Grab, Output};
use super::{Button, Controller, Event, Options};

/// How often the thread looks for new controllers while idle.
const RESCAN: Duration = Duration::from_secs(2);

enum Cmd {
    Open,
    Close,
    Options(Options),
}

/// The running input thread. Dropping it stops the thread, which releases
/// every grab (closing a node ends its grab).
pub struct Watcher {
    tx: Sender<Cmd>,
    wake: UnixStream,
}

impl Watcher {
    /// Starts reading controllers; `on_event` runs on the input thread.
    pub fn start(opts: Options, on_event: impl FnMut(Event) + Send + 'static) -> io::Result<Self> {
        let (wake, wake_rx) = UnixStream::pair()?;
        wake_rx.set_nonblocking(true)?;
        let (tx, rx) = channel();
        std::thread::Builder::new().name("overlay-input".into()).spawn(move || run(opts, rx, wake_rx, on_event))?;
        Ok(Self { tx, wake })
    }

    /// The overlay closed by itself: give the controllers back to the game.
    pub fn close(&self) {
        self.send(Cmd::Close);
    }

    /// The overlay opened without the Guide button (the keyboard shortcut,
    /// a debug hook): hold the controllers, as Guide would.
    pub fn open(&self) {
        self.send(Cmd::Open);
    }

    pub fn set_options(&self, opts: Options) {
        self.send(Cmd::Options(opts));
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok() {
            let _ = (&self.wake).write(&[1]);
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // The thread sees the closed socket and ends.
        let _ = self.wake.shutdown(std::net::Shutdown::Both);
    }
}

/// An open controller.
struct Pad {
    dev: evdev::raw_stream::RawDevice,
    info: Controller,
    /// `(min, max)` of the left stick axes.
    x: (i32, i32),
    y: (i32, i32),
    sx: f32,
    sy: f32,
}

fn run(opts: Options, rx: Receiver<Cmd>, mut wake: UnixStream, mut on_event: impl FnMut(Event)) {
    let mut core = Core::new(opts);
    let mut pads: HashMap<usize, Pad> = HashMap::new();
    // Nodes looked at: `Some(id)` = a controller, `None` = something else.
    let mut seen: HashMap<PathBuf, Option<usize>> = HashMap::new();
    let mut next_id = 0usize;
    let mut last_scan = Instant::now() - RESCAN;
    log::info!("overlay input: started");
    loop {
        if last_scan.elapsed() >= RESCAN {
            last_scan = Instant::now();
            if scan(&mut pads, &mut seen, &mut next_id, &core) {
                let mut list: Vec<Controller> = pads.values().map(|p| p.info.clone()).collect();
                list.sort_by(|a, b| a.node.cmp(&b.node));
                on_event(Event::Controllers(list));
            }
        }
        let now = Instant::now();
        let timeout = core.wait(now).unwrap_or(RESCAN).min(RESCAN);
        let ids: Vec<usize> = pads.keys().copied().collect();
        let mut fds: Vec<libc::pollfd> = std::iter::once(wake.as_raw_fd())
            .chain(ids.iter().map(|id| pads[id].dev.as_raw_fd()))
            .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
            .collect();
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd entries.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout.as_millis() as libc::c_int) };
        if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            log::warn!("overlay input: poll failed: {}", io::Error::last_os_error());
            std::thread::sleep(Duration::from_millis(200));
        }
        if fds[0].revents != 0 {
            let mut buf = [0u8; 64];
            // 0 bytes: the handle was dropped.
            if let Ok(0) = wake.read(&mut buf) {
                break;
            }
            for cmd in rx.try_iter() {
                let out = match cmd {
                    Cmd::Open => core.open(),
                    Cmd::Close => core.close(),
                    Cmd::Options(o) => {
                        core.opts = o;
                        Output::default()
                    }
                };
                apply(out, &mut pads, &mut on_event);
            }
        }
        for (slot, id) in fds[1..].iter().zip(ids) {
            if slot.revents == 0 {
                continue;
            }
            let gone = slot.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0;
            let out = if gone { None } else { read_pad(id, &mut pads, &mut core) };
            match out {
                Some(out) => apply(out, &mut pads, &mut on_event),
                None => {
                    if let Some(p) = pads.remove(&id) {
                        log::info!("overlay input: controller gone: {}", p.info.name);
                        seen.retain(|_, v| *v != Some(id));
                        let out = core.removed(id, pads.len());
                        apply(out, &mut pads, &mut on_event);
                        let mut list: Vec<Controller> = pads.values().map(|p| p.info.clone()).collect();
                        list.sort_by(|a, b| a.node.cmp(&b.node));
                        on_event(Event::Controllers(list));
                    }
                }
            }
        }
        while let Some(ev) = core.tick(Instant::now()) {
            on_event(ev);
        }
    }
    log::info!("overlay input: stopped");
}

/// Reads what a controller has queued. `None` = it went away.
fn read_pad(id: usize, pads: &mut HashMap<usize, Pad>, core: &mut Core) -> Option<Output> {
    let pad = pads.get_mut(&id)?;
    let mut out = Output::default();
    let events: Vec<evdev::InputEvent> = match pad.dev.fetch_events() {
        Ok(evs) => evs.collect(),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Some(out),
        Err(_) => return None,
    };
    let now = Instant::now();
    let mut stick_moved = false;
    for ev in events {
        let (ty, code, value) = (ev.event_type(), ev.code(), ev.value());
        let step = if ty == evdev::EventType::KEY {
            // 2 = autorepeat of a held key: the core repeats on its own.
            match (map_key(code), value) {
                (Some(b), 0 | 1) => core.button(id, b, value == 1, now),
                _ => continue,
            }
        } else if ty == evdev::EventType::ABSOLUTE {
            match code {
                ABS_X => {
                    pad.sx = normalize(value, pad.x);
                    stick_moved = true;
                    continue;
                }
                ABS_Y => {
                    pad.sy = -normalize(value, pad.y);
                    stick_moved = true;
                    continue;
                }
                ABS_HAT0X => hat(core, id, value, Button::Left, Button::Right, now),
                ABS_HAT0Y => hat(core, id, value, Button::Up, Button::Down, now),
                _ => continue,
            }
        } else {
            continue;
        };
        out.events.extend(step.events);
        out.grabs.extend(step.grabs);
    }
    if stick_moved {
        let step = core.stick(id, pad.sx, pad.sy, now);
        out.events.extend(step.events);
        out.grabs.extend(step.grabs);
    }
    Some(out)
}

/// A d-pad reported as a hat axis: -1 / 0 / 1.
fn hat(core: &mut Core, id: usize, value: i32, neg: Button, pos: Button, now: Instant) -> Output {
    let mut out = core.button(id, neg, value < 0, now);
    let more = core.button(id, pos, value > 0, now);
    out.events.extend(more.events);
    out.grabs.extend(more.grabs);
    out
}

fn apply(out: Output, pads: &mut HashMap<usize, Pad>, on_event: &mut impl FnMut(Event)) {
    for g in out.grabs {
        match g {
            Grab::Hold(id) => grab(pads.get_mut(&id), true),
            Grab::HoldAll => pads.values_mut().for_each(|p| grab(Some(p), true)),
            Grab::ReleaseAll => pads.values_mut().for_each(|p| grab(Some(p), false)),
        }
    }
    for ev in out.events {
        on_event(ev);
    }
}

fn grab(pad: Option<&mut Pad>, on: bool) {
    let Some(pad) = pad else { return };
    let res = if on { pad.dev.grab() } else { pad.dev.ungrab() };
    if let Err(e) = res {
        log::warn!("overlay input: {} {}: {e}", if on { "grabbing" } else { "releasing" }, pad.info.name);
    }
}

/// Opens the controllers that appeared since the last scan. Returns whether
/// the list changed.
fn scan(pads: &mut HashMap<usize, Pad>, seen: &mut HashMap<PathBuf, Option<usize>>, next_id: &mut usize, core: &Core) -> bool {
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return false };
    let mut changed = false;
    let mut present = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")) {
            continue;
        }
        present.push(path.clone());
        if seen.contains_key(&path) {
            continue;
        }
        match open_pad(&path) {
            // Not readable (yet: the seat's access rule may come a moment
            // after the node): try again on the next scan.
            Err(_) => {}
            Ok(None) => {
                seen.insert(path, None);
            }
            Ok(Some(mut pad)) => {
                let id = *next_id;
                *next_id += 1;
                log::info!("overlay input: controller {} ({})", pad.info.name, path.display());
                // Opened while the overlay is open: hold it like the others.
                if core.holds(id) {
                    grab(Some(&mut pad), true);
                }
                seen.insert(path, Some(id));
                pads.insert(id, pad);
                changed = true;
            }
        }
    }
    // Nodes that disappeared without an error on their fd.
    seen.retain(|path, id| {
        let keep = present.contains(path);
        if !keep {
            if let Some(id) = id {
                changed |= pads.remove(id).is_some();
            }
        }
        keep
    });
    changed
}

/// The controllers this app can read now (Settings shows them). Blocking:
/// opens every input node once.
pub fn list_controllers() -> Vec<Controller> {
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return Vec::new() };
    let mut out: Vec<Controller> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")))
        .filter_map(|p| open_pad(&p).ok().flatten().map(|pad| pad.info))
        .collect();
    out.sort_by(|a, b| a.node.cmp(&b.node));
    out
}

const BTN_SOUTH: u16 = 0x130;
const BUS_BLUETOOTH: u16 = 0x05;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

/// Opens `path` when it is a controller: it has a south (A) button. Devices
/// that only look like gamepads (the "joystick" interface of some wireless
/// keyboards) do not.
fn open_pad(path: &Path) -> io::Result<Option<Pad>> {
    let dev = evdev::raw_stream::RawDevice::open(path)?;
    let is_pad = dev.supported_keys().is_some_and(|k| k.contains(evdev::KeyCode::new(BTN_SOUTH)));
    if !is_pad {
        return Ok(None);
    }
    set_nonblocking(&dev)?;
    let mut x = (-32768, 32767);
    let mut y = (-32768, 32767);
    if let Ok(axes) = dev.get_absinfo() {
        for (axis, info) in axes {
            match axis.0 {
                ABS_X => x = (info.minimum(), info.maximum()),
                ABS_Y => y = (info.minimum(), info.maximum()),
                _ => {}
            }
        }
    }
    let id = dev.input_id();
    let bluetooth = (id.bus_type().0 == BUS_BLUETOOTH).then(|| dev.unique_name().unwrap_or_default().to_uppercase()).filter(|m| is_mac(m));
    let info = Controller { name: dev.name().unwrap_or("Controller").to_string(), node: path.to_path_buf(), bluetooth };
    Ok(Some(Pad { dev, info, x, y, sx: 0., sy: 0. }))
}

fn set_nonblocking(dev: &evdev::raw_stream::RawDevice) -> io::Result<()> {
    let fd = dev.as_raw_fd();
    // SAFETY: plain fcntl calls on a descriptor the device owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// `AA:BB:CC:DD:EE:FF`.
fn is_mac(s: &str) -> bool {
    s.len() == 17 && s.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Kernel key code to a button. X / Y follow `xpad` (Xbox controllers, the
/// most common on Linux): `BTN_X` (0x133) is X.
fn map_key(code: u16) -> Option<Button> {
    Some(match code {
        0x130 => Button::A,
        0x131 => Button::B,
        0x133 => Button::X,
        0x134 => Button::Y,
        0x136 => Button::LB,
        0x137 => Button::RB,
        0x138 => Button::LT,
        0x139 => Button::RT,
        0x13a => Button::Select,
        0x13b => Button::Start,
        // BTN_MODE, and KEY_HOMEPAGE (what some controllers report for Guide).
        0x13c | 172 => Button::Guide,
        0x220 => Button::Up,
        0x221 => Button::Down,
        0x222 => Button::Left,
        0x223 => Button::Right,
        _ => return None,
    })
}

/// `value` within `(min, max)` to -1..=1.
fn normalize(value: i32, (min, max): (i32, i32)) -> f32 {
    if max <= min {
        return 0.;
    }
    let t = (value - min) as f32 / (max - min) as f32;
    (t * 2. - 1.).clamp(-1., 1.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_and_keys() {
        assert_eq!(normalize(-32768, (-32768, 32767)), -1.);
        assert!(normalize(0, (-32768, 32767)).abs() < 0.001);
        assert_eq!(normalize(255, (0, 255)), 1.);
        assert_eq!(normalize(5, (3, 3)), 0.);
        assert_eq!(map_key(0x13c), Some(Button::Guide));
        assert_eq!(map_key(172), Some(Button::Guide));
        assert_eq!(map_key(0x137), Some(Button::RB));
        assert_eq!(map_key(0x2c0), None);
        assert!(is_mac("aa:bb:cc:dd:ee:ff"));
        assert!(!is_mac("usb-0000:00:14.0-1/input0"));
    }
}
