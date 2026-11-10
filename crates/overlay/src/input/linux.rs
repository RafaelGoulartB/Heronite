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
use super::{Button, Controller, ControllerId, Event, Options};

/// How often the thread looks for new controllers while idle.
const RESCAN: Duration = Duration::from_secs(2);
/// While Guide is held, how often the thread asks the kernel whether it still
/// is (its release may never reach us: see [`resync_guide`]).
const GUIDE_CHECK: Duration = Duration::from_millis(500);

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
    let mut last_guide_check = Instant::now();
    log::info!("overlay input: started");
    loop {
        if last_scan.elapsed() >= RESCAN {
            last_scan = Instant::now();
            let (changed, removed) = scan(&mut pads, &mut seen, &mut next_id, &core);
            for id in removed {
                let out = core.removed(id, pads.len());
                apply(out, &mut pads, &mut core, &mut on_event);
            }
            if changed {
                let mut list: Vec<Controller> = pads.values().map(|p| p.info.clone()).collect();
                list.sort_by(|a, b| a.id.cmp(&b.id));
                on_event(Event::Controllers(list));
            }
        }
        let now = Instant::now();
        let mut timeout = core.wait(now).unwrap_or(RESCAN).min(RESCAN);
        if core.guide_device().is_some() {
            timeout = timeout.min(GUIDE_CHECK.saturating_sub(last_guide_check.elapsed()));
        }
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
                apply(out, &mut pads, &mut core, &mut on_event);
            }
        }
        for (slot, id) in fds[1..].iter().zip(ids) {
            if slot.revents == 0 {
                continue;
            }
            let gone = slot.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0;
            let out = if gone { None } else { read_pad(id, &mut pads, &mut core) };
            match out {
                Some(events) => events.into_iter().for_each(&mut on_event),
                None => {
                    if let Some(p) = pads.remove(&id) {
                        log::info!("overlay input: controller gone: {}", p.info.name);
                        seen.retain(|_, v| *v != Some(id));
                        let out = core.removed(id, pads.len());
                        apply(out, &mut pads, &mut core, &mut on_event);
                        let mut list: Vec<Controller> = pads.values().map(|p| p.info.clone()).collect();
                        list.sort_by(|a, b| a.id.cmp(&b.id));
                        on_event(Event::Controllers(list));
                    }
                }
            }
        }
        // A Guide whose release never came: another program grabbed the
        // controller after us, or our grab came after its release.
        match core.guide_device() {
            Some(id) if last_guide_check.elapsed() >= GUIDE_CHECK => {
                last_guide_check = Instant::now();
                // Read what is queued first: the release may be there.
                if let Some(events) = read_pad(id, &mut pads, &mut core) {
                    events.into_iter().for_each(&mut on_event);
                }
                resync_guide(id, &mut pads, &mut core);
            }
            Some(_) => {}
            None => last_guide_check = Instant::now(),
        }
        while let Some(ev) = core.tick(Instant::now()) {
            on_event(ev);
        }
    }
    log::info!("overlay input: stopped");
}

/// Reads what a controller has queued and applies the grabs it asks for at
/// once (a grab that fails changes how the rest is read). Returns the events
/// for the overlay; `None` = it went away.
fn read_pad(id: usize, pads: &mut HashMap<usize, Pad>, core: &mut Core) -> Option<Vec<Event>> {
    let mut out = Vec::new();
    let events: Vec<evdev::InputEvent> = match pads.get_mut(&id)?.dev.fetch_events() {
        Ok(evs) => evs.collect(),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Some(out),
        Err(_) => return None,
    };
    let now = Instant::now();
    let mut stick_moved = false;
    let mut dropped = false;
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
                ABS_X | ABS_Y => {
                    let pad = pads.get_mut(&id)?;
                    if code == ABS_X {
                        pad.sx = normalize(value, pad.x);
                    } else {
                        pad.sy = -normalize(value, pad.y);
                    }
                    stick_moved = true;
                    continue;
                }
                ABS_HAT0X => hat(core, id, value, Button::Left, Button::Right, now),
                ABS_HAT0Y => hat(core, id, value, Button::Up, Button::Down, now),
                _ => continue,
            }
        } else if ty == evdev::EventType::SYNCHRONIZATION && code == SYN_DROPPED {
            // Releases may be among what the kernel dropped; what follows is
            // fresh, so this comes first.
            core.dropped(id);
            dropped = true;
            continue;
        } else {
            continue;
        };
        apply_grabs(step.grabs, pads, core);
        out.extend(step.events);
    }
    if stick_moved {
        let pad = pads.get(&id)?;
        let step = core.stick(id, pad.sx, pad.sy, now);
        apply_grabs(step.grabs, pads, core);
        out.extend(step.events);
    }
    // The kernel's queue overflowed: the Guide release may be among what it
    // dropped.
    if dropped {
        resync_guide(id, pads, core);
    }
    Some(out)
}

