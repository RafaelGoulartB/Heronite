//! A combination of buttons held long enough ([`HOLD_FOR`]), from the
//! buttons as they go down and up. Pure, so it is tested without a
//! controller.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use heronite_data::console_theme::ConsoleButton;

use crate::console::input::PadButton;

/// How long the combination is held: as long as the overlay's Guide for
/// console mode, so both feel the same.
pub const HOLD_FOR: Duration = heronite_overlay::input::CONSOLE_HOLD;

/// The buttons of a setting (none for `Off`).
pub fn combo(b: ConsoleButton) -> &'static [PadButton] {
    match b.resolved() {
        ConsoleButton::Guide => &[PadButton::Guide],
        ConsoleButton::BackStart => &[PadButton::Select, PadButton::Start],
        ConsoleButton::Bumpers => &[PadButton::LB, PadButton::RB],
        ConsoleButton::Off | ConsoleButton::Auto => &[],
    }
}

/// What a change of the buttons did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Every button of the combination is down, since then.
    Started(Instant),
    /// One came up before [`HOLD_FOR`].
    Cancelled,
    /// Held for [`HOLD_FOR`].
    Done,
}

#[derive(Default)]
pub struct Hold {
    combo: Vec<PadButton>,
    /// The buttons down now, on any controller.
    down: HashSet<PadButton>,
    since: Option<Instant>,
    /// Done already: nothing more until the combination is let go.
    fired: bool,
}

impl Hold {
    /// The combination to watch; a new one starts over.
    pub fn set_combo(&mut self, combo: &[PadButton]) {
        if self.combo != combo {
            self.combo = combo.to_vec();
            self.since = None;
            self.fired = false;
        }
    }

    pub fn button(&mut self, b: PadButton, down: bool, now: Instant) -> Option<Step> {
        if down {
            self.down.insert(b);
        } else {
            self.down.remove(&b);
        }
        let held = !self.combo.is_empty() && self.combo.iter().all(|c| self.down.contains(c));
        match (held, self.since) {
            (true, None) if !self.fired => {
                self.since = Some(now);
                Some(Step::Started(now))
            }
            (false, Some(_)) => {
                self.since = None;
                Some(Step::Cancelled)
            }
            (false, None) => {
                self.fired = false;
                None
            }
            _ => None,
        }
    }

    /// [`Step::Done`] once the combination has been held for [`HOLD_FOR`].
    pub fn tick(&mut self, now: Instant) -> Option<Step> {
        let since = self.since?;
        if now.saturating_duration_since(since) < HOLD_FOR {
            return None;
        }
        self.since = None;
        self.fired = true;
        Some(Step::Done)
    }

    /// When the combination went down, while it is held.
    pub fn since(&self) -> Option<Instant> {
        self.since
    }

    /// Forgets every button (a controller went away: its releases never come).
    pub fn clear(&mut self) {
        self.down.clear();
        self.since = None;
        self.fired = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64, t0: Instant) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn each_setting_has_its_buttons() {
        assert_eq!(combo(ConsoleButton::Off), &[] as &[PadButton]);
        assert_eq!(combo(ConsoleButton::Guide), &[PadButton::Guide]);
        assert_eq!(combo(ConsoleButton::BackStart), &[PadButton::Select, PadButton::Start]);
        assert_eq!(combo(ConsoleButton::Bumpers), &[PadButton::LB, PadButton::RB]);
        let auto = if cfg!(windows) { combo(ConsoleButton::BackStart) } else { combo(ConsoleButton::Guide) };
        assert_eq!(combo(ConsoleButton::Auto), auto, "the system's button");
    }

    #[test]
    fn a_combination_held_for_a_second_is_done_once() {
        let t0 = Instant::now();
        let mut h = Hold::default();
        h.set_combo(combo(ConsoleButton::Bumpers));
        assert_eq!(h.button(PadButton::LB, true, t0), None, "half of it");
        assert_eq!(h.button(PadButton::RB, true, at(50, t0)), Some(Step::Started(at(50, t0))));
        assert_eq!(h.since(), Some(at(50, t0)));
        assert_eq!(h.tick(at(900, t0)), None);
        assert_eq!(h.tick(at(1050, t0)), Some(Step::Done));
        assert_eq!(h.tick(at(3000, t0)), None, "once");
        // Still held: a press of another button does not start it again.
        assert_eq!(h.button(PadButton::South, true, at(3100, t0)), None);
        assert_eq!(h.button(PadButton::RB, false, at(3200, t0)), None);
        assert_eq!(h.button(PadButton::RB, true, at(3300, t0)), Some(Step::Started(at(3300, t0))), "again once let go");
    }

    #[test]
    fn letting_go_early_cancels() {
        let t0 = Instant::now();
        let mut h = Hold::default();
        h.set_combo(combo(ConsoleButton::Guide));
        assert_eq!(h.button(PadButton::Guide, true, t0), Some(Step::Started(t0)));
        assert_eq!(h.button(PadButton::Guide, false, at(400, t0)), Some(Step::Cancelled));
        assert_eq!(h.tick(at(2000, t0)), None);
        assert_eq!(h.button(PadButton::Guide, true, at(2100, t0)), Some(Step::Started(at(2100, t0))));
    }

    #[test]
    fn off_holds_nothing_and_clear_forgets_the_buttons() {
        let t0 = Instant::now();
        let mut h = Hold::default();
        h.set_combo(combo(ConsoleButton::Off));
        assert_eq!(h.button(PadButton::Guide, true, t0), None);
        h.set_combo(combo(ConsoleButton::Bumpers));
        h.button(PadButton::LB, true, t0);
        h.clear();
        assert_eq!(h.button(PadButton::RB, true, t0), None, "LB went away with its controller");
        assert_eq!(h.since(), None);
    }
}
