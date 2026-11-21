//! The PIN of hidden games: typed to show them, set, changed or turned off
//! (`heronite_data::hidden_lock`, session state in
//! `services::hidden_lock`).
//!
//! One pad of digits for the mouse, the keyboard (digits, Backspace, Enter;
//! offered by the Collection through `dialogs::handle_key`)
//! and a controller (the d-pad moves over the keys, A presses one, X deletes,
//! Start or Y goes on, B cancels). The hash is worked out off the UI thread.
//! Five wrong PINs in a row make the next try wait (30 s, doubling).

mod render;

use std::time::Duration;

use gpui_kit::*;
use heronite_data::{hidden_lock, AppPaths};
use heronite_i18n::tr;

use super::DialogEvent;
use crate::services::hidden_lock as session;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinMode {
    /// Typed before hidden games show.
    Unlock,
    /// A first PIN.
    Set,
    Change,
    /// Turns the PIN off.
    Remove,
}

impl PinMode {
    /// `HERONITE_DEBUG_DIALOG=hidden-pin:<unlock|set|change|remove>`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unlock" => Self::Unlock,
            "set" => Self::Set,
            "change" => Self::Change,
            "remove" => Self::Remove,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    /// The saved PIN, checked.
    Current,
    New,
    /// The new PIN once more.
    Confirm,
}

/// The dialog did what its mode asks (the PIN was right, saved or removed);
/// it closes right after.
pub struct PinDone(pub PinMode);

/// The pad: 1-9, then an empty place, 0 and delete.
const KEYS: usize = 12;
const EMPTY_KEY: usize = 9;
const ZERO_KEY: usize = 10;
const DELETE_KEY: usize = 11;

pub struct HiddenPinDialog {
    mode: PinMode,
    step: Step,
    entry: String,
    new_pin: String,
    error: Option<SharedString>,
    /// A hash is being worked out: the pad waits.
    checking: bool,
    app: AppPaths,
    /// The key under the controller's cursor.
    key: usize,
    _tick: Option<Task<()>>,
}

impl EventEmitter<DialogEvent> for HiddenPinDialog {}
impl EventEmitter<PinDone> for HiddenPinDialog {}

impl HiddenPinDialog {
    pub fn new(mode: PinMode, app: AppPaths, cx: &mut Context<Self>) -> Self {
        let step = if mode == PinMode::Set { Step::New } else { Step::Current };
        let mut this = Self {
            mode,
            step,
            entry: String::new(),
            new_pin: String::new(),
            error: None,
            checking: false,
            app,
            key: 4,
            _tick: None,
        };
        if step == Step::Current && session::wait_left().is_some() {
            this.start_countdown(cx);
        }
        this.debug_typing(cx);
        this
    }