/// Forgets the Guide of `id` when the device says it is up although its
/// release never reached us. `EVIOCGKEY` reads the device itself, whoever
/// holds the grab. It also drops the key events still queued for us (the
/// kernel keeps the answer and the queue consistent), so callers read the
/// queue first.
fn resync_guide(id: usize, pads: &mut HashMap<usize, Pad>, core: &mut Core) {
    if !core.guide_held_on(id) {
        return;
    }
    let Some(pad) = pads.get(&id) else { return };
    let down = pad.dev.get_key_state().map(|keys| GUIDE_CODES.iter().any(|&c| keys.contains(evdev::KeyCode::new(c))));
    if let Ok(false) = down {
        log::info!("overlay input: {}: Guide is up but its release never came: forgetting it", pad.info.name);
        let out = core.lost_guide(id);
        apply_grabs(out.grabs, pads, core);
    }
}

/// A d-pad reported as a hat axis: -1 / 0 / 1.
fn hat(core: &mut Core, id: usize, value: i32, neg: Button, pos: Button, now: Instant) -> Output {
    let mut out = core.button(id, neg, value < 0, now);
    let more = core.button(id, pos, value > 0, now);
    out.events.extend(more.events);
    out.grabs.extend(more.grabs);
    out
}

fn apply(out: Output, pads: &mut HashMap<usize, Pad>, core: &mut Core, on_event: &mut impl FnMut(Event)) {
    apply_grabs(out.grabs, pads, core);
    for ev in out.events {
        on_event(ev);
    }
}

fn apply_grabs(grabs: Vec<Grab>, pads: &mut HashMap<usize, Pad>, core: &mut Core) {
    for g in grabs {
        match g {
            Grab::Hold(id) => {
                let busy = grab(pads.get_mut(&id), true).is_err_and(|e| e.raw_os_error() == Some(libc::EBUSY));
                // Another program holds the controller (another launcher, a
                // second Heronite, an input remapper): it gets the Guide
                // release, not us. Left held, the Guide would turn every later
                // press into a chord.
                if busy && core.guide_held_on(id) {
                    log::info!("overlay input: another program holds the controller: its Guide press is not ours");
                    apply_grabs(core.lost_guide(id).grabs, pads, core);
                }
            }
            Grab::HoldAll => pads.values_mut().for_each(|p| {
                let _ = grab(Some(p), true);
            }),
            Grab::ReleaseAll => pads.values_mut().for_each(|p| {
                let _ = grab(Some(p), false);
            }),
        }
    }
}

/// Grabs or releases `pad`. Asking for what it already has does nothing
/// (`RawDevice` remembers its grab).
fn grab(pad: Option<&mut Pad>, on: bool) -> io::Result<()> {
    let Some(pad) = pad else { return Ok(()) };
    let res = if on { pad.dev.grab() } else { pad.dev.ungrab() };
    if let Err(e) = &res {
        log::warn!("overlay input: {} {}: {e}", if on { "grabbing" } else { "releasing" }, pad.info.name);
    }
    res
}

/// Opens the controllers that appeared since the last scan. Returns whether
/// the list changed, and the controllers that went away (for the core).
fn scan(
    pads: &mut HashMap<usize, Pad>,
    seen: &mut HashMap<PathBuf, Option<usize>>,
    next_id: &mut usize,
    core: &Core,
) -> (bool, Vec<usize>) {
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return (false, Vec::new()) };
    let mut changed = false;
    let mut removed = Vec::new();
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
                    let _ = grab(Some(&mut pad), true);
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
                if pads.remove(id).is_some() {
                    changed = true;
                    removed.push(*id);
                }
            }
        }
        keep
    });
    (changed, removed)
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
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

const BTN_SOUTH: u16 = 0x130;
const BUS_BLUETOOTH: u16 = 0x05;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const SYN_DROPPED: u16 = 0x03;
/// `BTN_MODE` and `KEY_HOMEPAGE`: what [`map_key`] reads as Guide.
const GUIDE_CODES: [u16; 2] = [0x13c, 172];

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
    let info = Controller { name: dev.name().unwrap_or("Controller").to_string(), id: ControllerId::Evdev(path.to_path_buf()), bluetooth };
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
        c if GUIDE_CODES.contains(&c) => Button::Guide,
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
