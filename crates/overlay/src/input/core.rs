//! What the Guide button does, without devices: pure, so it is tested
//! without a controller. The thread in `input::linux` feeds it the presses of
//! every controller and applies the grabs it asks for.
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
    held: Option<(Button, Instant)>,
    /// Where the left stick points (past the threshold).
    stick: Option<Button>,
}

impl Core {
    pub fn new(opts: Options) -> Self {
        Self { opts, open: false, guide: None, held: None, stick: None }
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether `dev` must stay grabbed (its Guide is down or the overlay is open).
    pub fn holds(&self, dev: usize) -> bool {
        self.open || self.guide.is_some_and(|(d, _)| d == dev)
    }

    /// A button of controller `dev` went down (`true`) or up.
    pub fn button(&mut self, dev: usize, b: Button, down: bool, now: Instant) -> Output {
        let mut out = Output::default();
        if b == Button::Guide {
            self.guide_button(dev, down, &mut out);
            return out;
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
                self.held = Some((b, now + REPEAT_AFTER));
            }
            out.events.push(Event::Press(b));
        } else if self.held.is_some_and(|(h, _)| h == b) {
            self.held = None;
        }
        out
    }

    fn guide_button(&mut self, dev: usize, down: bool, out: &mut Output) {
        if down {
            if self.guide.is_none() {
                self.guide = Some((dev, false));
                if !self.open {
                    out.grabs.push(Grab::Hold(dev));
                }
            }
            return;
        }
        let Some((gdev, used)) = self.guide else { return };
        if gdev != dev {
            return;
        }
        self.guide = None;
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

    /// A controller went away.
    pub fn removed(&mut self, dev: usize, remaining: usize) -> Output {
        let mut out = Output::default();
        if self.guide.is_some_and(|(d, _)| d == dev) {
            self.guide = None;
        }
        if self.open && remaining == 0 {
            self.set_closed(&mut out);
        }
        out
    }

    /// The left stick of `dev` moved (`x`, `y` in -1..=1, `y` up). While Guide
    /// is held, up / down count as the volume chord too.
    pub fn stick(&mut self, dev: usize, x: f32, y: f32, now: Instant) -> Output {
        let threshold = |wanted: Button| if self.stick == Some(wanted) { STICK_OFF } else { STICK_ON };
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
        if dir == self.stick {
            return Output::default();
        }
        let old = std::mem::replace(&mut self.stick, dir);
        let mut out = Output::default();
        if let Some(o) = old {
            out = self.button(dev, o, false, now);
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
        self.held.map(|(_, at)| at.saturating_duration_since(now))
    }

    /// The repeat due at `now`, if any.
    pub fn tick(&mut self, now: Instant) -> Option<Event> {
        let (b, at) = self.held?;
        if now < at || !self.open {
            return None;
        }
        self.held = Some((b, now + REPEAT_EVERY));
        Some(Event::Press(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Options = Options { screenshot: true, screenshot_button: Button::RB, volume: true, moment: true, replay: true };

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
        assert_eq!(again.grabs, vec![Grab::ReleaseAll]);
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
        let mut c = Core::new(Options { screenshot: false, screenshot_button: Button::RB, volume: false, moment: false, replay: false });
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
    fn last_controller_gone_closes() {
        let mut c = Core::new(ALL);
        press(&mut c, 0, Button::Guide);
        assert!(c.removed(1, 1).events.is_empty());
        assert_eq!(c.removed(0, 0).events, vec![Event::Close]);
    }
}
