//! Controllers through SDL on Windows (`heronite_platform::controllers`).
//!
//! SDL's controller state is global to the process, so one thread owns it and
//! serves every listener (console mode, the prompt to enter it), each with its
//! own [`Translator`]. The thread opens SDL when the first listener comes and
//! closes it (letting go of the controllers) when the last one leaves.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use heronite_platform::controllers::{Axis, Button, Controllers, Event};

use super::{PadButton, PadMsg, RawEvent, StickAxis, Translator};

/// How often open controllers are read: Windows' timer ticks every ~15.6 ms,
/// so this wakes on every tick (what the overlay's XInput polling does).
const POLL: Duration = Duration::from_millis(8);
/// How often open controllers are read when only [`super::start_buttons`]
/// listens (a button held for a second needs no 8 ms reads).
const BUTTONS_POLL: Duration = Duration::from_millis(30);
/// How often SDL looks for new controllers while none is open.
const IDLE_POLL: Duration = Duration::from_millis(250);
/// Controllers SDL reports this soon after it starts count as found at start.
const SETTLE: Duration = Duration::from_secs(1);

struct Listener {
    id: u64,
    tx: UnboundedSender<PadMsg>,
    tr: Translator,
    /// Told about the controllers that were already open.
    greeted: bool,
}

#[derive(Default)]
struct Hub {
    listeners: Mutex<Vec<Listener>>,
    /// Wakes the thread when a listener comes or goes.
    changed: Condvar,
    next_id: AtomicU64,
}

impl Hub {
    fn lock(&self) -> MutexGuard<'_, Vec<Listener>> {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A listener; dropping it unsubscribes (its receiver then ends).
pub(super) struct Source {
    hub: Arc<Hub>,
    id: u64,
}

impl Drop for Source {
    fn drop(&mut self) {
        self.hub.lock().retain(|l| l.id != self.id);
        self.hub.changed.notify_all();
    }
}

/// The hub, with its thread started on first use (`None` if it could not
/// be; the next call tries again).
fn hub() -> Option<Arc<Hub>> {
    static HUB: Mutex<Option<Arc<Hub>>> = Mutex::new(None);
    let mut slot = HUB.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hub) = &*slot {
        return Some(hub.clone());
    }
    let hub = Arc::new(Hub::default());
    let worker = hub.clone();
    std::thread::Builder::new().name("gamepad".into()).spawn(move || run(&worker)).map_err(|e| log::warn!("gamepad thread: {e}")).ok()?;
    *slot = Some(hub.clone());
    Some(hub)
}

pub(super) fn start(tr: Translator) -> Option<(Source, UnboundedReceiver<PadMsg>)> {
    let hub = hub()?;
    let (tx, rx) = unbounded();
    let id = hub.next_id.fetch_add(1, Ordering::Relaxed);
    hub.lock().push(Listener { id, tx, tr, greeted: false });
    hub.changed.notify_all();
    Some((Source { hub, id }, rx))
}

fn run(hub: &Hub) {
    // With when it was opened.
    let mut sdl: Option<(Controllers, Instant)> = None;
    loop {
        // Nobody listens: let go of the controllers and sleep until someone does.
        {
            let mut listeners = hub.lock();
            if listeners.is_empty() {
                if let Some(old) = sdl.take() {
                    // Closing SDL takes a moment (the devices close, its thread
                    // stops): not while `start` would wait for the lock.
                    drop(listeners);
                    drop(old);
                    listeners = hub.lock();
                }
                while listeners.is_empty() {
                    listeners = hub.changed.wait(listeners).unwrap_or_else(|e| e.into_inner());
                }
            }
        }
        if sdl.is_none() {
            match Controllers::open() {
                Ok(pads) => sdl = Some((pads, Instant::now())),
                Err(e) => {
                    log::warn!("controllers unavailable: {e}");
                    // Their receivers end, as when gilrs cannot start; a later
                    // listener tries again.
                    hub.lock().clear();
                    continue;
                }
            }
        }
        let Some((pads, opened)) = sdl.as_mut() else { continue };
        // Controllers SDL finds just after it starts were already there.
        let settling = opened.elapsed() < SETTLE;
        // The controllers found so far go to new listeners before this
        // round's events, which only greeted listeners get.
        {
            let mut listeners = hub.lock();
            for l in listeners.iter_mut().filter(|l| !l.greeted) {
                l.greeted = true;
                for name in pads.names() {
                    l.tx.unbounded_send(PadMsg::Connected(name.to_string(), false)).ok();
                }
            }
        }
        let mut events = Vec::new();
        while let Some(ev) = pads.poll() {
            events.push(raw_event(ev, settling));
        }
        let mut listeners = hub.lock();
        let now = Instant::now();
        let swap = super::swap_ab();
        listeners.retain_mut(|l| {
            if !l.greeted {
                return true;
            }
            let mut alive = true;
            for raw in events.iter().filter_map(|e| e.clone()) {
                if let Some(m) = l.tr.event(raw, now, swap) {
                    alive &= l.tx.unbounded_send(m).is_ok();
                }
            }
            if let Some(m) = l.tr.tick(now) {
                alive &= l.tx.unbounded_send(m).is_ok();
            }
            alive
        });
        let every = match pads.any_open() {
            false => IDLE_POLL,
            true if listeners.iter().all(|l| l.tr.buttons_only()) => BUTTONS_POLL,
            true => POLL,
        };
        let wait = listeners.iter().filter_map(|l| l.tr.wait(now)).fold(every, Duration::min);
        // Woken early when a listener comes or goes.
        drop(hub.changed.wait_timeout(listeners, wait));
    }
}

