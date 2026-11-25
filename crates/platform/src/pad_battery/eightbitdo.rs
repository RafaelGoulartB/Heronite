//! 8BitDo controllers (vendor `2DC8`), read two ways (BatteryHub's
//! `eightbitdo.cpp`):
//!
//! - The vendor channel of the 2.4 GHz receiver or the cable: collection
//!   `FF7A:0001` or `FFA0:0001` (64-byte reports, `81` out, `02` in). It
//!   answers read-only status queries, in XInput mode too (where XInput sees
//!   the receiver as a wired Xbox 360 pad):
//!   - `81 04 00 01` (info) → `02 04 04 00 00 01 …`, bytes 22..23 the internal
//!     product id, little-endian: `6013` (or `3109`) the receiver, any other
//!     the pad on its cable, charging;
//!   - `81 04 20 01` (link) → `02 04 04 00 20 01 …`, byte 18 non-zero while
//!     the receiver has its pad;
//!   - `81 04 21 01 00 00 06` (battery) → `02 04 04 00 21 01 06 00 …`, byte 18
//!     the battery in percent (1..=100) in bits 0..6, bit 7 charging.
//!
//!   Protocol: <https://github.com/MiguelSanzBr/8bitdo-ultimate2-linux>
//!   (`docs/PROTOCOL.md`).
//! - The input reports the pad sends anyway (D-input or Bluetooth), listened
//!   to: collection `0001:0005` or `0001:0004`, report `01` or `04`, byte 14
//!   the battery in percent in bits 0..6, bit 7 charging. The Ultimate 2
//!   Wireless (`6012`) carries it in its 34-byte reports only (firmware 1.03
//!   on; 1.02 sends 12 bytes without it). Layout: SDL,
//!   `src/joystick/hidapi/SDL_hidapi_8bitdo.c` (`HandleStatePacket`).
//!
//! Left out from BatteryHub: it gets feature report `06` of the Pro 3, Pro 2,
//! SN30 Pro and SF30 Pro before listening, without saying what that does (it
//! may switch their report mode), so here those pads tell their battery only
//! when they already send it; it also keeps the last reading of an Ultimate 2
//! put on its dock (the receiver then goes idle, `6013`) and shows it asleep,
//! where this tells none.

use std::io;
use std::time::{Duration, Instant};

use super::{Driver, Reading};
use crate::hid::{Collection, Device};

pub(super) const VENDOR: u16 = 0x2DC8;

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

/// The whole read: the vendor queries, then listening.
const BUDGET: Duration = Duration::from_millis(1800);
/// How long one vendor query may take.
const QUERY_WAIT: Duration = Duration::from_millis(300);
/// How long to listen for a report with the battery.
const LISTEN_WAIT: Duration = Duration::from_millis(600);

/// The internal product ids of the receiver (the info reply).
const RECEIVER_IDS: [u16; 2] = [0x6013, 0x3109];

const INFO: ([u8; 4], [u8; 2]) = ([0x81, 0x04, 0x00, 0x01], [0x00, 0x01]);
const LINK: ([u8; 4], [u8; 2]) = ([0x81, 0x04, 0x20, 0x01], [0x20, 0x01]);
const BATTERY: ([u8; 7], [u8; 2]) = ([0x81, 0x04, 0x21, 0x01, 0x00, 0x00, 0x06], [0x21, 0x01]);

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    if vendor != VENDOR {
        return None;
    }
    Some(match product {
        // On its receiver (XInput), on the cable or Bluetooth (D-input), its
        // receiver idle (on the dock).
        0x310B | 0x6012 | 0x6013 => "8BitDo Ultimate 2 Wireless",
        0x202F | 0x202E => "8BitDo Ultimate 3-mode",
        0x301B | 0x301A | 0x310A => "8BitDo Ultimate 2C Wireless",
        0x3012 | 0x3013 | 0x3109 => "8BitDo Ultimate 2.4G",
        0x6007 => "8BitDo Ultimate Bluetooth",
        0x6009 => "8BitDo Pro 3",
        0x6003 | 0x6006 => "8BitDo Pro 2",
        0x6001 | 0x6101 => "8BitDo SN30 Pro",
        0x6000 | 0x6100 => "8BitDo SF30 Pro",
        _ => return None,
    })
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let deadline = Instant::now() + BUDGET;
    let ours = |c: &&Collection| c.vendor == vendor && c.product == product;
    // An error is returned only when nothing could be asked at all.
    let mut failure = None;
    let mut asked = false;

    let vendor_channel = collections
        .iter()
        .filter(ours)
        .filter(|c| matches!(c.usage_page, 0xFF7A | 0xFFA0) && c.usage == 0x0001 && c.input_len >= 64 && c.output_len >= 64);
    for collection in vendor_channel {
        match Device::open(collection).and_then(|device| ask(&device, product, deadline)) {
            Ok(Answer::Battery(reading)) => return Ok(Some(reading)),
            Ok(Answer::Nothing) => return Ok(None),
            Ok(Answer::NotAnswered) => asked = true,
            Err(e) => {
                failure.get_or_insert(e);
            }
        }
    }

    let gamepad =
        collections.iter().filter(ours).filter(|c| c.usage_page == 0x0001 && matches!(c.usage, 0x0005 | 0x0004) && c.input_len >= 15);
    for collection in gamepad {
        match Device::open_read_only(collection).and_then(|device| listen(&device, product, deadline)) {
            Ok(Some(reading)) => return Ok(Some(reading)),
            Ok(None) => asked = true,
            Err(e) => {
                failure.get_or_insert(e);
            }
        }
    }

    match failure {
        Some(e) if !asked => Err(e),
        _ => Ok(None),
    }
}

