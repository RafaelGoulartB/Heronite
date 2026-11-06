//! Pure XInput snapshot conversion. Back/Start taps are delayed until release
//! so the first half of a Guide combination cannot navigate the panel.

use super::Button;

const START: u32 = 0x0010;
const BACK: u32 = 0x0020;
const GUIDE_COMBO: u32 = START | BACK;
const LT: u32 = 1 << 16;
const RT: u32 = 1 << 17;
const TRIGGER_THRESHOLD: u8 = 30;
const STICK_DEADZONE: i32 = 7849;

const BUTTONS: [(u32, Button); 12] = [
    (0x1000, Button::A),
    (0x2000, Button::B),
    (0x4000, Button::X),
    (0x8000, Button::Y),
    (0x0100, Button::LB),
    (0x0200, Button::RB),
    (0x0001, Button::Up),
    (0x0002, Button::Down),
    (0x0004, Button::Left),
    (0x0008, Button::Right),
    (LT, Button::LT),
    (RT, Button::RT),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_x: i16,
    pub left_y: i16,
}

#[cfg(windows)]
impl From<heronite_platform::gamepad::State> for Snapshot {
    fn from(state: heronite_platform::gamepad::State) -> Self {
        Self {
            buttons: state.buttons,
            left_trigger: state.left_trigger,
            right_trigger: state.right_trigger,
            left_x: state.left_x,
            left_y: state.left_y,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Change {
    Button(Button, bool),
    Stick(f32, f32),
}

#[derive(Default)]
enum Combo {
    #[default]
    Idle,
    Pending(u32),
    Guide,
    /// The remaining half must be released before another gesture starts.
    Releasing,
}

pub(super) struct Adapter {
    state: Snapshot,
    down: u32,
    ignored: u32,
    combo: Combo,
    stick_ready: bool,
}

impl Adapter {
    /// Already-held controls must be released after connecting or resetting.
    pub fn new(state: Snapshot) -> Self {
        Self {
            state,
            down: 0,
            ignored: buttons(state),
            combo: if u32::from(state.buttons) & GUIDE_COMBO == 0 { Combo::Idle } else { Combo::Releasing },
            stick_ready: neutral(state),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.state);
    }

    pub fn update(&mut self, state: Snapshot) -> Vec<Change> {
        let physical = buttons(state);
        self.ignored &= physical;
        let down = physical & !self.ignored;
        let mut out = Vec::new();
        let combo = down & GUIDE_COMBO;
        let mut release_guide = false;
        match self.combo {
            Combo::Idle | Combo::Pending(_) if combo == GUIDE_COMBO => {
                self.combo = Combo::Guide;
                out.push(Change::Button(Button::Guide, true));
            }
            Combo::Idle | Combo::Pending(_) => {
                if let Combo::Pending(pending) = self.combo {
                    for (bit, button) in [(START, Button::Start), (BACK, Button::Select)] {
                        if pending & !combo & bit != 0 {
                            out.push(Change::Button(button, true));
                            out.push(Change::Button(button, false));
                        }
                    }
                }
                self.combo = if combo == 0 { Combo::Idle } else { Combo::Pending(combo) };
            }
            Combo::Guide if combo != GUIDE_COMBO => {
                release_guide = true;
                self.combo = Combo::Releasing;
            }
            Combo::Guide | Combo::Releasing => {}
        }
        // Use the physical buttons here: reconnecting with either held must not
        // turn the other half into a fresh Start/Back tap.
        if matches!(self.combo, Combo::Releasing) && physical & GUIDE_COMBO == 0 {
            self.combo = Combo::Idle;
        }
        for (bit, button) in BUTTONS {
            if (self.down ^ down) & bit != 0 {
                out.push(Change::Button(button, down & bit != 0));
            }
        }
        if !self.stick_ready {
            self.stick_ready = neutral(state);
        } else if (state.left_x, state.left_y) != (self.state.left_x, self.state.left_y) {
            out.push(Change::Stick(normalize(state.left_x), normalize(state.left_y)));
        }
        // Other controls in this snapshot still belong to the chord.
        if release_guide {
            out.push(Change::Button(Button::Guide, false));
        }
        self.state = state;
        self.down = down;
        out
    }
}

fn buttons(state: Snapshot) -> u32 {
    u32::from(state.buttons)
        | if state.left_trigger > TRIGGER_THRESHOLD { LT } else { 0 }
        | if state.right_trigger > TRIGGER_THRESHOLD { RT } else { 0 }
}

fn neutral(state: Snapshot) -> bool {
    i32::from(state.left_x).abs() <= STICK_DEADZONE && i32::from(state.left_y).abs() <= STICK_DEADZONE
}

fn normalize(value: i16) -> f32 {
    f32::from(value) / if value < 0 { 32768. } else { 32767. }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::core::Core;
    use crate::input::{Event, Options};
    use std::time::{Duration, Instant};

    const ALL: Options = Options { screenshot: true, screenshot_button: Button::RB, volume: true, moment: true, replay: true };

    fn snapshot(buttons: u32) -> Snapshot {
        Snapshot { buttons: buttons as u16, ..Snapshot::default() }
    }

    fn feed(adapter: &mut Adapter, core: &mut Core, state: Snapshot, now: Instant) -> Vec<Event> {
        adapter
            .update(state)
            .into_iter()
            .flat_map(|change| match change {
                Change::Button(button, down) => core.button(0, button, down, now).events,
                Change::Stick(x, y) => core.stick(0, x, y, now).events,
            })
            .collect()
    }

    #[test]
    fn combo_opens_and_closes_once_without_start_or_back_navigation() {
        for first in [START, BACK] {
            let now = Instant::now();
            let mut adapter = Adapter::new(Snapshot::default());
            let mut core = Core::new(ALL);
            for expected in [Event::Open, Event::Close] {
                assert!(feed(&mut adapter, &mut core, snapshot(first), now).is_empty());
                assert!(feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now).is_empty());
                assert!(feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now).is_empty());
                assert_eq!(feed(&mut adapter, &mut core, snapshot(first), now), vec![expected]);
                assert!(
                    feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now).is_empty(),
                    "re-pressing while the other half is held is not a new gesture"
                );
                assert!(feed(&mut adapter, &mut core, snapshot(0), now).is_empty());
            }
        }
    }