/// `settling`: SDL started a moment ago, so an added controller was already
/// connected (no "connected" notice for it).
fn raw_event(ev: Event, settling: bool) -> Option<RawEvent> {
    Some(match ev {
        Event::Added { name } => RawEvent::Connected { name, plugged: !settling },
        Event::Removed { remaining } => RawEvent::Disconnected { remaining },
        Event::Button(b, down) => RawEvent::Button(button(b), down),
        Event::Axis(Axis::LeftTrigger, v) => RawEvent::Trigger { right: false, value: v },
        Event::Axis(Axis::RightTrigger, v) => RawEvent::Trigger { right: true, value: v },
        Event::Axis(a, v) => RawEvent::Axis(stick(a)?, v),
    })
}

fn button(b: Button) -> PadButton {
    match b {
        Button::South => PadButton::South,
        Button::East => PadButton::East,
        Button::West => PadButton::West,
        Button::North => PadButton::North,
        Button::Back => PadButton::Select,
        Button::Guide => PadButton::Guide,
        Button::Start => PadButton::Start,
        Button::LeftShoulder => PadButton::LB,
        Button::RightShoulder => PadButton::RB,
        Button::DPadUp => PadButton::Up,
        Button::DPadDown => PadButton::Down,
        Button::DPadLeft => PadButton::Left,
        Button::DPadRight => PadButton::Right,
    }
}

fn stick(a: Axis) -> Option<StickAxis> {
    Some(match a {
        Axis::LeftX => StickAxis::LeftX,
        Axis::LeftY => StickAxis::LeftY,
        Axis::RightX => StickAxis::RightX,
        Axis::RightY => StickAxis::RightY,
        Axis::LeftTrigger | Axis::RightTrigger => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn a_listener_that_leaves_has_its_channel_closed() {
        let (a, rx_a) = start(Translator::default()).expect("the controller thread starts");
        // One reading the buttons as they are (`start_buttons`) shares it too.
        let (b, rx_b) = start(Translator::buttons()).expect("a second listener shares it");
        drop(a);
        // Ends once the hub let go of the sender; only connection news can
        // come before (from controllers on this machine).
        let got = futures::executor::block_on(rx_a.collect::<Vec<_>>());
        assert!(got.iter().all(|m| matches!(m, PadMsg::Connected(..))), "{got:?}");
        drop(b);
        let got = futures::executor::block_on(rx_b.collect::<Vec<_>>());
        assert!(got.iter().all(|m| matches!(m, PadMsg::Connected(..))), "{got:?}");
        assert!(hub().expect("hub").lock().is_empty());
    }

    #[test]
    fn sdl_events_become_raw_events() {
        assert_eq!(raw_event(Event::Button(Button::Back, true), false), Some(RawEvent::Button(PadButton::Select, true)));
        assert_eq!(raw_event(Event::Button(Button::Guide, false), false), Some(RawEvent::Button(PadButton::Guide, false)));
        assert_eq!(raw_event(Event::Axis(Axis::RightTrigger, 0.9), false), Some(RawEvent::Trigger { right: true, value: 0.9 }));
        assert_eq!(raw_event(Event::Axis(Axis::LeftY, 1.0), false), Some(RawEvent::Axis(StickAxis::LeftY, 1.0)));
        assert_eq!(
            raw_event(Event::Added { name: "PS5 Controller".into() }, false),
            Some(RawEvent::Connected { name: "PS5 Controller".into(), plugged: true })
        );
        assert_eq!(
            raw_event(Event::Added { name: "PS5 Controller".into() }, true),
            Some(RawEvent::Connected { name: "PS5 Controller".into(), plugged: false })
        );
        assert_eq!(raw_event(Event::Removed { remaining: None }, true), Some(RawEvent::Disconnected { remaining: None }));
    }
}
