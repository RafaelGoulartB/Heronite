//! Flydigi Vader 5 Pro (`37D7:2401`) and Apex 5 (`37D7:2501`), on their
//! 2.4 GHz receiver or the cable, read on Flydigi's vendor collection
//! `FFA0:0001` (BatteryHub's `flydigi.cpp`).
//!
//! Two read-only queries, `5A A5 <command> …`, in output report `00` (Vader 5
//! Pro, unnumbered) or `03` (Apex 5); replies `5A A5 <command> …` (some
//! firmware keeps an input report id `04` in front):
//! - `5A A5 07 00 00` (architecture): bytes 3, 4 `01 00` on modern firmware;
//! - `5A A5 01 02 00` (info): byte 5 the model id (Vader 5 Pro 130; Apex 5
//!   128, 129, 133, 134, 135), byte 6 the connection (modern firmware: 0
//!   cable, 1 receiver; older: 1 cable, 2 receiver), byte 11 the battery: low
//!   nibble a level 0..=5 (20 % each), high nibble the state (1 charging,
//!   2 full).
//!
//! Protocol: SDL, `src/joystick/hidapi/SDL_hidapi_flydigi.c`, and
//! <https://github.com/HeyOkay/HaloBattery/issues/191> (Windows traces); Apex
//! 5 model id 135 is not in SDL's table, BatteryHub saw it on a real Apex 5
//! on its receiver. Nothing else is sent: no takeover of the controller, no
//! configuration.

use std::io;
use std::time::{Duration, Instant};

use super::{map_to_100, Driver, Reading};
use crate::hid::{Collection, Device};

pub(super) const VENDOR: u16 = 0x37D7;

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

/// The whole read, both queries.
const BUDGET: Duration = Duration::from_millis(1800);
/// How long one reply may take.
const REPLY_WAIT: Duration = Duration::from_millis(800);
const WRITE_WAIT: Duration = Duration::from_millis(200);

const ARCHITECTURE: u8 = 0x07;
const INFO: u8 = 0x01;

struct Model {
    product: u16,
    /// The output report of the queries (0: unnumbered).
    report_id: u8,
    /// The model ids the info reply may tell.
    ids: &'static [u8],
    name: &'static str,
}

const MODELS: &[Model] = &[
    Model { product: 0x2401, report_id: 0x00, ids: &[130], name: "Flydigi Vader 5 Pro" },
    Model { product: 0x2501, report_id: 0x03, ids: &[128, 129, 133, 134, 135], name: "Flydigi Apex 5" },
];

