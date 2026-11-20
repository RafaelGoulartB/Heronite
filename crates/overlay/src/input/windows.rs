//! XInput polling never consumes input: games may also receive the controls.
//! With no controller connected, sleep until the next slot scan.

use std::io;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use heronite_platform::gamepad;

use super::core::Core;
use super::xinput::{self, poll_wait, Adapter, Change, Failures};
use super::{Button, Controller, ControllerId, Event, Options};

const RESCAN: Duration = Duration::from_secs(1);
const SLOTS: usize = gamepad::CONTROLLER_COUNT as usize;

enum Cmd {
    Open,
    Close,
    Options(Options),
}

/// Dropping the handle stops polling. XInput offers no exclusive controller grab.
pub struct Watcher {
    tx: Sender<Cmd>,
}

impl Watcher {
    /// `on_event` runs on the input thread. Native polling failures are logged;
    /// thread creation errors are returned.
    pub fn start(opts: Options, on_event: impl FnMut(Event) + Send + 'static) -> io::Result<Self> {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("overlay-input".into()).spawn(move || run(options(opts), rx, on_event))?;
        Ok(Self { tx })
    }

    pub fn close(&self) {
        self.send(Cmd::Close);
    }

    pub fn open(&self) {
        self.send(Cmd::Open);
    }

    pub fn set_options(&self, opts: Options) {
        self.send(Cmd::Options(options(opts)));
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_err() {
            log::warn!("overlay input: XInput worker has stopped");
        }
    }
}

fn options(mut opts: Options) -> Options {
    if matches!(opts.screenshot_button, Button::Start | Button::Select) {
        log::info!("overlay input: Back and Start are reserved for Guide on Windows; using RB for screenshots");
        opts.screenshot_button = Button::RB;
    }
    opts
}

fn run(opts: Options, rx: Receiver<Cmd>, mut on_event: impl FnMut(Event)) {
    let mut core = Core::new(opts);
    let mut pads: [Option<Adapter>; SLOTS] = std::array::from_fn(|_| None);
    let mut next_scan = [Instant::now(); SLOTS];
    let mut failures: [Failures; SLOTS] = std::array::from_fn(|_| Failures::default());
    log::info!("overlay input: started XInput polling (Back+Start, non-exclusive)");
    loop {
        let now = Instant::now();
        let mut changed = false;
        for index in 0..SLOTS {
            if pads[index].is_none() && now < next_scan[index] {
                continue;
            }
            next_scan[index] = now + RESCAN;
            // `Ok(None)` is ERROR_DEVICE_NOT_CONNECTED: gone at once.
            let state = match gamepad::state(index as u32) {
                Ok(state) => {
                    if failures[index].clear() {
                        log::info!("overlay input: XInput slot {} recovered", index + 1);
                    }
                    state
                }
                Err(error) => {
                    let failure = &mut failures[index];
                    if failure.record(error.to_string()) {
                        log::warn!("overlay input: XInputGetState slot {}: {error}", index + 1);
                    }
                    if pads[index].is_some() && !failure.disconnects() {
                        // Possibly transient: keep the controller and its held state.
                        continue;
                    }
                    None
                }
            };
            match state {
                Some(state) => {
                    if let Some(pad) = &mut pads[index] {
                        for change in pad.update(state.into(), now) {
                            let out = match change {
                                Change::Button(button, down) => core.button(index, button, down, now),
                                Change::Stick(x, y) => core.stick(index, x, y, now),
                            };
                            // Grab requests belong to evdev only; XInput cannot apply them.
                            for event in out.events {
                                on_event(event);
                            }
                        }
                    } else {
                        log::info!("overlay input: XInput controller {} connected", index + 1);
                        pads[index] = Some(Adapter::new(state.into()));
                        changed = true;
                    }
                }
                None => {
                    if pads[index].take().is_some() {
                        if failures[index].disconnects() {
                            log::warn!("overlay input: XInput controller {} dropped after repeated read errors", index + 1);
                        } else {
                            log::info!("overlay input: XInput controller {} disconnected", index + 1);
                        }
                        for event in core.removed(index, pads.iter().flatten().count()).events {
                            on_event(event);
                        }
                        changed = true;
                    }
                }
            }
        }
        if changed {
            on_event(Event::Controllers(
                pads.iter().enumerate().filter(|(_, pad)| pad.is_some()).map(|(index, _)| controller(index as u32)).collect(),
            ));
        }
        if let Some(event) = core.tick(Instant::now()) {
            on_event(event);
        }
        let now = Instant::now();
        let wait = poll_wait(pads.iter().any(Option::is_some), core.wait(now), &next_scan, now);
        match rx.recv_timeout(wait) {
            Ok(Cmd::Open) => xinput::ui_toggled(&mut core, true, pads.iter_mut().flatten()),
            Ok(Cmd::Close) => xinput::ui_toggled(&mut core, false, pads.iter_mut().flatten()),
            Ok(Cmd::Options(opts)) => core.opts = opts,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    log::info!("overlay input: stopped XInput polling");
}

fn controller(index: u32) -> Controller {
    Controller { name: format!("XInput controller {}", index + 1), id: ControllerId::XInput(index), bluetooth: None }
}

/// Blocking snapshot of the four documented XInput slots. Failures are logged,
/// rather than presented as unsupported controllers or fabricated devices.
pub fn list_controllers() -> Vec<Controller> {
    (0..gamepad::CONTROLLER_COUNT)
        .filter_map(|index| match gamepad::state(index) {
            Ok(Some(_)) => Some(controller(index)),
            Ok(None) => None,
            Err(error) => {
                log::warn!("overlay input: XInputGetState slot {}: {error}", index + 1);
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_identity_is_its_slot_without_claiming_a_transport() {
        for index in 0..gamepad::CONTROLLER_COUNT {
            let controller = controller(index);
            assert_eq!(controller.id, ControllerId::XInput(index));
            assert!(controller.bluetooth.is_none());
        }
    }

    #[test]
    fn imported_reserved_screenshot_buttons_use_rb() {
        let opts =
            Options { screenshot: true, screenshot_button: Button::Start, volume: true, moment: true, replay: false, console_hold: false };
        for button in [Button::Start, Button::Select] {
            assert_eq!(options(Options { screenshot_button: button, ..opts }).screenshot_button, Button::RB);
        }
        assert_eq!(options(Options { screenshot_button: Button::A, ..opts }).screenshot_button, Button::A);
    }
}
