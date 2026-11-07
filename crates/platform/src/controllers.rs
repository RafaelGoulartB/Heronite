//! Game controllers of every kind through SDL 3, for console mode on Windows.
//!
//! Windows.Gaming.Input and XInput only read some controllers well: XInput
//! knows only Xbox pads, and Windows.Gaming.Input reads PlayStation and
//! Nintendo controllers through their basic HID report, which stops changing
//! as soon as Steam (or any SDL program) switches the controller to its full
//! report. SDL talks to DualShock 4, DualSense, Switch Pro and Joy-Con
//! controllers directly (HIDAPI), reads both kinds of report, and uses XInput,
//! Raw Input and DirectInput for the rest.
//!
//! SDL is told not to switch controllers to their full report itself
//! (`SDL_JOYSTICK_ENHANCED_REPORTS=0`): that change cannot be undone on a
//! PlayStation controller without turning it off, and it would stop games that
//! read it through DirectInput from seeing it. Switch controllers go back to
//! the report mode they had when they are closed.
//!
//! [`Controllers`] owns SDL's controller subsystem: one may exist at a time in
//! the process, and it is used from the thread that opened it (it is not
//! `Send`).

use std::ffi::CStr;
use std::io;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use sdl3_sys::error::SDL_GetError;
use sdl3_sys::events::{
    SDL_Event, SDL_EventType, SDL_PollEvent, SDL_EVENT_GAMEPAD_ADDED, SDL_EVENT_GAMEPAD_AXIS_MOTION, SDL_EVENT_GAMEPAD_BUTTON_DOWN,
    SDL_EVENT_GAMEPAD_BUTTON_UP, SDL_EVENT_GAMEPAD_REMOVED,
};
use sdl3_sys::gamepad::{
    SDL_CloseGamepad, SDL_Gamepad, SDL_GamepadAxis, SDL_GamepadButton, SDL_GetGamepadName, SDL_GetGamepadProduct, SDL_GetGamepadVendor,
    SDL_GetGamepads, SDL_OpenGamepad, SDL_GAMEPAD_AXIS_LEFTX, SDL_GAMEPAD_AXIS_LEFTY, SDL_GAMEPAD_AXIS_LEFT_TRIGGER,
    SDL_GAMEPAD_AXIS_RIGHTX, SDL_GAMEPAD_AXIS_RIGHTY, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, SDL_GAMEPAD_BUTTON_BACK,
    SDL_GAMEPAD_BUTTON_DPAD_DOWN, SDL_GAMEPAD_BUTTON_DPAD_LEFT, SDL_GAMEPAD_BUTTON_DPAD_RIGHT, SDL_GAMEPAD_BUTTON_DPAD_UP,
    SDL_GAMEPAD_BUTTON_EAST, SDL_GAMEPAD_BUTTON_GUIDE, SDL_GAMEPAD_BUTTON_LEFT_SHOULDER, SDL_GAMEPAD_BUTTON_NORTH,
    SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER, SDL_GAMEPAD_BUTTON_SOUTH, SDL_GAMEPAD_BUTTON_START, SDL_GAMEPAD_BUTTON_WEST,
};
use sdl3_sys::hints::{
    SDL_SetHint, SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS, SDL_HINT_JOYSTICK_ENHANCED_REPORTS, SDL_HINT_NO_SIGNAL_HANDLERS,
    SDL_HINT_TIMER_RESOLUTION,
};
use sdl3_sys::init::{SDL_InitSubSystem, SDL_QuitSubSystem, SDL_INIT_GAMEPAD};
use sdl3_sys::joystick::SDL_JoystickID;
use sdl3_sys::stdinc::SDL_free;

/// A button by its position (south is Xbox A, PlayStation cross and
/// Nintendo B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    South,
    East,
    West,
    North,
    /// View, Share, Create, minus.
    Back,
    /// Xbox, PS, Home.
    Guide,
    /// Menu, Options, plus.
    Start,
    LeftShoulder,
    RightShoulder,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

/// What [`Controllers::poll`] reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A controller was plugged in (or turned on) and is open.
    Added {
        name: String,
    },
    /// A controller went away. `remaining` names one that is still open.
    Removed {
        remaining: Option<String>,
    },
    Button(Button, bool),
    /// Sticks span -1..=1 with positive Y up; triggers span 0..=1.
    Axis(Axis, f32),
}

struct Pad {
    id: SDL_JoystickID,
    handle: NonNull<SDL_Gamepad>,
    name: String,
}

/// Whether a [`Controllers`] exists: SDL's subsystems are global.
static OPEN: AtomicBool = AtomicBool::new(false);

/// SDL's controller subsystem with every controller it knows open. Dropping
/// it closes them and shuts the subsystem down.
pub struct Controllers {
    pads: Vec<Pad>,
    /// SDL's event queue belongs to the thread that started it.
    _thread_bound: PhantomData<*const ()>,
}