/// What the vendor channel told.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Battery(Reading),
    /// Nothing, and nothing else will: a receiver without its pad, a pad on
    /// its cable without a percentage.
    Nothing,
    /// No battery: listen to the pad's reports instead.
    NotAnswered,
}

/// The vendor queries, in BatteryHub's order.
fn ask(device: &Device, product: u16, deadline: Instant) -> io::Result<Answer> {
    let on_receiver = query(device, &INFO.0, INFO.1, deadline)?.as_deref().and_then(internal_id).map(|id| RECEIVER_IDS.contains(&id));
    if on_receiver == Some(false) {
        // On its cable, charging.
        let battery = query(device, &BATTERY.0, BATTERY.1, deadline)?.as_deref().and_then(parse_battery);
        return Ok(match battery {
            Some((percent, _)) => Answer::Battery(Reading { percent, charging: true }),
            None => Answer::Nothing,
        });
    }
    let linked = query(device, &LINK.0, LINK.1, deadline)?.as_deref().map(parse_linked);
    if linked != Some(false) {
        if let Some((percent, charging)) = query(device, &BATTERY.0, BATTERY.1, deadline)?.as_deref().and_then(parse_battery) {
            return Ok(Answer::Battery(Reading { percent, charging: charging && percent < 100 }));
        }
    }
    let receiver = on_receiver == Some(true) || RECEIVER_IDS.contains(&product);
    Ok(if receiver { Answer::Nothing } else { Answer::NotAnswered })
}

/// Sends `request` and waits for the reply to command `cmd`, for
/// [`QUERY_WAIT`] at most (and not past `deadline`).
fn query(device: &Device, request: &[u8], cmd: [u8; 2], deadline: Instant) -> io::Result<Option<Vec<u8>>> {
    let end = deadline.min(Instant::now() + QUERY_WAIT);
    let Some(left) = left_until(end) else { return Ok(None) };
    device.flush();
    if !device.write(request, left)? {
        return Ok(None);
    }
    while let Some(left) = left_until(end) {
        let Some(report) = device.read(left)? else { break };
        if report.len() < 19 {
            // As BatteryHub: not this channel's 64-byte reports.
            break;
        }
        if reply_to(&report, cmd).is_some() {
            return Ok(Some(report));
        }
    }
    Ok(None)
}

/// Listens for a gamepad report with the battery, for [`LISTEN_WAIT`] at most
/// (and not past `deadline`).
fn listen(device: &Device, product: u16, deadline: Instant) -> io::Result<Option<Reading>> {
    let end = deadline.min(Instant::now() + LISTEN_WAIT);
    while let Some(left) = left_until(end) {
        let Some(report) = device.read(left)? else { break };
        if let Some(reading) = parse_gamepad(&report, product) {
            return Ok(Some(reading));
        }
    }
    Ok(None)
}

/// The time left until `end`, when some is.
fn left_until(end: Instant) -> Option<Duration> {
    end.checked_duration_since(Instant::now()).filter(|left| !left.is_zero())
}

/// `report` when it is the vendor channel's reply to command `cmd`
/// (`02 04 … cmd`), long enough for byte 18.
fn reply_to(report: &[u8], cmd: [u8; 2]) -> Option<&[u8]> {
    (report.len() >= 19 && report[0] == 0x02 && report[1] == 0x04 && report[4..6] == cmd).then_some(report)
}

/// The internal product id in the info reply (bytes 22..23).
fn internal_id(reply: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes([*reply.get(22)?, *reply.get(23)?]))
}

/// Whether the receiver has its pad (link reply, byte 18).
fn parse_linked(reply: &[u8]) -> bool {
    reply.get(18).is_some_and(|&b| b != 0)
}

/// The percentage (1..=100) and charging bit of the battery reply (byte 18).
fn parse_battery(reply: &[u8]) -> Option<(u8, bool)> {
    let byte = *reply.get(18)?;
    let percent = byte & 0x7F;
    (1..=100).contains(&percent).then_some((percent, byte & 0x80 != 0))
}

