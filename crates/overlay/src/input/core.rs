//! What the Guide button does, without devices: pure, so it is tested
//! without a controller. Input backends feed it each controller's presses;
//! only evdev can apply the grabs it asks for.
//!
//! - Guide down grabs that controller at once, so what follows (RB, ↑ ↓)
//!   reaches only us, not the game.
//! - Guide + RB (or the button chosen) is a screenshot, Guide + ↑ / ↓ a
//!   volume step, Guide + Y a moment for the journal, Guide + X the replay
//!   saved. Any other button while Guide is held makes it a chord too: no
//!   overlay on release.
//! - Guide released without a chord opens the overlay (every controller is
//!   grabbed while it is open) or closes it.
//! - While open, the presses become navigation, with held directions and the
//!   left stick repeating.
//! - With [`Options::console_hold`] (no game runs), Guide held alone for
//!   [`CONSOLE_HOLD`] while closed asks for console mode instead
//!   ([`Event::ConsoleMode`]); its release then opens nothing. A tap still
//!   opens the overlay.

use std::time::{Duration, Instant};

/// A controller button, after mapping the kernel codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    Start,
    Select,
    Up,
    Down,
    Left,
    Right,
    Guide,
}

impl From<heronite_data::overlay_prefs::ChordButton> for Button {
    fn from(b: heronite_data::overlay_prefs::ChordButton) -> Self {
        use heronite_data::overlay_prefs::ChordButton as C;
        match b {
            C::RB => Button::RB,
            C::LB => Button::LB,
            C::A => Button::A,
            C::B => Button::B,
            C::X => Button::X,
            C::Y => Button::Y,
            C::Left => Button::Left,
            C::Right => Button::Right,
            C::Start => Button::Start,
            C::Select => Button::Select,
        }
    }
}

impl Button {
    fn is_direction(self) -> bool {
        matches!(self, Button::Up | Button::Down | Button::Left | Button::Right)
    }
}

/// How long Guide is held for console mode ([`Options::console_hold`]).
pub const CONSOLE_HOLD: Duration = Duration::from_millis(1000);

/// What the overlay hears.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Guide tapped while closed: the overlay should open (the controllers
    /// are held from now on).
    Open,
    /// Guide tapped while open (or every controller went away): close it.
    Close,
    /// Guide + the screenshot button.
    Screenshot,
    /// Guide + ↑ (`1`) or ↓ (`-1`).
    Volume(i8),
    /// Guide + Y: mark this moment in the game's journal.
    Moment,
    /// Guide + X: save the replay (the last seconds of the game).
    Replay,
    /// A press while open (repeats included).
    Press(Button),
    /// The connected controllers changed.
    Controllers(Vec<super::Controller>),
    /// Guide went down while closed, with [`Options::console_hold`]: held
    /// until [`CONSOLE_HOLD`] after this, it is console mode.
    ConsoleHolding(Instant),
    /// That Guide came up early, or became a chord.
    ConsoleCancelled,
    /// Held for [`CONSOLE_HOLD`]: enter console mode (its release opens nothing).
    ConsoleMode,
}

/// Grab changes the thread applies (device ids are the thread's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    Hold(usize),
    HoldAll,
    ReleaseAll,
}

/// Which shortcuts work without opening the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub screenshot: bool,
    /// Held with Guide for a screenshot (RB unless changed).
    pub screenshot_button: Button,
    pub volume: bool,
    /// Guide + Y marks a moment (unless Y is the screenshot button).
    pub moment: bool,
    /// Guide + X saves the replay (unless X is the screenshot button).
    pub replay: bool,
    /// Guide held alone enters console mode ([`Event::ConsoleMode`]); the
    /// app turns it on while no game runs.
    pub console_hold: bool,
}

/// First repeat after this long, then every [`REPEAT_EVERY`] (same pace as
/// console mode).
const REPEAT_AFTER: Duration = Duration::from_millis(380);
const REPEAT_EVERY: Duration = Duration::from_millis(85);
/// Stick deflection that counts as a press, and the one that releases it.
const STICK_ON: f32 = 0.55;
const STICK_OFF: f32 = 0.35;

#[derive(Default)]
pub(super) struct Output {
    pub events: Vec<Event>,
    pub grabs: Vec<Grab>,
}