    /// `HERONITE_DEBUG_PIN=<digits>[,enter]…`: types those digits (Enter
    /// goes on), for captures of each step.
    fn debug_typing(&mut self, cx: &mut Context<Self>) {
        let Some(keys) = crate::env::value("HERONITE_DEBUG_PIN") else { return };
        let steps: Vec<String> = keys.split(',').map(str::to_owned).collect();
        cx.spawn(async move |this, cx| {
            for step in steps {
                cx.background_executor().timer(Duration::from_millis(600)).await;
                let ok = this.update(cx, |this, cx| {
                    if step == "enter" {
                        this.submit(cx);
                    } else {
                        step.chars().for_each(|c| this.type_digit(c, cx));
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn title(&self) -> SharedString {
        match self.mode {
            PinMode::Unlock => tr!("Show hidden games"),
            PinMode::Set => tr!("Set a PIN"),
            PinMode::Change => tr!("Change the PIN"),
            PinMode::Remove => tr!("Turn off the PIN"),
        }
        .into()
    }

    fn prompt(&self) -> SharedString {
        match (self.step, self.mode) {
            (Step::Current, PinMode::Unlock) => tr!("Enter your PIN to show the hidden games."),
            (Step::Current, PinMode::Remove) => tr!("Enter your PIN. Hidden games then show with the switch alone."),
            (Step::Current, _) => tr!("Enter your current PIN."),
            (Step::New, PinMode::Set) => tr!("Choose a PIN of 4 to 8 digits. Heronite asks for it before it shows hidden games."),
            (Step::New, _) => tr!("Enter the new PIN, 4 to 8 digits."),
            (Step::Confirm, _) => tr!("Enter the PIN again to confirm it."),
        }
        .into()
    }

    /// What going on does at this step.
    fn primary_label(&self) -> SharedString {
        match (self.step, self.mode) {
            (Step::Current, PinMode::Unlock) => tr!("Unlock"),
            (Step::Current, PinMode::Remove) => tr!("Turn off"),
            (Step::Confirm, _) => tr!("Save PIN"),
            _ => tr!("Next"),
        }
        .into()
    }

    /// The pad takes no digits: a hash is being worked out, or a wrong PIN
    /// made it wait.
    fn waiting(&self) -> bool {
        self.checking || (self.step == Step::Current && session::wait_left().is_some())
    }

    fn type_digit(&mut self, c: char, cx: &mut Context<Self>) {
        if self.waiting() || !c.is_ascii_digit() || self.entry.len() >= *hidden_lock::PIN_LEN.end() {
            return;
        }
        self.entry.push(c);
        self.error = None;
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if !self.waiting() && self.entry.pop().is_some() {
            cx.notify();
        }
    }

    fn press(&mut self, key: usize, cx: &mut Context<Self>) {
        match key {
            0..=8 => self.type_digit(char::from(b'1' + key as u8), cx),
            ZERO_KEY => self.type_digit('0', cx),
            DELETE_KEY => self.delete(cx),
            _ => {}
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.waiting() {
            return;
        }
        let pin = std::mem::take(&mut self.entry);
        if !hidden_lock::valid_pin(&pin) {
            self.entry = pin;
            self.error = Some(tr!("A PIN has 4 to 8 digits.").into());
            cx.notify();
            return;
        }
        match self.step {
            Step::Current => self.check(pin, cx),
            Step::New => {
                self.new_pin = pin;
                self.step = Step::Confirm;
                self.error = None;
            }
            Step::Confirm if pin != self.new_pin => {
                self.new_pin.clear();
                self.step = Step::New;
                self.error = Some(tr!("The two PINs differ. Choose the new PIN again.").into());
            }
            Step::Confirm => self.save(pin, cx),
        }
        cx.notify();
    }

    /// Checks the saved PIN off the UI thread, then goes on (or counts a
    /// wrong try).
    fn check(&mut self, pin: String, cx: &mut Context<Self>) {
        self.checking = true;
        let app = self.app.clone();
        cx.spawn(async move |this, cx| {
            let right = cx.background_spawn(async move { hidden_lock::load(&app).is_some_and(|p| p.matches(&pin)) }).await;
            this.update(cx, |this, cx| {
                this.checking = false;
                if right {
                    session::succeeded();
                    match this.mode {
                        PinMode::Unlock => this.finish(cx),
                        PinMode::Remove => this.remove(cx),
                        _ => {
                            this.step = Step::New;
                            this.error = None;
                        }
                    }
                } else {
                    // Past the free tries the wait line replaces this one.
                    this.error = Some(tr!("Wrong PIN. Try again.").into());
                    if !session::failed().is_zero() {
                        this.start_countdown(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, pin: String, cx: &mut Context<Self>) {
        let app = self.app.clone();
        self.run(move || hidden_lock::set(&app, &pin).map(|_| session::note_pin_set(true)), cx);
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let app = self.app.clone();
        self.run(move || hidden_lock::clear(&app).map(|_| session::note_pin_set(false)), cx);
    }

    /// Writes the PIN file off the UI thread, then closes (or says why not).
    fn run(&mut self, job: impl FnOnce() -> anyhow::Result<()> + Send + 'static, cx: &mut Context<Self>) {
        self.checking = true;
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { job() }).await;
            this.update(cx, |this, cx| {
                this.checking = false;
                match res {
                    Ok(()) => this.finish(cx),
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        cx.emit(PinDone(self.mode));
        cx.emit(DialogEvent::Close);
    }

    /// Redraws every second while a wrong PIN makes the pad wait.
    fn start_countdown(&mut self, cx: &mut Context<Self>) {
        self._tick = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let more = this
                .update(cx, |this, cx| {
                    let more = session::wait_left().is_some();
                    if !more {
                        this.error = None;
                    }
                    cx.notify();
                    more
                })
                .unwrap_or(false);
            if !more {
                break;
            }
        }));
    }

    /// The text under the dots while the pad waits (`None` otherwise).
    fn wait_text(&self) -> Option<SharedString> {
        let left = session::wait_left().filter(|_| self.step == Step::Current)?;
        Some(tr!("Too many wrong PINs. Try again in {seconds} s.", seconds = left.as_secs() + 1).into())
    }
}

/// The key the controller's cursor goes to from `key`: the pad has 3 columns
/// and 4 rows, and the empty place is stepped over.
fn next_key(key: usize, pad: crate::console::Pad) -> usize {
    use crate::console::Pad;
    let (row, col) = (key / 3, key % 3);
    let next = match pad {
        Pad::Left => row * 3 + (col + 2) % 3,
        Pad::Right => row * 3 + (col + 1) % 3,
        Pad::Up => ((row + 3) % 4) * 3 + col,
        Pad::Down => ((row + 1) % 4) * 3 + col,
        _ => key,
    };
    if next != EMPTY_KEY {
        return next;
    }
    match pad {
        Pad::Left => DELETE_KEY,
        Pad::Right => ZERO_KEY,
        Pad::Up => 3,
        _ => 0,
    }
}

impl crate::console::PadTarget for HiddenPinDialog {
    fn on_pad(&mut self, pad: crate::console::Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::console::Pad;
        match pad {
            Pad::Up | Pad::Down | Pad::Left | Pad::Right => {
                self.key = next_key(self.key, pad);
                cx.notify();
            }
            Pad::A => self.press(self.key, cx),
            Pad::X => self.delete(cx),
            Pad::Start | Pad::Y => self.submit(cx),
            Pad::B => cx.emit(DialogEvent::Close),
            _ => return false,
        }
        true
    }

    fn hints(&self, _: &App) -> Vec<crate::console::Hint> {
        use crate::console::{hint, Btn};
        vec![hint(Btn::A, tr!("Type")), hint(Btn::X, tr!("Delete")), hint(Btn::Start, self.primary_label()), hint(Btn::B, tr!("Cancel"))]
    }
}

#[cfg(test)]
mod tests {
    use super::{next_key, PinMode, DELETE_KEY, EMPTY_KEY, KEYS, ZERO_KEY};
    use crate::console::Pad;

    #[test]
    fn the_cursor_never_rests_on_the_empty_place() {
        for key in (0..KEYS).filter(|k| *k != EMPTY_KEY) {
            for pad in [Pad::Up, Pad::Down, Pad::Left, Pad::Right] {
                assert_ne!(next_key(key, pad), EMPTY_KEY, "{key} {pad:?}");
            }
        }
        assert_eq!(next_key(ZERO_KEY, Pad::Left), DELETE_KEY);
        assert_eq!(next_key(DELETE_KEY, Pad::Right), ZERO_KEY);
        assert_eq!(next_key(6, Pad::Down), 0);
        assert_eq!(next_key(0, Pad::Up), 3);
    }

    #[test]
    fn debug_modes_parse() {
        assert_eq!(PinMode::parse("unlock"), Some(PinMode::Unlock));
        assert_eq!(PinMode::parse("remove"), Some(PinMode::Remove));
        assert_eq!(PinMode::parse("x"), None);
    }
}
