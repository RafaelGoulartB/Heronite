//! Sony DualShock 4, DualSense and DualSense Edge, on a cable, Sony's DualShock
//! 4 wireless adapter or Bluetooth (BatteryHub's `playstation.cpp`). The
//! battery is part of the input report the gamepad collection (`01:05`)
//! sends anyway, so the driver only listens:
//!
//! - DualShock 4: USB report `01` byte 30, Bluetooth report `11` byte 32;
//!   bits 0-3 the level (0..=10, 11 when full on a cable), bit 4 the cable.
//! - DualSense: USB report `01` byte 53, Bluetooth report `31` byte 54; bits
//!   0-3 the level (0..=10), bits 4-7 the state (1 charging, 2 full).
//!
//! A level `n` below 10 stands for `n`0 to `n`9 %, read as its middle.
//!
//! Layout reference: Linux `drivers/hid/hid-playstation.c`.
//!
//! Over Bluetooth both controllers send a reduced report `01` (no battery)
//! until a feature report is read; BatteryHub reads feature report `05` for
//! that, Heronite never does (it cannot be undone without turning the
//! controller off, and DirectInput games stop seeing it; see `controllers`).
//! Over Bluetooth the battery is read only when the full report already
//! arrives (Steam may have enabled it), `None` otherwise.

use std::io;
use std::time::{Duration, Instant};

use super::{Driver, Reading};
use crate::hid::{Collection, Device};

const VENDOR: u16 = 0x054C;

/// The DualShock 4 wireless adapter: its battery byte is 0 while no
/// controller is linked to it.
const DS4_ADAPTER: u16 = 0x0BA0;

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

/// How long to listen to one controller: it sends its report every few ms.
const LISTEN: Duration = Duration::from_millis(500);
/// How long the driver may take in all.
const BUDGET: Duration = Duration::from_millis(2000);

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    if vendor != VENDOR {
        return None;
    }
    match product {
        // V1, V2, the wireless adapter.
        0x05C4 | 0x09CC | DS4_ADAPTER => Some("DualShock 4"),
        0x0CE6 => Some("DualSense"),
        0x0DF2 => Some("DualSense Edge"),
        _ => None,
    }
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let ds4 = matches!(product, 0x05C4 | 0x09CC | DS4_ADAPTER);
    let deadline = Instant::now() + BUDGET;
    // The gamepad collection of each controller of this model (two
    // DualSenses: the first that tells).
    let gamepads = collections.iter().filter(|c| c.vendor == vendor && c.product == product && c.usage_page == 0x01 && c.usage == 0x05);
    let (mut failed, mut listened) = (None, false);
    for collection in gamepads {
        // Reports over Bluetooth are longer than the 64 bytes of USB.
        let link = if collection.input_len > 64 {
            Link::Bluetooth
        } else if product == DS4_ADAPTER {
            Link::Adapter
        } else {
            Link::Cable
        };
        let parse = if ds4 { parse_ds4 } else { parse_dualsense };
        match listen(collection, link, parse, deadline) {
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
/// report `parse` takes.
fn listen(collection: &Collection, link: Link, parse: fn(&[u8], Link) -> Option<Reply>, deadline: Instant) -> io::Result<Option<Reading>> {
    let device = Device::open_read_only(collection)?;
    let end = deadline.min(Instant::now() + LISTEN);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        let Some(report) = device.read(left)? else { break };
        match parse(&report, link) {
            Some(Reply::Battery(reading)) => return Ok(Some(reading)),
            Some(Reply::NoController) => break,
            None => {}
        }
    }
    Ok(None)
}

/// How the controller is connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Cable,
    /// The DualShock 4 wireless adapter (USB reports).
    Adapter,
    /// Report `01` is the reduced one then.
    Bluetooth,
}

/// What an input report tells.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Battery(Reading),
    /// The DualShock 4 wireless adapter, without a controller.
    NoController,
}

/// An input report of a DualShock 4 (`[0]` its id).
fn parse_ds4(report: &[u8], link: Link) -> Option<Reply> {
    let at = match report.first()? {
        0x01 if link != Link::Bluetooth => 30,
        0x11 => 32,
        _ => return None,
    };
    let byte = *report.get(at)?;
    if link == Link::Adapter && byte == 0 {
        return Some(Reply::NoController);
    }
    let level = i32::from(byte & 0x0F);
    let cable = byte & 0x10 != 0;
    let percent = if level >= 10 { 100 } else { level * 10 + 5 };
    Some(Reply::Battery(Reading { percent: percent as u8, charging: cable && level < 11 }))
}