pub(super) struct Core {
    pub opts: Options,
    open: bool,
    /// The controller whose Guide is down, and whether a chord was used.
    guide: Option<(usize, bool)>,
    /// The held direction and when it repeats next.
    held: Option<(usize, Button, Instant)>,
    /// Where the left stick points (past the threshold).
    stick: Option<(usize, Button)>,
    /// When the held Guide went down, while it may still be console mode.
    console: Option<Instant>,
}

impl Core {
    pub fn new(opts: Options) -> Self {
        Self { opts, open: false, guide: None, held: None, stick: None, console: None }
    }

    /// The Guide held for console mode will not be: tell the app.
    fn cancel_console(&mut self, out: &mut Output) {
        if self.console.take().is_some() {
            out.events.push(Event::ConsoleCancelled);
        }
    }

    #[cfg(any(windows, test))]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether `dev` must stay grabbed (its Guide is down or the overlay is open).
    #[cfg(any(target_os = "linux", test))]
    pub fn holds(&self, dev: usize) -> bool {
        self.open || self.guide.is_some_and(|(d, _)| d == dev)
    }

    /// A button of controller `dev` went down (`true`) or up.
    pub fn button(&mut self, dev: usize, b: Button, down: bool, now: Instant) -> Output {
        let mut out = Output::default();
        if b == Button::Guide {
            self.guide_button(dev, down, now, &mut out);
            return out;
        }
        if down && self.guide.is_some() {
            // A chord, not console mode.
            self.cancel_console(&mut out);
        }
        if let Some((gdev, used)) = &mut self.guide {
            // A chord: Guide is held (on any controller).
            if down {
                *used = true;
                match b {
                    b if self.opts.screenshot && b == self.opts.screenshot_button => out.events.push(Event::Screenshot),
                    Button::Up if self.opts.volume => out.events.push(Event::Volume(1)),
                    Button::Down if self.opts.volume => out.events.push(Event::Volume(-1)),
                    Button::Y if self.opts.moment => out.events.push(Event::Moment),
                    Button::X if self.opts.replay => out.events.push(Event::Replay),
                    _ => {}
                }
                // A chord on another controller grabs it too, so the game
                // does not get the rest of it.
                if *gdev != dev && !self.open {
                    out.grabs.push(Grab::Hold(dev));
                }
            }
            return out;
        }
        if !self.open {
            return out;
        }
        if down {
            if b.is_direction() {
                self.held = Some((dev, b, now + REPEAT_AFTER));
            }
            out.events.push(Event::Press(b));
        } else if self.held.is_some_and(|(d, h, _)| d == dev && h == b) {
            self.held = None;
        }
        out
    }

    fn guide_button(&mut self, dev: usize, down: bool, now: Instant, out: &mut Output) {
        if down {
            if self.guide.is_none() {
                self.guide = Some((dev, false));
                self.held = None;
                // Even while open (where it is normally held already): a grab
                // that fails tells that another program owns the controller.
                out.grabs.push(Grab::Hold(dev));
                if self.opts.console_hold && !self.open {
                    self.console = Some(now);
                    out.events.push(Event::ConsoleHolding(now));
                }
            }
            return;
        }
        let Some((gdev, used)) = self.guide else { return };
        if gdev != dev {
            return;
        }
        self.guide = None;
        self.cancel_console(out);
        if used {
            if !self.open {
                out.grabs.push(Grab::ReleaseAll);
            }
            return;
        }
        if self.open {
            self.set_closed(out);
        } else {
            self.open = true;
            self.held = None;
            self.stick = None;
            out.events.push(Event::Open);
            out.grabs.push(Grab::HoldAll);
        }
    }

    fn set_closed(&mut self, out: &mut Output) {
        self.open = false;
        self.held = None;
        self.stick = None;
        out.events.push(Event::Close);
        if self.guide.is_none() {
            out.grabs.push(Grab::ReleaseAll);
        }
    }