impl Controllers {
    /// Starts SDL's controller subsystem and opens the controllers already
    /// connected. Fails when another [`Controllers`] exists.
    pub fn open() -> io::Result<Self> {
        if OPEN.swap(true, Ordering::AcqRel) {
            return Err(io::Error::other("SDL controllers are already open in this process"));
        }
        // SAFETY: the hint names and values are NUL-terminated strings that
        // SDL copies; this runs before the subsystem starts.
        unsafe {
            SDL_SetHint(SDL_HINT_JOYSTICK_ENHANCED_REPORTS, c"0".as_ptr());
            // There are no SDL windows: input is not tied to a window's focus
            // (the app checks its own window).
            SDL_SetHint(SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS, c"1".as_ptr());
            // SDL would raise Windows' timer resolution to 1 ms until the
            // process ends (power), and turn Ctrl+C / SIGTERM into its own
            // quit event, which nothing reads here.
            SDL_SetHint(SDL_HINT_TIMER_RESOLUTION, c"0".as_ptr());
            SDL_SetHint(SDL_HINT_NO_SIGNAL_HANDLERS, c"1".as_ptr());
        }
        // SAFETY: plain call; failure is reported through the return value.
        if !unsafe { SDL_InitSubSystem(SDL_INIT_GAMEPAD) } {
            OPEN.store(false, Ordering::Release);
            return Err(sdl_error("starting SDL's controller subsystem"));
        }
        let mut this = Self { pads: Vec::new(), _thread_bound: PhantomData };
        let mut count = 0;
        // SAFETY: `count` is writable; the returned array is freed below.
        let ids = unsafe { SDL_GetGamepads(&mut count) };
        if !ids.is_null() {
            let len = usize::try_from(count).unwrap_or(0);
            // SAFETY: SDL returns `count` ids in an array it allocated.
            let found = unsafe { std::slice::from_raw_parts(ids, len) }.to_vec();
            // SAFETY: the array came from SDL and is not used afterwards.
            unsafe { SDL_free(ids.cast()) };
            for id in found {
                this.add(id);
            }
        }
        Ok(this)
    }

