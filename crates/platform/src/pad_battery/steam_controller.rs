//! Valve's Steam Controllers (BatteryHub's `steam_controller.cpp`), read on
//! their vendor collection (page `FF00`), which sends battery reports among
//! its others. The driver only listens. Layout reference: Linux
//! `drivers/hid/hid-steam.c`.
//!
//! - The 2015 Steam Controller on its wireless receiver (`28DE:1142`): report
//!   `01 00 04 0B …` (a battery event, 11 bytes of payload), byte 14 the
//!   battery in percent. It tells no charging.
//! - The 2026 Steam Controller on a cable (`1302`), Bluetooth (`1303`), its
//!   puck (`1304`) or an internal receiver (`1305`): report `43`, byte 1 the
//!   charging state (2 charging), byte 2 the battery in percent. The puck
//!   has four slots (collections of usage `01`), present whether or not a
//!   controller is linked to each; its other collection is the charging
//!   contacts, and is skipped.
//!
//! The battery reports are not frequent: a read may end before one comes,
//! and then tells none.

use std::io;
use std::time::{Duration, Instant};

use super::{Driver, Reading};
use crate::hid::{Collection, Device};

const VENDOR: u16 = 0x28DE;
/// The 2015 controller's wireless receiver.
const RECEIVER_2015: u16 = 0x1142;
/// The 2026 controller's puck.
const PUCK: u16 = 0x1304;

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

/// How long the driver may take in all, shared between the slots.
const BUDGET: Duration = Duration::from_millis(2000);
/// A puck slot that sends nothing for this long has no controller.
const SLOT_WAIT: Duration = Duration::from_millis(250);

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    (vendor == VENDOR && matches!(product, RECEIVER_2015 | 0x1302 | 0x1303 | PUCK | 0x1305)).then_some("Steam Controller")
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let deadline = Instant::now() + BUDGET;
    let parse = if product == RECEIVER_2015 { parse_2015 } else { parse_2026 };
    let puck = product == PUCK;
    let slots: Vec<&Collection> = collections
        .iter()
        .filter(|c| c.vendor == vendor && c.product == product && c.usage_page == 0xFF00 && c.input_len >= 15 && (!puck || c.usage == 0x01))
        .collect();
    let (mut failed, mut listened) = (None, false);
    for (i, slot) in slots.iter().enumerate() {
        // An empty slot keeps no more than its share of what is left.
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
        let end = Instant::now() + left / (slots.len() - i) as u32;
        match listen(slot, puck, parse, end) {
            Ok(Some(reading)) => return Ok(Some(reading)),
            Ok(None) => listened = true,
            Err(e) => {
                failed.get_or_insert(e);
            }
        }
    }
    // An error only when no slot could be listened to.
    match failed {
        Some(e) if !listened => Err(e),
        _ => Ok(None),
    }
}

/// Listens to `slot` until `end` for a report `parse` takes. A puck slot
/// that sends nothing for [`SLOT_WAIT`] is left at once.
fn listen(slot: &Collection, puck: bool, parse: fn(&[u8]) -> Option<Reading>, end: Instant) -> io::Result<Option<Reading>> {
    let device = Device::open_read_only(slot)?;
    let mut first = true;
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let wait = if puck && first { left.min(SLOT_WAIT) } else { left };
        let Some(report) = device.read(wait)? else {
            if puck && first {
                return Ok(None);
            }
            continue;
        };
        first = false;
        if let Some(reading) = parse(&report) {
            return Ok(Some(reading));
        }
    }
    Ok(None)
}

/// A report of the 2015 controller's receiver (`[0]` its id, `01`); only the
/// battery event tells.
fn parse_2015(report: &[u8]) -> Option<Reading> {
    match report {
        [0x01, 0x00, 0x04, 11, rest @ ..] if rest.len() >= 11 && rest[10] <= 100 => Some(Reading { percent: rest[10], charging: false }),
        _ => None,
    }
}

/// A report of the 2026 controller (`[0]` its id); only report `43` tells.
fn parse_2026(report: &[u8]) -> Option<Reading> {
    match report {
        [0x43, state, percent, ..] if report.len() >= 15 && *percent <= 100 => Some(Reading { percent: *percent, charging: *state == 2 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! No captures of these controllers come with BatteryHub: the reports
    //! below are built from the offsets of the module documentation. Windows
    //! pads numbered reports to the collection's length, 64 bytes here.

    use super::*;

    fn padded(start: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..start.len()].copy_from_slice(start);
        r
    }

    /// A battery event of the 2015 controller, `percent` at byte 14.
    fn event_2015(percent: u8) -> Vec<u8> {
        let mut r = padded(&[0x01, 0x00, 0x04, 11]);
        r[14] = percent;
        r
    }

    fn reading(percent: u8, charging: bool) -> Option<Reading> {
        Some(Reading { percent, charging })
    }

    #[test]
    fn the_2015_controller_tells_it_in_its_battery_event() {
        assert_eq!(parse_2015(&event_2015(87)), reading(87, false));
        assert_eq!(parse_2015(&event_2015(100)), reading(100, false));
        assert_eq!(parse_2015(&event_2015(101)), None);
        assert_eq!(parse_2015(&event_2015(87)[..14]), None, "too short");
        // Other events: input (01), connection (03).
        assert_eq!(parse_2015(&padded(&[0x01, 0x00, 0x01, 0x3C])), None);
        assert_eq!(parse_2015(&padded(&[0x01, 0x00, 0x03, 0x01])), None);
        let mut length = event_2015(87);
        length[3] = 12;
        assert_eq!(parse_2015(&length), None, "another payload length");
    }

    #[test]
    fn the_2026_controller_tells_it_in_report_43() {
        assert_eq!(parse_2026(&padded(&[0x43, 0x01, 64])), reading(64, false));
        assert_eq!(parse_2026(&padded(&[0x43, 0x02, 64])), reading(64, true));
        assert_eq!(parse_2026(&padded(&[0x43, 0x00, 100])), reading(100, false));
        assert_eq!(parse_2026(&padded(&[0x43, 0x02, 0xFF])), None);
        assert_eq!(parse_2026(&padded(&[0x42, 0x02, 64])), None, "another report");
        assert_eq!(parse_2026(&padded(&[0x43, 0x02, 64])[..14]), None, "too short");
    }

    #[test]
    fn models_by_product() {
        for product in [0x1142, 0x1302, 0x1303, 0x1304, 0x1305] {
            assert_eq!(model(VENDOR, product), Some("Steam Controller"), "{product:04X}");
        }
        assert_eq!(model(VENDOR, 0x1205), None, "the Steam Deck");
        assert_eq!(model(VENDOR, 0x1102), None, "the 2015 controller on its cable");
        assert_eq!(model(0x054C, 0x1142), None);
    }
}