/// An input report of a DualSense or DualSense Edge (`[0]` its id).
fn parse_dualsense(report: &[u8], link: Link) -> Option<Reply> {
    let at = match report.first()? {
        0x01 if link != Link::Bluetooth => 53,
        0x31 => 54,
        _ => return None,
    };
    let byte = *report.get(at)?;
    let (level, state) = (i32::from(byte & 0x0F), byte >> 4);
    let reading = match state {
        // Fully charged, on a cable.
        0x2 => Reading { percent: 100, charging: true },
        _ => Reading { percent: (level * 10 + 5).clamp(0, 100) as u8, charging: state == 0x1 },
    };
    Some(Reply::Battery(reading))
}

#[cfg(test)]
mod tests {
    //! No captures of these controllers come with BatteryHub: the reports
    //! below are built from the offsets of the module documentation.

    use super::*;

    /// A report `id` of `len` bytes, with `battery` at `at`.
    fn report(id: u8, len: usize, at: usize, battery: u8) -> Vec<u8> {
        let mut r = vec![0u8; len];
        r[0] = id;
        r[at] = battery;
        r
    }

    fn battery(percent: u8, charging: bool) -> Option<Reply> {
        Some(Reply::Battery(Reading { percent, charging }))
    }

    #[test]
    fn a_dualshock_4_tells_it_in_its_input_report() {
        // On battery: 7 of 10.
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x07), Link::Adapter), battery(75, false));
        // On a cable, charging, then full.
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x14), Link::Cable), battery(45, true), "the level's middle, as off the cable");
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x1B), Link::Cable), battery(100, false));
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x0A), Link::Adapter), battery(100, false), "10 of 10, clamped");
        // Over Bluetooth, the full report.
        assert_eq!(parse_ds4(&report(0x11, 547, 32, 0x03), Link::Bluetooth), battery(35, false));
        assert_eq!(parse_ds4(&report(0x11, 547, 32, 0x00), Link::Bluetooth), battery(5, false));
    }

    #[test]
    fn the_dualshock_4_adapter_without_a_controller_tells_none() {
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x00), Link::Adapter), Some(Reply::NoController));
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x00), Link::Cable), battery(5, false));
    }

    #[test]
    fn a_dualsense_tells_it_in_its_input_report() {
        // Discharging, 6 of 10.
        assert_eq!(parse_dualsense(&report(0x01, 64, 53, 0x06), Link::Cable), battery(65, false));
        // Charging, then full.
        assert_eq!(parse_dualsense(&report(0x01, 64, 53, 0x13), Link::Cable), battery(35, true));
        assert_eq!(parse_dualsense(&report(0x01, 64, 53, 0x2A), Link::Cable), battery(100, true));
        assert_eq!(parse_dualsense(&report(0x01, 64, 53, 0x0A), Link::Cable), battery(100, false), "10 of 10, clamped");
        // An error state still tells the level.
        assert_eq!(parse_dualsense(&report(0x01, 64, 53, 0xF2), Link::Cable), battery(25, false));
        // Over Bluetooth, the full report.
        assert_eq!(parse_dualsense(&report(0x31, 78, 54, 0x18), Link::Bluetooth), battery(85, true));
    }

    #[test]
    fn the_reduced_bluetooth_report_is_skipped() {
        // Windows pads it to the collection's report length, so its length
        // tells nothing.
        assert_eq!(parse_ds4(&report(0x01, 547, 30, 0x07), Link::Bluetooth), None);
        assert_eq!(parse_dualsense(&report(0x01, 78, 53, 0x06), Link::Bluetooth), None);
    }

    #[test]
    fn other_and_short_reports_are_skipped() {
        assert_eq!(parse_ds4(&report(0x05, 64, 30, 0x07), Link::Cable), None);
        assert_eq!(parse_ds4(&report(0x01, 64, 30, 0x07)[..30], Link::Cable), None);
        assert_eq!(parse_ds4(&report(0x11, 78, 32, 0x07)[..32], Link::Bluetooth), None);
        assert_eq!(parse_dualsense(&report(0x02, 64, 53, 0x06), Link::Cable), None);
        assert_eq!(parse_dualsense(&report(0x01, 64, 53, 0x06)[..53], Link::Cable), None);
        assert_eq!(parse_dualsense(&report(0x31, 78, 54, 0x06)[..54], Link::Bluetooth), None);
        assert_eq!(parse_ds4(&[], Link::Cable), None);
    }

    #[test]
    fn models_by_product() {
        assert_eq!(model(VENDOR, 0x05C4), Some("DualShock 4"));
        assert_eq!(model(VENDOR, 0x09CC), Some("DualShock 4"));
        assert_eq!(model(VENDOR, 0x0BA0), Some("DualShock 4"));
        assert_eq!(model(VENDOR, 0x0CE6), Some("DualSense"));
        assert_eq!(model(VENDOR, 0x0DF2), Some("DualSense Edge"));
        assert_eq!(model(VENDOR, 0x0ECC), None, "the PlayStation Link adapter");
        assert_eq!(model(0x045E, 0x0CE6), None);
    }
}