    /// The names of the open controllers, in the order they were opened.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.pads.iter().map(|p| p.name.as_str())
    }

    /// Whether a controller is open (their state has to be read often).
    pub fn any_open(&self) -> bool {
        !self.pads.is_empty()
    }

    /// The next event, reading the controllers first; `None` when there is
    /// nothing new. Never blocks.
    pub fn poll(&mut self) -> Option<Event> {
        loop {
            let mut ev = SDL_Event::default();
            // SAFETY: `ev` is a writable event.
            if !unsafe { SDL_PollEvent(&mut ev) } {
                return None;
            }
            // SAFETY: every event starts with its type.
            let kind = SDL_EventType(unsafe { ev.r#type });
            if kind == SDL_EVENT_GAMEPAD_ADDED {
                // SAFETY: the type says which member is valid.
                let id = unsafe { ev.gdevice.which };
                if let Some(name) = self.add(id) {
                    return Some(Event::Added { name });
                }
            } else if kind == SDL_EVENT_GAMEPAD_REMOVED {
                // SAFETY: as above.
                let id = unsafe { ev.gdevice.which };
                if let Some(ix) = self.pads.iter().position(|p| p.id == id) {
                    let pad = self.pads.remove(ix);
                    log::info!("controller disconnected: {}", pad.name);
                    // SAFETY: the handle came from SDL_OpenGamepad and is closed once.
                    unsafe { SDL_CloseGamepad(pad.handle.as_ptr()) };
                    return Some(Event::Removed { remaining: self.pads.first().map(|p| p.name.clone()) });
                }
            } else if kind == SDL_EVENT_GAMEPAD_BUTTON_DOWN || kind == SDL_EVENT_GAMEPAD_BUTTON_UP {
                // SAFETY: as above.
                let b = unsafe { ev.gbutton };
                if self.is_open(b.which) {
                    if let Some(button) = button(SDL_GamepadButton(b.button.into())) {
                        return Some(Event::Button(button, b.down));
                    }
                }
            } else if kind == SDL_EVENT_GAMEPAD_AXIS_MOTION {
                // SAFETY: as above.
                let a = unsafe { ev.gaxis };
                if self.is_open(a.which) {
                    if let Some(axis) = axis(SDL_GamepadAxis(a.axis.into())) {
                        return Some(Event::Axis(axis, axis_value(axis, a.value)));
                    }
                }
            }
        }
    }

    fn is_open(&self, id: SDL_JoystickID) -> bool {
        self.pads.iter().any(|p| p.id == id)
    }

    /// Opens a controller SDL announced; its name when it was not open yet.
    /// (The ones connected at start are announced again by an event.)
    fn add(&mut self, id: SDL_JoystickID) -> Option<String> {
        if self.is_open(id) {
            return None;
        }
        // SAFETY: plain call; NULL is a failure.
        let Some(handle) = NonNull::new(unsafe { SDL_OpenGamepad(id) }) else {
            log::warn!("controller {}: {}", id.0, sdl_error("opening it"));
            return None;
        };
        // SAFETY: the handle is open; SDL owns the returned string.
        let name = unsafe { c_string(SDL_GetGamepadName(handle.as_ptr())) }.unwrap_or_else(|| "Controller".to_string());
        // SAFETY: the handle is open.
        let (vendor, product) = unsafe { (SDL_GetGamepadVendor(handle.as_ptr()), SDL_GetGamepadProduct(handle.as_ptr())) };
        log::info!("controller connected: {name} ({vendor:04x}:{product:04x})");
        self.pads.push(Pad { id, handle, name: name.clone() });
        Some(name)
    }
}

impl Drop for Controllers {
    fn drop(&mut self) {
        for pad in self.pads.drain(..) {
            // SAFETY: each handle is open and closed once.
            unsafe { SDL_CloseGamepad(pad.handle.as_ptr()) };
        }
        // SAFETY: balances the SDL_InitSubSystem of `open`.
        unsafe { SDL_QuitSubSystem(SDL_INIT_GAMEPAD) };
        OPEN.store(false, Ordering::Release);
    }
}

fn button(b: SDL_GamepadButton) -> Option<Button> {
    Some(match b {
        SDL_GAMEPAD_BUTTON_SOUTH => Button::South,
        SDL_GAMEPAD_BUTTON_EAST => Button::East,
        SDL_GAMEPAD_BUTTON_WEST => Button::West,
        SDL_GAMEPAD_BUTTON_NORTH => Button::North,
        SDL_GAMEPAD_BUTTON_BACK => Button::Back,
        SDL_GAMEPAD_BUTTON_GUIDE => Button::Guide,
        SDL_GAMEPAD_BUTTON_START => Button::Start,
        SDL_GAMEPAD_BUTTON_LEFT_SHOULDER => Button::LeftShoulder,
        SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER => Button::RightShoulder,
        SDL_GAMEPAD_BUTTON_DPAD_UP => Button::DPadUp,
        SDL_GAMEPAD_BUTTON_DPAD_DOWN => Button::DPadDown,
        SDL_GAMEPAD_BUTTON_DPAD_LEFT => Button::DPadLeft,
        SDL_GAMEPAD_BUTTON_DPAD_RIGHT => Button::DPadRight,
        _ => return None,
    })
}

fn axis(a: SDL_GamepadAxis) -> Option<Axis> {
    Some(match a {
        SDL_GAMEPAD_AXIS_LEFTX => Axis::LeftX,
        SDL_GAMEPAD_AXIS_LEFTY => Axis::LeftY,
        SDL_GAMEPAD_AXIS_RIGHTX => Axis::RightX,
        SDL_GAMEPAD_AXIS_RIGHTY => Axis::RightY,
        SDL_GAMEPAD_AXIS_LEFT_TRIGGER => Axis::LeftTrigger,
        SDL_GAMEPAD_AXIS_RIGHT_TRIGGER => Axis::RightTrigger,
        _ => return None,
    })
}

/// SDL's axes span -32768..=32767 with positive Y down; triggers 0..=32767.
fn axis_value(axis: Axis, raw: i16) -> f32 {
    let v = (f32::from(raw) / 32767.0).clamp(-1.0, 1.0);
    match axis {
        Axis::LeftY | Axis::RightY => -v,
        Axis::LeftTrigger | Axis::RightTrigger => v.max(0.0),
        Axis::LeftX | Axis::RightX => v,
    }
}

/// # Safety
/// `s` is NULL or a NUL-terminated string that lives for the call.
unsafe fn c_string(s: *const std::ffi::c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees a NUL-terminated string.
    let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn sdl_error(what: &str) -> io::Error {
    // SAFETY: SDL_GetError returns a valid (possibly empty) string.
    let detail = unsafe { c_string(SDL_GetError()) }.unwrap_or_else(|| "unknown SDL error".to_string());
    io::Error::other(format!("{what}: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_point_up_and_triggers_are_positive() {
        assert_eq!(axis_value(Axis::LeftY, -32768), 1.0);
        assert_eq!(axis_value(Axis::RightY, 32767), -1.0);
        assert_eq!(axis_value(Axis::LeftX, -32768), -1.0);
        assert_eq!(axis_value(Axis::RightX, 32767), 1.0);
        assert_eq!(axis_value(Axis::LeftTrigger, 32767), 1.0);
        assert_eq!(axis_value(Axis::RightTrigger, 0), 0.0);
        assert_eq!(axis_value(Axis::LeftX, 0), 0.0);
    }

    /// Every event within a moment (SDL notices virtual controllers and
    /// their changes on its next reads).
    fn drain(pads: &mut Controllers) -> Vec<Event> {
        let mut out = Vec::new();
        for _ in 0..20 {
            while let Some(ev) = pads.poll() {
                out.push(ev);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out
    }

    /// One test: SDL is global and a second owner is refused, so tests that
    /// open it cannot run side by side.
    #[test]
    fn reads_a_controller_and_opens_once_at_a_time() {
        use sdl3_sys::gamepad::{SDL_GAMEPAD_AXIS_COUNT, SDL_GAMEPAD_BUTTON_COUNT};
        use sdl3_sys::joystick::{
            SDL_AttachVirtualJoystick, SDL_DetachVirtualJoystick, SDL_GetJoystickFromID, SDL_SetJoystickVirtualAxis,
            SDL_SetJoystickVirtualButton, SDL_VirtualJoystickDesc, SDL_JOYSTICK_TYPE_GAMEPAD,
        };

        let mut pads = Controllers::open().expect("SDL's controller subsystem starts");
        assert!(Controllers::open().is_err(), "a second owner is refused");
        // The controllers of this machine, if any.
        drain(&mut pads);
        let before = pads.pads.len();

        let desc = SDL_VirtualJoystickDesc {
            r#type: SDL_JOYSTICK_TYPE_GAMEPAD.0 as u16,
            naxes: SDL_GAMEPAD_AXIS_COUNT.0 as u16,
            nbuttons: SDL_GAMEPAD_BUTTON_COUNT.0 as u16,
            name: c"Heronite test pad".as_ptr(),
            ..Default::default()
        };
        // SAFETY: `desc` is a valid description that SDL copies.
        let id = unsafe { SDL_AttachVirtualJoystick(&desc) };
        assert_ne!(id.0, 0, "{}", sdl_error("attaching a virtual controller"));
        let added = drain(&mut pads);
        assert!(added.iter().any(|e| matches!(e, Event::Added { name } if !name.is_empty())), "{added:?}");
        assert_eq!(pads.pads.len(), before + 1);
        assert!(pads.any_open());

        // SAFETY: the virtual controller is attached and open (Controllers opened it).
        let joystick = unsafe { SDL_GetJoystickFromID(id) };
        assert!(!joystick.is_null());
        // SAFETY: `joystick` is open; the indices are the gamepad's own.
        unsafe {
            SDL_SetJoystickVirtualButton(joystick, SDL_GAMEPAD_BUTTON_SOUTH.0, true);
            SDL_SetJoystickVirtualAxis(joystick, SDL_GAMEPAD_AXIS_LEFTY.0, i16::MIN);
            SDL_SetJoystickVirtualAxis(joystick, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER.0, i16::MAX);
        }
        let moved = drain(&mut pads);
        assert!(moved.contains(&Event::Button(Button::South, true)), "{moved:?}");
        // Pushed up: positive.
        assert!(moved.contains(&Event::Axis(Axis::LeftY, 1.0)), "{moved:?}");
        assert!(moved.contains(&Event::Axis(Axis::RightTrigger, 1.0)), "{moved:?}");
        // SAFETY: as above.
        unsafe { SDL_SetJoystickVirtualButton(joystick, SDL_GAMEPAD_BUTTON_SOUTH.0, false) };
        let released = drain(&mut pads);
        assert!(released.contains(&Event::Button(Button::South, false)), "{released:?}");

        // SAFETY: `id` is the attached virtual controller.
        assert!(unsafe { SDL_DetachVirtualJoystick(id) });
        let removed = drain(&mut pads);
        assert!(removed.iter().any(|e| matches!(e, Event::Removed { .. })), "{removed:?}");
        assert_eq!(pads.pads.len(), before);

        drop(pads);
        let again = Controllers::open().expect("SDL restarts after being closed");
        drop(again);
    }

    #[test]
    fn buttons_and_axes_follow_positions() {
        assert_eq!(button(SDL_GAMEPAD_BUTTON_SOUTH), Some(Button::South));
        assert_eq!(button(SDL_GAMEPAD_BUTTON_EAST), Some(Button::East));
        assert_eq!(button(SDL_GAMEPAD_BUTTON_GUIDE), Some(Button::Guide));
        assert_eq!(button(SDL_GAMEPAD_BUTTON_DPAD_LEFT), Some(Button::DPadLeft));
        assert_eq!(button(SDL_GamepadButton(-1)), None);
        assert_eq!(axis(SDL_GAMEPAD_AXIS_RIGHT_TRIGGER), Some(Axis::RightTrigger));
        assert_eq!(axis(SDL_GamepadAxis(-1)), None);
    }
}