/// The battery in a gamepad input report (`[0]` is its id, `01` or `04`).
fn parse_gamepad(report: &[u8], product: u16) -> Option<Reading> {
    // The Ultimate 2 Wireless sends short reports without it on older firmware.
    let min_len = if product == 0x6012 { 34 } else { 15 };
    if report.len() < min_len || !matches!(report[0], 0x01 | 0x04) {
        return None;
    }
    let percent = report[14] & 0x7F;
    (percent <= 100).then_some(Reading { percent, charging: report[14] & 0x80 != 0 && percent < 100 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vendor reply to `cmd`, 64 bytes, with byte 18 `value`.
    fn reply(cmd: [u8; 2], value: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..6].copy_from_slice(&[0x02, 0x04, 0x04, 0x00, cmd[0], cmd[1]]);
        r[18] = value;
        r
    }

    #[test]
    fn vendor_replies_are_told_by_their_command() {
        let link = reply(LINK.1, 1);
        assert!(reply_to(&link, LINK.1).is_some());
        assert!(reply_to(&link, BATTERY.1).is_none());
        let mut other = link.clone();
        other[0] = 0x03;
        assert!(reply_to(&other, LINK.1).is_none());
        assert!(reply_to(&link[..18], LINK.1).is_none(), "too short");
    }

    #[test]
    fn the_link_reply_tells_whether_the_pad_is_there() {
        assert!(parse_linked(&reply(LINK.1, 1)));
        assert!(!parse_linked(&reply(LINK.1, 0)));
    }

    #[test]
    fn the_battery_reply_tells_percent_and_charging() {
        let mut r = reply(BATTERY.1, 57);
        r[6] = 0x06;
        assert_eq!(parse_battery(&r), Some((57, false)));
        r[18] = 0x80 | 42;
        assert_eq!(parse_battery(&r), Some((42, true)));
        r[18] = 0;
        assert_eq!(parse_battery(&r), None, "0 % is no reading on this channel");
        r[18] = 101;
        assert_eq!(parse_battery(&r), None);
    }

    #[test]
    fn the_info_reply_tells_receiver_from_cable() {
        let mut r = reply(INFO.1, 0);
        (r[22], r[23]) = (0x13, 0x60);
        assert_eq!(internal_id(&r), Some(0x6013));
        assert!(RECEIVER_IDS.contains(&internal_id(&r).unwrap()));
        (r[22], r[23]) = (0x12, 0x60);
        assert_eq!(internal_id(&r), Some(0x6012));
        assert_eq!(internal_id(&r[..23]), None);
    }

    #[test]
    fn gamepad_reports_carry_it_in_byte_14() {
        let mut r = vec![0u8; 15];
        r[0] = 0x01;
        r[14] = 80;
        assert_eq!(parse_gamepad(&r, 0x6009), Some(Reading { percent: 80, charging: false }));
        r[0] = 0x04;
        r[14] = 0x80 | 30;
        assert_eq!(parse_gamepad(&r, 0x6009), Some(Reading { percent: 30, charging: true }));
        r[14] = 0x80 | 100;
        assert_eq!(parse_gamepad(&r, 0x6009), Some(Reading { percent: 100, charging: false }), "full, not charging");
        r[14] = 0;
        assert_eq!(parse_gamepad(&r, 0x6009), Some(Reading { percent: 0, charging: false }), "0 % is a reading here");
        r[14] = 110;
        assert_eq!(parse_gamepad(&r, 0x6009), None);
        r[14] = 50;
        r[0] = 0x03;
        assert_eq!(parse_gamepad(&r, 0x6009), None, "another report");
        r[0] = 0x01;
        assert_eq!(parse_gamepad(&r[..14], 0x6009), None, "too short");
    }

    #[test]
    fn the_ultimate_2_tells_it_in_34_byte_reports_only() {
        let mut r = vec![0u8; 34];
        r[0] = 0x04;
        r[14] = 64;
        assert_eq!(parse_gamepad(&r, 0x6012), Some(Reading { percent: 64, charging: false }));
        assert_eq!(parse_gamepad(&r[..15], 0x6012), None, "firmware 1.02's short reports");
        assert_eq!(parse_gamepad(&r[..15], 0x6003), Some(Reading { percent: 64, charging: false }));
    }

    #[test]
    fn models_come_from_the_product_id() {
        assert_eq!(model(VENDOR, 0x310B), Some("8BitDo Ultimate 2 Wireless"));
        assert_eq!(model(VENDOR, 0x6003), Some("8BitDo Pro 2"));
        assert_eq!(model(VENDOR, 0x1234), None);
        assert_eq!(model(0x045E, 0x310B), None);
    }
}
