//! Nintendo Switch Pro controllers and those that pass for one (`057E:2009`),
//! the GameSir Tarantula Pro on its 2.4 GHz receiver in Switch mode too
//! (BatteryHub's `switch_pro.cpp`).
//!
//! The battery is in the full input report `30`: byte 2, bits 7-5 the level
//! (0..=4, empty to full), bit 4 charging. Reference: SDL's
//! `src/joystick/hidapi/SDL_hidapi_switch.c`.
//!
//! The controller sends the full report only once a host asked for it (Steam
//! Input does); until then it sends the simple report `3F`, without the
//! battery. The driver only listens to the gamepad collection, opened
//! read-only: it never asks for the full report.

use std::io;
use std::time::{Duration, Instant};

use super::{Driver, Reading};
use crate::hid::{Collection, Device};

const VENDOR: u16 = 0x057E;
const PRODUCT: u16 = 0x2009;

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

/// How long to listen to one controller: in full report mode it sends one
/// every few ms.
const LISTEN: Duration = Duration::from_millis(350);
/// How long the driver may take in all.
const BUDGET: Duration = Duration::from_millis(2000);

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    (vendor == VENDOR && product == PRODUCT).then_some("Nintendo Switch Pro Controller")
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let deadline = Instant::now() + BUDGET;
    // The joystick or gamepad collection of each controller.
    let gamepads = collections.iter().filter(|c| {
        c.vendor == vendor && c.product == product && c.usage_page == 0x01 && matches!(c.usage, 0x04 | 0x05) && c.input_len >= 13
    });
    let (mut failed, mut listened) = (None, false);
    for collection in gamepads {
        match listen(collection, deadline) {
            Ok(Some(reading)) => return Ok(Some(reading)),
            Ok(None) => listened = true,
            Err(e) => {
                failed.get_or_insert(e);
            }
        }
    }
    // An error only when no controller could be listened to.
    match failed {
        Some(e) if !listened => Err(e),
        _ => Ok(None),
    }
}

/// Listens to `collection` for [`LISTEN`] (until `deadline` at most) for a
/// full report.
fn listen(collection: &Collection, deadline: Instant) -> io::Result<Option<Reading>> {
    let device = Device::open_read_only(collection)?;
    let end = deadline.min(Instant::now() + LISTEN);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let Some(report) = device.read(left)? else { break };
        if let Some(reading) = parse_full_report(&report) {
            return Ok(Some(reading));
        }
    }
    Ok(None)
}

/// Full input report `30` (`[0]` its id); `None` for any other report.
fn parse_full_report(report: &[u8]) -> Option<Reading> {
    if report.len() < 13 || report[0] != 0x30 {
        return None;
    }
    let level = (report[2] & 0xE0) >> 5;
    if level > 4 {
        return None;
    }
    // Charging stops counting once full.
    Some(Reading { percent: 25 * level, charging: report[2] & 0x10 != 0 && level < 4 })
}

#[cfg(test)]
mod tests {
    //! No captures of these controllers come with BatteryHub: the reports
    //! below are built from SDL's layout.

    use super::*;

    /// A report `id` of 64 bytes with `battery` as byte 2.
    fn report(id: u8, battery: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        (r[0], r[1], r[2]) = (id, 0x5A, battery);
        r
    }

    fn reading(percent: u8, charging: bool) -> Option<Reading> {
        Some(Reading { percent, charging })
    }

    #[test]
    fn the_full_report_tells_the_level() {
        // Levels 0 to 4 (bits 7-5), the connection nibble (bits 3-0) aside.
        assert_eq!(parse_full_report(&report(0x30, 0x0E)), reading(0, false));
        assert_eq!(parse_full_report(&report(0x30, 0x20)), reading(25, false));
        assert_eq!(parse_full_report(&report(0x30, 0x4E)), reading(50, false));
        assert_eq!(parse_full_report(&report(0x30, 0x60)), reading(75, false));
        assert_eq!(parse_full_report(&report(0x30, 0x8E)), reading(100, false));
    }

    #[test]
    fn charging_is_bit_4_until_full() {
        assert_eq!(parse_full_report(&report(0x30, 0x51)), reading(50, true));
        assert_eq!(parse_full_report(&report(0x30, 0x91)), reading(100, false));
    }

    #[test]
    fn other_reports_and_levels_are_skipped() {
        assert_eq!(parse_full_report(&report(0x3F, 0x80)), None, "the simple report");
        assert_eq!(parse_full_report(&report(0x21, 0x80)), None, "a subcommand reply");
        assert_eq!(parse_full_report(&report(0x30, 0xA0)), None, "level 5");
        assert_eq!(parse_full_report(&report(0x30, 0xE0)), None, "level 7");
        assert_eq!(parse_full_report(&report(0x30, 0x80)[..12]), None, "too short");
    }

    #[test]
    fn the_model_is_the_pro_controller() {
        assert_eq!(model(VENDOR, PRODUCT), Some("Nintendo Switch Pro Controller"));
        assert_eq!(model(VENDOR, 0x2006), None, "a Joy-Con");
        assert_eq!(model(0x054C, PRODUCT), None);
    }
}