    /// The overlay closed itself (B, an action): let the game have the
    /// controllers again. No [`Event::Close`]: the UI knows.
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        if self.open {
            self.set_closed(&mut out);
            out.events.clear();
        }
        out
    }

    /// The UI opened the overlay by itself (the keyboard shortcut, a debug
    /// hook): hold the controllers.
    pub fn open(&mut self) -> Output {
        let mut out = Output::default();
        if !self.open {
            self.open = true;
            out.grabs.push(Grab::HoldAll);
        }
        out
    }

    /// The controller whose Guide is held, as far as the core knows.
    #[cfg(any(target_os = "linux", test))]
    pub fn guide_device(&self) -> Option<usize> {
        self.guide.map(|(d, _)| d)
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn guide_held_on(&self, dev: usize) -> bool {
        self.guide_device() == Some(dev)
    }

    /// The kernel dropped events of `dev`: a held direction may have been
    /// released unseen, so it stops repeating.
    #[cfg(any(target_os = "linux", test))]
    pub fn dropped(&mut self, dev: usize) {
        if self.held.is_some_and(|(d, _, _)| d == dev) {
            self.held = None;
        }
        if self.stick.is_some_and(|(d, _)| d == dev) {
            self.stick = None;
        }
    }

    /// The Guide release of `dev` will never come: another program grabbed
    /// the controller first (it gets the release, not us), or the kernel
    /// dropped events. Forget the Guide without opening the overlay, or every
    /// later press would be read as a chord (RB a screenshot, X a replay).
    #[cfg(any(target_os = "linux", test))]
    pub fn lost_guide(&mut self, dev: usize) -> Output {
        let mut out = Output::default();
        if self.guide_held_on(dev) {
            self.guide = None;
            self.cancel_console(&mut out);
            if !self.open {
                out.grabs.push(Grab::ReleaseAll);
            }
        }
        out
    }

    /// A controller went away.
    pub fn removed(&mut self, dev: usize, remaining: usize) -> Output {
        let mut out = Output::default();
        if self.guide.is_some_and(|(d, _)| d == dev) {
            self.guide = None;
            self.cancel_console(&mut out);
            // A chord may have grabbed other controllers: the game gets them back.
            if !self.open {
                out.grabs.push(Grab::ReleaseAll);
            }
        }
        if self.held.is_some_and(|(d, _, _)| d == dev) {
            self.held = None;
        }
        if self.stick.is_some_and(|(d, _)| d == dev) {
            self.stick = None;
        }
        if self.open && remaining == 0 {
            self.set_closed(&mut out);
        }
        out
    }

    /// Cancel navigation when the UI changes modes. A held Guide keeps its
    /// chords, but its release must not reopen (or close) the panel the UI
    /// just changed, so it counts as used.
    #[cfg(any(windows, test))]
    pub fn reset_input(&mut self) {
        if let Some((_, used)) = &mut self.guide {
            *used = true;
        }
        // The app moved on: it drops a console hold on its own.
        self.console = None;
        self.held = None;
        self.stick = None;
    }

    /// The left stick of `dev` moved (`x`, `y` in -1..=1, `y` up). While Guide
    /// is held, up / down count as the volume chord too.
    pub fn stick(&mut self, dev: usize, x: f32, y: f32, now: Instant) -> Output {
        let threshold = |wanted: Button| if self.stick == Some((dev, wanted)) { STICK_OFF } else { STICK_ON };
        let dir = if x.abs() > y.abs() {
            if x >= threshold(Button::Right) {
                Some(Button::Right)
            } else if x <= -threshold(Button::Left) {
                Some(Button::Left)
            } else {
                None
            }
        } else if y >= threshold(Button::Up) {
            Some(Button::Up)
        } else if y <= -threshold(Button::Down) {
            Some(Button::Down)
        } else {
            None
        };
        let own_stick = self.stick.filter(|(d, _)| *d == dev).map(|(_, b)| b);
        if dir == own_stick {
            return Output::default();
        }
        let old = std::mem::replace(&mut self.stick, dir.map(|b| (dev, b)));
        let mut out = Output::default();
        if let Some((old_dev, o)) = old {
            out = self.button(old_dev, o, false, now);
        }
        if let Some(d) = dir {
            let more = self.button(dev, d, true, now);
            out.events.extend(more.events);
            out.grabs.extend(more.grabs);
        }
        out
    }

    /// How long the thread may sleep before [`Core::tick`] has a repeat.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        let repeat = self.held.map(|(_, _, at)| at.saturating_duration_since(now));
        let console = self.console.map(|since| (since + CONSOLE_HOLD).saturating_duration_since(now));
        match (repeat, console) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The repeat due at `now`, or console mode once Guide was held long
    /// enough.
    pub fn tick(&mut self, now: Instant) -> Option<Event> {
        if let Some(since) = self.console {
            if now.saturating_duration_since(since) >= CONSOLE_HOLD {
                self.console = None;
                // Its release opens nothing.
                if let Some((_, used)) = &mut self.guide {
                    *used = true;
                }
                return Some(Event::ConsoleMode);
            }
        }
        let (dev, b, at) = self.held?;
        if now < at || !self.open {
            return None;
        }
        self.held = Some((dev, b, now + REPEAT_EVERY));
        Some(Event::Press(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Options =
        Options { screenshot: true, screenshot_button: Button::RB, volume: true, moment: true, replay: true, console_hold: false };

    fn press(core: &mut Core, dev: usize, b: Button) -> Output {
        let now = Instant::now();
        let mut out = core.button(dev, b, true, now);
        let up = core.button(dev, b, false, now);
        out.events.extend(up.events);
        out.grabs.extend(up.grabs);
        out
    }

    #[test]
    fn guide_tap_opens_and_closes() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        let down = c.button(0, Button::Guide, true, now);
        assert_eq!(down.grabs, vec![Grab::Hold(0)], "grabbed as soon as Guide is down");
        assert!(down.events.is_empty());
        let up = c.button(0, Button::Guide, false, now);
        assert_eq!(up.events, vec![Event::Open]);
        assert_eq!(up.grabs, vec![Grab::HoldAll]);
        assert!(c.is_open() && c.holds(3));

        let again = press(&mut c, 0, Button::Guide);
        assert_eq!(again.events, vec![Event::Close]);
        assert_eq!(again.grabs, vec![Grab::Hold(0), Grab::ReleaseAll], "Guide down asks for the grab even while open");
        assert!(!c.is_open() && !c.holds(0));
    }

    #[test]
    fn chords_do_not_open_the_overlay() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.button(0, Button::RB, true, now).events, vec![Event::Screenshot]);
        assert_eq!(c.button(0, Button::Up, true, now).events, vec![Event::Volume(1)]);
        assert_eq!(c.button(0, Button::Down, true, now).events, vec![Event::Volume(-1)]);
        let up = c.button(0, Button::Guide, false, now);
        assert!(up.events.is_empty(), "a chord never opens the overlay");
        assert_eq!(up.grabs, vec![Grab::ReleaseAll]);
        assert!(!c.is_open());
    }

    #[test]
    fn the_screenshot_button_can_change() {
        let mut c = Core::new(Options { screenshot_button: Button::Y, ..ALL });
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert!(c.button(0, Button::RB, true, now).events.is_empty(), "RB is a plain chord now");
        assert_eq!(c.button(0, Button::Y, true, now).events, vec![Event::Screenshot]);
        assert!(c.button(0, Button::Guide, false, now).events.is_empty());
        assert!(!c.is_open());
    }

    #[test]
    fn guide_y_marks_a_moment_unless_y_takes_the_screenshot() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.button(0, Button::Y, true, now).events, vec![Event::Moment]);
        assert!(c.button(0, Button::Guide, false, now).events.is_empty());

        let mut c = Core::new(Options { screenshot_button: Button::Y, ..ALL });
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.button(0, Button::Y, true, now).events, vec![Event::Screenshot]);
    }

    #[test]
    fn guide_x_saves_the_replay_unless_x_takes_the_screenshot() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.button(0, Button::X, true, now).events, vec![Event::Replay]);
        assert!(c.button(0, Button::Guide, false, now).events.is_empty(), "a chord never opens the overlay");

        let mut c = Core::new(Options { screenshot_button: Button::X, ..ALL });
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.button(0, Button::X, true, now).events, vec![Event::Screenshot]);

        let mut c = Core::new(Options { replay: false, ..ALL });
        c.button(0, Button::Guide, true, now);
        assert!(c.button(0, Button::X, true, now).events.is_empty());
    }

    #[test]
    fn disabled_shortcuts_still_swallow_the_chord() {
        let mut c = Core::new(Options {
            screenshot: false,
            screenshot_button: Button::RB,
            volume: false,
            moment: false,
            replay: false,
            console_hold: false,
        });
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert!(c.button(0, Button::RB, true, now).events.is_empty());
        assert!(c.button(0, Button::Guide, false, now).events.is_empty());
        assert!(!c.is_open());
    }

    #[test]
    fn presses_only_count_while_open() {
        let mut c = Core::new(ALL);
        assert!(press(&mut c, 0, Button::A).events.is_empty(), "closed: the game's");
        press(&mut c, 0, Button::Guide);
        assert_eq!(press(&mut c, 0, Button::A).events, vec![Event::Press(Button::A)]);
        assert_eq!(press(&mut c, 1, Button::LB).events, vec![Event::Press(Button::LB)], "every controller drives it");
    }

    #[test]
    fn held_directions_repeat() {
        let mut c = Core::new(ALL);
        let t0 = Instant::now();
        c.open();
        assert_eq!(c.button(0, Button::Down, true, t0).events, vec![Event::Press(Button::Down)]);
        assert_eq!(c.tick(t0 + Duration::from_millis(100)), None);
        assert_eq!(c.tick(t0 + REPEAT_AFTER), Some(Event::Press(Button::Down)));
        c.button(0, Button::Down, false, t0);
        assert_eq!(c.tick(t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn stick_moves_with_hysteresis() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.open();
        assert_eq!(c.stick(0, 0., 0.6, now).events, vec![Event::Press(Button::Up)]);
        assert!(c.stick(0, 0., 0.4, now).events.is_empty(), "still past the release threshold");
        assert!(c.stick(0, 0., 0.1, now).events.is_empty());
        assert_eq!(c.stick(0, -0.9, 0., now).events, vec![Event::Press(Button::Left)]);
    }

    #[test]
    fn ui_close_releases_without_an_event() {
        let mut c = Core::new(ALL);
        press(&mut c, 0, Button::Guide);
        let out = c.close();
        assert!(out.events.is_empty());
        assert_eq!(out.grabs, vec![Grab::ReleaseAll]);
        assert!(!c.is_open());
    }

    #[test]
    fn a_lost_guide_release_does_not_leave_chords_behind() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert!(c.guide_held_on(0) && !c.guide_held_on(1));
        assert!(c.lost_guide(1).grabs.is_empty(), "another controller's Guide is not this one");
        let out = c.lost_guide(0);
        assert!(out.events.is_empty(), "never opens the overlay");
        assert_eq!(out.grabs, vec![Grab::ReleaseAll]);
        assert!(!c.holds(0));
        assert!(press(&mut c, 0, Button::RB).events.is_empty(), "RB alone is the game's again");
        assert!(press(&mut c, 0, Button::X).events.is_empty(), "X alone is the game's again");
        assert!(c.button(0, Button::Guide, false, now).events.is_empty(), "the late release does nothing");
        assert!(!c.is_open());
    }

    #[test]
    fn a_lost_guide_while_open_keeps_the_overlay_held() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.open();
        c.button(0, Button::Guide, true, now);
        let out = c.lost_guide(0);
        assert!(out.events.is_empty() && out.grabs.is_empty());
        assert!(c.is_open() && c.holds(0));
        assert_eq!(press(&mut c, 0, Button::A).events, vec![Event::Press(Button::A)]);
    }

    #[test]
    fn a_guide_pad_gone_mid_chord_gives_the_others_back() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.button(1, Button::RB, true, now).grabs, vec![Grab::Hold(1)]);
        assert_eq!(c.removed(0, 1).grabs, vec![Grab::ReleaseAll]);
        assert!(!c.holds(1));
        assert!(press(&mut c, 1, Button::RB).events.is_empty(), "RB alone is the game's again");
    }

    #[test]
    fn dropped_events_stop_a_repeat() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.open();
        c.button(0, Button::Down, true, now);
        c.dropped(1);
        assert_eq!(c.tick(now + REPEAT_AFTER), Some(Event::Press(Button::Down)), "another controller's drop");
        c.dropped(0);
        assert_eq!(c.tick(now + Duration::from_secs(5)), None);
    }

    #[test]
    fn last_controller_gone_closes() {
        let mut c = Core::new(ALL);
        press(&mut c, 0, Button::Guide);
        assert!(c.removed(1, 1).events.is_empty());
        assert_eq!(c.removed(0, 0).events, vec![Event::Close]);
    }

    #[test]
    fn disconnected_controller_does_not_leave_repeats_on_other_pads() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.open();
        c.stick(0, 0., 1., now);
        assert!(c.removed(0, 1).events.is_empty());
        assert!(c.is_open());
        assert_eq!(c.tick(now + REPEAT_AFTER), None);
        assert_eq!(c.stick(1, 0., 1., now).events, vec![Event::Press(Button::Up)]);
        assert!(c.stick(0, 0., 0., now).events.is_empty());
        assert_eq!(c.tick(now + REPEAT_AFTER), Some(Event::Press(Button::Up)));
    }

    #[test]
    fn another_controllers_release_does_not_cancel_repeat() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.open();
        c.button(0, Button::Down, true, now);
        c.button(1, Button::Down, false, now);
        c.removed(1, 1);
        assert_eq!(c.tick(now + REPEAT_AFTER), Some(Event::Press(Button::Down)));
    }

    #[test]
    fn guide_stops_navigation_repeat_while_a_chord_is_held() {
        let mut c = Core::new(ALL);
        let now = Instant::now();
        c.open();
        c.button(0, Button::Down, true, now);
        c.button(0, Button::Guide, true, now);
        assert_eq!(c.tick(now + REPEAT_AFTER), None);
        assert_eq!(c.button(0, Button::Up, true, now).events, vec![Event::Volume(1)]);
        assert!(c.button(0, Button::Guide, false, now).events.is_empty());
        assert_eq!(c.tick(now + REPEAT_AFTER), None);
    }

    const CONSOLE: Options = Options { console_hold: true, ..ALL };

    #[test]
    fn guide_held_alone_is_console_mode_and_opens_nothing() {
        let mut c = Core::new(CONSOLE);
        let t0 = Instant::now();
        let down = c.button(0, Button::Guide, true, t0);
        assert_eq!(down.events, vec![Event::ConsoleHolding(t0)]);
        assert_eq!(down.grabs, vec![Grab::Hold(0)], "grabbed as always");
        assert_eq!(c.wait(t0), Some(CONSOLE_HOLD));
        assert_eq!(c.tick(t0 + CONSOLE_HOLD / 2), None);
        assert_eq!(c.tick(t0 + CONSOLE_HOLD), Some(Event::ConsoleMode));
        assert_eq!(c.tick(t0 + CONSOLE_HOLD * 3), None, "once");
        let up = c.button(0, Button::Guide, false, t0 + CONSOLE_HOLD * 3);
        assert!(up.events.is_empty(), "no overlay after it: {:?}", up.events);
        assert_eq!(up.grabs, vec![Grab::ReleaseAll]);
    }

    #[test]
    fn a_guide_tap_or_chord_still_does_what_it_did() {
        let mut c = Core::new(CONSOLE);
        let t0 = Instant::now();
        c.button(0, Button::Guide, true, t0);
        let up = c.button(0, Button::Guide, false, t0 + CONSOLE_HOLD / 4);
        assert_eq!(up.events, vec![Event::ConsoleCancelled, Event::Open], "a tap opens the overlay");
        assert_eq!(c.tick(t0 + CONSOLE_HOLD * 2), None);
        // Open: Guide does not count for console mode.
        assert!(c.button(0, Button::Guide, true, t0).events.is_empty());
        c.button(0, Button::Guide, false, t0);
        assert!(!c.open);
        // A chord cancels it.
        c.button(0, Button::Guide, true, t0);
        let rb = c.button(0, Button::RB, true, t0 + CONSOLE_HOLD / 4);
        assert_eq!(rb.events, vec![Event::ConsoleCancelled, Event::Screenshot]);
        assert_eq!(c.tick(t0 + CONSOLE_HOLD * 2), None);
        // Without the option, nothing changes.
        let mut plain = Core::new(ALL);
        assert!(plain.button(0, Button::Guide, true, t0).events.is_empty());
        assert_eq!(plain.tick(t0 + CONSOLE_HOLD * 2), None);
        assert_eq!(plain.button(0, Button::Guide, false, t0 + CONSOLE_HOLD * 2).events, vec![Event::Open]);
    }

    #[test]
    fn a_lost_guide_cancels_console_mode() {
        let mut c = Core::new(CONSOLE);
        let t0 = Instant::now();
        c.button(3, Button::Guide, true, t0);
        assert_eq!(c.lost_guide(3).events, vec![Event::ConsoleCancelled]);
        assert_eq!(c.tick(t0 + CONSOLE_HOLD * 2), None);
        c.button(3, Button::Guide, true, t0);
        assert_eq!(c.removed(3, 0).events, vec![Event::ConsoleCancelled]);
        assert_eq!(c.tick(t0 + CONSOLE_HOLD * 2), None);
    }
}