    #[test]
    fn simultaneous_combo_release_has_no_phantom_navigation() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        for expected in [Event::Open, Event::Close] {
            assert!(feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now).is_empty());
            assert_eq!(feed(&mut adapter, &mut core, snapshot(0), now), vec![expected]);
            assert!(feed(&mut adapter, &mut core, snapshot(0), now).is_empty());
        }
    }

    #[test]
    fn single_start_and_back_still_navigate_on_release() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        core.open();
        for (bit, button) in [(START, Button::Start), (BACK, Button::Select)] {
            assert!(feed(&mut adapter, &mut core, snapshot(bit), now).is_empty());
            assert_eq!(feed(&mut adapter, &mut core, snapshot(0), now), vec![Event::Press(button)]);
        }
    }

    #[test]
    fn chords_are_not_navigation_and_do_not_toggle() {
        let now = Instant::now();
        for (button, expected) in [
            (0x0200, Event::Screenshot),
            (0x0001, Event::Volume(1)),
            (0x0002, Event::Volume(-1)),
            (0x8000, Event::Moment),
            (0x4000, Event::Replay),
        ] {
            let mut adapter = Adapter::new(Snapshot::default());
            let mut core = Core::new(ALL);
            assert_eq!(feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO | button), now), vec![expected]);
            assert!(feed(&mut adapter, &mut core, snapshot(0), now).is_empty());
            assert!(!core.is_open());
        }
    }

    #[test]
    fn final_snapshot_buttons_still_belong_to_the_chord() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now);
        assert_eq!(feed(&mut adapter, &mut core, snapshot(0x0200), now), vec![Event::Screenshot]);
        assert!(!core.is_open());
    }

    #[test]
    fn native_axes_buttons_and_triggers_are_converted() {
        assert_eq!(normalize(i16::MIN), -1.);
        assert_eq!(normalize(0), 0.);
        assert_eq!(normalize(i16::MAX), 1.);
        for (bit, button) in BUTTONS.into_iter().filter(|(bit, _)| *bit < LT) {
            let mut adapter = Adapter::new(Snapshot::default());
            assert_eq!(adapter.update(snapshot(bit)), vec![Change::Button(button, true)]);
            assert!(adapter.update(snapshot(bit)).is_empty());
            assert_eq!(adapter.update(snapshot(0)), vec![Change::Button(button, false)]);
        }
        let mut adapter = Adapter::new(Snapshot::default());
        assert!(adapter.update(Snapshot { left_trigger: 30, right_trigger: 30, ..Snapshot::default() }).is_empty());
        assert_eq!(
            adapter.update(Snapshot { left_trigger: 31, right_trigger: 255, left_x: i16::MIN, left_y: i16::MAX, ..Snapshot::default() }),
            vec![Change::Button(Button::LT, true), Change::Button(Button::RT, true), Change::Stick(-1., 1.)]
        );
        assert_eq!(
            adapter.update(Snapshot::default()),
            vec![Change::Button(Button::LT, false), Change::Button(Button::RT, false), Change::Stick(0., 0.)]
        );
        assert!(adapter.update(snapshot(0x0cc0)).is_empty(), "unsupported bits are not Guide");
    }

    #[test]
    fn navigation_repeats_until_release() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        core.open();
        assert_eq!(feed(&mut adapter, &mut core, snapshot(0x0002), now), vec![Event::Press(Button::Down)]);
        assert_eq!(core.tick(now + Duration::from_secs(1)), Some(Event::Press(Button::Down)));
        assert!(feed(&mut adapter, &mut core, snapshot(0), now).is_empty());
        assert_eq!(core.tick(now + Duration::from_secs(2)), None);
        assert_eq!(
            feed(&mut adapter, &mut core, Snapshot { left_x: i16::MAX, ..Snapshot::default() }, now),
            vec![Event::Press(Button::Right)]
        );
        assert_eq!(core.tick(now + Duration::from_secs(3)), Some(Event::Press(Button::Right)));
        feed(&mut adapter, &mut core, Snapshot::default(), now);
        assert_eq!(core.tick(now + Duration::from_secs(4)), None);
    }

    #[test]
    fn reset_consumes_held_combo_until_release() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        core.open();
        feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now);
        core.close();
        core.reset_input();
        adapter.reset();
        assert!(feed(&mut adapter, &mut core, snapshot(BACK), now).is_empty());
        assert!(feed(&mut adapter, &mut core, snapshot(0), now).is_empty());
        assert!(!core.is_open());
        feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now);
        assert_eq!(feed(&mut adapter, &mut core, snapshot(0), now), vec![Event::Open]);
    }

    #[test]
    fn reset_cancels_repeats_and_requires_held_controls_to_return_to_rest() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        core.open();
        let held = Snapshot { buttons: 0x0002, left_x: i16::MAX, left_trigger: 255, ..Snapshot::default() };
        assert_eq!(
            feed(&mut adapter, &mut core, held, now),
            vec![Event::Press(Button::Down), Event::Press(Button::LT), Event::Press(Button::Right)]
        );
        core.reset_input();
        adapter.reset();
        assert!(feed(&mut adapter, &mut core, held, now).is_empty());
        assert_eq!(core.tick(now + Duration::from_secs(5)), None);
        assert!(feed(&mut adapter, &mut core, Snapshot::default(), now).is_empty());
        assert_eq!(
            feed(&mut adapter, &mut core, held, now),
            vec![Event::Press(Button::Down), Event::Press(Button::LT), Event::Press(Button::Right)]
        );
    }

    #[test]
    fn either_combo_half_held_at_connect_is_consumed() {
        let now = Instant::now();
        for held in [BACK, START] {
            let mut adapter = Adapter::new(snapshot(held));
            let mut core = Core::new(ALL);
            core.open();
            assert!(feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now).is_empty());
            assert!(feed(&mut adapter, &mut core, snapshot(0), now).is_empty());
            assert!(core.is_open());
            feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now);
            assert_eq!(feed(&mut adapter, &mut core, snapshot(0), now), vec![Event::Close]);
        }
    }

    #[test]
    fn disconnect_and_reconnect_do_not_complete_old_gestures() {
        let now = Instant::now();
        let mut adapter = Adapter::new(Snapshot::default());
        let mut core = Core::new(ALL);
        feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now);
        assert!(core.removed(0, 0).events.is_empty());
        let held = Snapshot { buttons: GUIDE_COMBO as u16, left_x: i16::MAX, left_trigger: 255, ..Snapshot::default() };
        let mut adapter = Adapter::new(held);
        assert!(feed(&mut adapter, &mut core, held, now).is_empty());
        assert!(feed(&mut adapter, &mut core, Snapshot::default(), now).is_empty());
        feed(&mut adapter, &mut core, snapshot(GUIDE_COMBO), now);
        assert_eq!(feed(&mut adapter, &mut core, snapshot(0), now), vec![Event::Open]);
        assert_eq!(core.removed(0, 0).events, vec![Event::Close]);
        assert_eq!(core.tick(now + Duration::from_secs(5)), None);
    }

    #[cfg(windows)]
    #[test]
    fn platform_snapshot_has_no_axis_or_trigger_changes() {
        let state = heronite_platform::gamepad::State {
            buttons: GUIDE_COMBO as u16,
            left_trigger: 42,
            right_trigger: 255,
            left_x: i16::MIN,
            left_y: i16::MAX,
        };
        assert_eq!(
            Snapshot::from(state),
            Snapshot { buttons: GUIDE_COMBO as u16, left_trigger: 42, right_trigger: 255, left_x: i16::MIN, left_y: i16::MAX }
        );
    }
}
