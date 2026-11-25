//! GameSir controllers in XInput mode on their 2.4 GHz receiver or a cable,
//! read on GameSir's own HID channel (BatteryHub's `gamesir.cpp`):
//!
//! - Cyclone 2 family, the G7 Pro 8K too: vendor collection `FFF0:0040`
//!   (64-byte reports, ids `0F` out, `12` in). Request `0F 03`: it only asks,
//!   the pad then streams its state on that collection until it is closed.
//!   Reply, input report `12`: byte 35 the charging flag (1), byte 36 the
//!   battery in percent. Protocol: <https://github.com/vdemonchy/cyclone2-linux>
//!   (`docs/protocol.md`); the G7 Pro 8K answered it here, on its receiver and
//!   on the cable.
//! - Nova Lite, Nova 2, Nova 2 Lite: vendor collection page `FF7A` (64-byte
//!   unnumbered reports). Request `01 01` ("device info", what GameSir's
//!   client sends); reply `01 …`, byte 18 the battery in percent, `FF` while
//!   charging (no percentage then). Protocol:
//!   <https://github.com/WWWyyy452/GamesirBatteryTray> and
//!   <https://github.com/AJAY0406/Gamesir-Nova-2-Lite-Battery-Detection> (MIT).
//!
//! A receiver whose controller is off goes by another product id (Cyclone 2
//! and G7 Pro 8K: `0575`) and is not read: it would answer 0 %.

use std::io;
use std::time::{Duration, Instant};

use super::{Driver, Reading};
use crate::hid::{Collection, Device};

pub(super) const VENDOR: u16 = 0x3537;

pub(super) const DRIVERS: &[Driver] = &[Driver { model: cyclone_model, read: read_cyclone }, Driver { model: nova_model, read: read_nova }];

/// How long a reply may take. The pad answers within a few ms; a receiver
/// whose pad went to sleep never does.
const WAIT: Duration = Duration::from_millis(1500);
const WRITE_WAIT: Duration = Duration::from_millis(500);

fn cyclone_model(vendor: u16, product: u16) -> Option<&'static str> {
    if vendor != VENDOR {
        return None;
    }
    match product {
        // On its receiver, on the cable.
        0x10C6 | 0x10C5 => Some("GameSir G7 Pro 8K"),
        // Linked to its receiver, or on the cable.
        0x100B => Some("GameSir Cyclone 2"),
        _ => None,
    }
}

fn nova_model(vendor: u16, product: u16) -> Option<&'static str> {
    // 1098: receiver with a pad linked; 100F: Nova 2 Lite on a cable;
    // 1040: Nova Lite 2 receiver (BatteryHub issue #47, untested there).
    (vendor == VENDOR && matches!(product, 0x1098 | 0x100F | 0x1040)).then_some("GameSir Nova")
}

fn read_cyclone(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let found = collections.iter().find(|c| {
        c.vendor == vendor && c.product == product && c.usage_page == 0xFFF0 && c.usage == 0x0040 && c.output_len >= 3 && c.input_len >= 37
    });
    let Some(collection) = found else { return Ok(None) };
    let device = Device::open(collection)?;
    device.flush();
    if !device.write(&[0x0F, 0x03], WRITE_WAIT)? {
        return Ok(None);
    }
    first_reply(&device, parse_cyclone)
}

fn read_nova(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let found = collections.iter().find(|c| c.vendor == vendor && c.product == product && c.usage_page == 0xFF7A && c.output_len >= 3);
    let Some(collection) = found else { return Ok(None) };
    let device = Device::open(collection)?;
    device.flush();
    // Report id 0: the collection does not number its reports.
    if !device.write(&[0x00, 0x01, 0x01], WRITE_WAIT)? {
        return Ok(None);
    }
    first_reply(&device, parse_nova)
}

/// Reads until a report `parse` takes, for [`WAIT`] at most.
fn first_reply(device: &Device, parse: fn(&[u8]) -> Option<Reply>) -> io::Result<Option<Reading>> {
    let deadline = Instant::now() + WAIT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Some(report) = device.read(left)? else { break };
        match parse(&report) {
            Some(Reply::Battery(reading)) => return Ok(Some(reading)),
            Some(Reply::NoPercentage) => return Ok(None),
            None => {}
        }
    }
    Ok(None)
}