fn find(vendor: u16, product: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| vendor == VENDOR && m.product == product)
}

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    find(vendor, product).map(|m| m.name)
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let Some(model) = find(vendor, product) else { return Ok(None) };
    let deadline = Instant::now() + BUDGET;
    // An error is returned only when nothing could be asked at all.
    let mut failure = None;
    let mut asked = false;
    let found = collections.iter().filter(|c| {
        c.vendor == vendor && c.product == product && c.usage_page == 0xFFA0 && c.usage == 0x0001 && c.input_len >= 12 && c.output_len >= 6
    });
    for collection in found {
        match Device::open(collection).and_then(|device| ask(&device, model, deadline)) {
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

/// Both queries, on one opened collection.
fn ask(device: &Device, model: &Model, deadline: Instant) -> io::Result<Option<Reading>> {
    device.flush();
    let request = |command, arg| [model.report_id, 0x5A, 0xA5, command, arg, 0];
    if !device.write(&request(ARCHITECTURE, 0), WRITE_WAIT)? {
        return Ok(None);
    }
    let modern = reply(device, ARCHITECTURE, deadline)?.is_some_and(|r| is_modern(&r));
    if !device.write(&request(INFO, 2), WRITE_WAIT)? {
        return Ok(None);
    }
    Ok(reply(device, INFO, deadline)?.and_then(|r| parse_info(&r, model.ids, modern)))
}

/// The reply to `command` (from `5A A5`), waiting [`REPLY_WAIT`] at most (and
/// not past `deadline`).
fn reply(device: &Device, command: u8, deadline: Instant) -> io::Result<Option<Vec<u8>>> {
    let end = deadline.min(Instant::now() + REPLY_WAIT);
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) {
        let Some(report) = device.read(left)? else { break };
        if let Some(reply) = reply_to(&report, command) {
            return Ok(Some(reply.to_vec()));
        }
    }
    Ok(None)
}

/// The reply to `command` in an input report, from its `5A A5`: some firmware
/// keeps a report id `04` in front of it.
fn reply_to(report: &[u8], command: u8) -> Option<&[u8]> {
    let reply = match report {
        [0x04, 0x5A, 0xA5, _, ..] => &report[1..],
        _ => report,
    };
    (reply.len() >= 5 && reply[..3] == [0x5A, 0xA5, command]).then_some(reply)
}

/// Whether the architecture reply is modern firmware's (bytes 3, 4 `01 00`).
fn is_modern(reply: &[u8]) -> bool {
    reply.get(3..5) == Some(&[0x01, 0x00])
}

/// The battery in the info reply, when the model id is one of `ids` and the
/// fields are in range.
fn parse_info(reply: &[u8], ids: &[u8], modern: bool) -> Option<Reading> {
    if reply.len() < 12 || !ids.contains(&reply[5]) {
        return None;
    }
    let (level, state) = (reply[11] & 0x0F, reply[11] >> 4);
    if state > 2 || level > 5 {
        return None;
    }
    let connection = reply[6];
    let known = if modern { connection <= 1 } else { matches!(connection, 1 | 2) };
    if !known {
        return None;
    }
    // Full is 100 whatever the level; otherwise 0..=5 is 20 % a step.
    let percent = if state == 2 { 100 } else { map_to_100(level.into(), 0, 5) };
    Some(Reading { percent, charging: state == 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const APEX: &[u8] = &[128, 129, 133, 134, 135];

    /// An info reply: model id, connection, battery byte.
    fn info(id: u8, connection: u8, battery: u8) -> Vec<u8> {
        let mut r = vec![0u8; 32];
        r[..3].copy_from_slice(&[0x5A, 0xA5, INFO]);
        (r[3], r[4], r[5], r[6], r[11]) = (2, 0, id, connection, battery);
        r
    }

    #[test]
    fn replies_are_found_with_or_without_a_report_id() {
        let plain = [0x5A, 0xA5, 0x07, 0x01, 0x00, 0x00];
        assert_eq!(reply_to(&plain, ARCHITECTURE), Some(&plain[..]));
        let numbered = [0x04, 0x5A, 0xA5, 0x07, 0x01, 0x00, 0x00];
        assert_eq!(reply_to(&numbered, ARCHITECTURE), Some(&plain[..]));
        assert_eq!(reply_to(&plain, INFO), None, "another command");
        assert_eq!(reply_to(&[0x5A, 0xA5, 0x07, 0x01], ARCHITECTURE), None, "too short");
        assert_eq!(reply_to(&[0xEF, 0x00, 0x00, 0x00, 0x00], ARCHITECTURE), None, "a state report");
    }

    #[test]
    fn the_architecture_reply_tells_modern_firmware() {
        assert!(is_modern(&[0x5A, 0xA5, 0x07, 0x01, 0x00]));
        assert!(!is_modern(&[0x5A, 0xA5, 0x07, 0x00, 0x00]));
        assert!(!is_modern(&[0x5A, 0xA5, 0x07, 0x01, 0x01]));
    }

    #[test]
    fn the_info_reply_tells_level_and_state() {
        assert_eq!(parse_info(&info(130, 1, 0x03), &[130], true), Some(Reading { percent: 60, charging: false }));
        assert_eq!(parse_info(&info(135, 0, 0x14), APEX, true), Some(Reading { percent: 80, charging: true }));
        assert_eq!(parse_info(&info(128, 1, 0x23), APEX, true), Some(Reading { percent: 100, charging: false }), "full");
        assert_eq!(parse_info(&info(128, 1, 0x00), APEX, true), Some(Reading { percent: 0, charging: false }));
        assert_eq!(parse_info(&info(128, 1, 0x06), APEX, true), None, "level out of range");
        assert_eq!(parse_info(&info(128, 1, 0x32), APEX, true), None, "unknown state");
    }

    #[test]
    fn only_known_models_are_read() {
        assert_eq!(parse_info(&info(130, 1, 0x05), APEX, true), None, "a Vader on an Apex id");
        assert_eq!(parse_info(&info(127, 1, 0x05), APEX, true), None);
        assert_eq!(parse_info(&info(130, 1, 0x05)[..11], &[130], true), None, "too short");
    }

    #[test]
    fn the_connection_byte_depends_on_the_firmware() {
        let ok = Some(Reading { percent: 40, charging: false });
        assert_eq!(parse_info(&info(130, 0, 0x02), &[130], true), ok, "modern, cable");
        assert_eq!(parse_info(&info(130, 1, 0x02), &[130], true), ok, "modern, receiver");
        assert_eq!(parse_info(&info(130, 2, 0x02), &[130], true), None);
        assert_eq!(parse_info(&info(130, 0, 0x02), &[130], false), None);
        assert_eq!(parse_info(&info(130, 1, 0x02), &[130], false), ok, "older, cable");
        assert_eq!(parse_info(&info(130, 2, 0x02), &[130], false), ok, "older, receiver");
        assert_eq!(parse_info(&info(130, 3, 0x02), &[130], false), None);
    }

    #[test]
    fn models_come_from_the_product_id() {
        assert_eq!(model(VENDOR, 0x2401), Some("Flydigi Vader 5 Pro"));
        assert_eq!(model(VENDOR, 0x2501), Some("Flydigi Apex 5"));
        assert_eq!(model(VENDOR, 0x2402), None);
        assert_eq!(model(0x2DC8, 0x2401), None);
    }
}