/// A reply to the battery request.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Battery(Reading),
    /// Charging, which some pads tell without a percentage.
    NoPercentage,
}

/// Input report `12` of the Cyclone 2 family (`[0]` is its id).
fn parse_cyclone(report: &[u8]) -> Option<Reply> {
    if report.first() != Some(&0x12) {
        return None;
    }
    let (&charging, &percent) = (report.get(35)?, report.get(36)?);
    (percent <= 100).then_some(Reply::Battery(Reading { percent, charging: charging == 1 }))
}

/// The "device info" reply of a Nova (`[0]` is `01`; state frames `F4 00 …`
/// arrive on the same collection and are skipped).
fn parse_nova(report: &[u8]) -> Option<Reply> {
    if report.first() != Some(&0x01) {
        return None;
    }
    match *report.get(18)? {
        0xFF => Some(Reply::NoPercentage),
        percent if percent <= 100 => Some(Reply::Battery(Reading { percent, charging: false })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replies of a G7 Pro 8K, measured on its receiver (`10C6`) and on the
    /// cable (`10C5`), both full.
    const RECEIVER: &str = "12-80-80-80-80-0F-00-00-00-00-CE-CA-00-FF-FF-FE-FF-03-00-F3-FF-4D-20-2E-FF-00-BB-1F-CE-D7-0A-00-00-00-00-00-64-00-00-00-00-00-00-00-00-00-00-00-00-00-00-00-00-00-7C-7F-85-7E-0F-0C-00-06-03-00";
    const CABLE: &str = "12-80-80-80-80-0F-00-00-00-00-7F-B1-00-FE-FF-03-00-FE-FF-E6-02-F6-1B-11-0B-00-DD-5B-E9-BA-1B-00-00-00-00-00-64-00-00-00-00-00-00-00-00-00-00-00-00-00-00-00-00-00-7C-7F-85-7E-0F-00-00-00-00-00";

    fn bytes(hex: &str) -> Vec<u8> {
        hex.split('-').map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    fn battery(percent: u8, charging: bool) -> Option<Reply> {
        Some(Reply::Battery(Reading { percent, charging }))
    }

    #[test]
    fn the_cyclone_family_tells_it_in_report_12() {
        assert_eq!(parse_cyclone(&bytes(RECEIVER)), battery(100, false));
        assert_eq!(parse_cyclone(&bytes(CABLE)), battery(100, false));
        let mut low = bytes(RECEIVER);
        (low[35], low[36]) = (1, 42);
        assert_eq!(parse_cyclone(&low), battery(42, true));
        let mut other = bytes(RECEIVER);
        other[0] = 0x10;
        assert_eq!(parse_cyclone(&other), None, "the event reports carry no battery");
        let mut odd = bytes(RECEIVER);
        odd[36] = 0xFF;
        assert_eq!(parse_cyclone(&odd), None);
        assert_eq!(parse_cyclone(&bytes(RECEIVER)[..36]), None, "too short");
    }

    #[test]
    fn a_nova_tells_it_in_its_device_info() {
        let mut info = vec![0u8; 64];
        info[0] = 0x01;
        info[18] = 57;
        assert_eq!(parse_nova(&info), battery(57, false));
        info[18] = 0xFF;
        assert_eq!(parse_nova(&info), Some(Reply::NoPercentage));
        info[18] = 101;
        assert_eq!(parse_nova(&info), None);
        assert_eq!(parse_nova(&[0xF4, 0x00, 0x01]), None, "a state frame");
    }

    #[test]
    fn receivers_without_their_pad_are_not_read() {
        assert_eq!(cyclone_model(VENDOR, 0x10C6), Some("GameSir G7 Pro 8K"));
        assert_eq!(cyclone_model(VENDOR, 0x100B), Some("GameSir Cyclone 2"));
        assert_eq!(cyclone_model(VENDOR, 0x0575), None);
        assert_eq!(nova_model(VENDOR, 0x1098), Some("GameSir Nova"));
        assert_eq!(cyclone_model(0x045E, 0x10C6), None);
    }
}
