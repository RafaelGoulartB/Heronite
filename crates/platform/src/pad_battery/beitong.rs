//! BEITONG (Betop) KP20 (`20BC:5128`) and KP40D (`20BC:515C`) on their
//! 2.4 GHz receivers, in XInput mode, which sees the receiver as a wired pad
//! (BatteryHub's `beitong.cpp`).
//!
//! The battery comes from the receiver's vendor collection `00FF:0003`
//! (64-byte reports, id `02`) with the "get base info" request BEITONG
//! iControl sends for its home page's battery. Every byte after the report id
//! is scrambled both ways with a fixed nibble cipher, zero padding after the
//! payload excepted:
//! `enc(x) = S[x >> 4] << 4 | (x & 0x0F) ^ 0x08`,
//! `S = 0 A 6 F 1 C 2 9 E 3 7 D 4 B 5 8`.
//! - Request, plain `02 15` (command 5, status; sub-command 1, base info): on
//!   the wire `02 AD 00 00 …`.
//! - Reply, plain `02 15 <battery 0..=100> 00 <charge state>`, 1 charging.
//!   The KP40D keeps the low nibble of the charge state only.
//!
//! The receiver answers without iControl's config mode: nothing else is sent.
//! Other frames on the collection (`02 25` keys, `02 21` sync) only come while
//! iControl is open, and are skipped.
//!
//! Protocol: reverse engineered by BatteryHub from BEITONG iControl 1.0.5
//! (`kp20.js`, `kp40d.js`) and USB traffic of a BTP-KP20 receiver (the cipher
//! checked against 32,000+ decoded bytes). The KP40D was not tested on
//! hardware there.

use std::io;
use std::time::{Duration, Instant};

use super::{Driver, Reading};
use crate::hid::{Collection, Device};

pub(super) const VENDOR: u16 = 0x20BC;

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

const KP20: u16 = 0x5128;
const KP40D: u16 = 0x515C;

const REPORT_ID: u8 = 0x02;
/// Command 5 (status), sub-command 1 (base info).
const GET_BASE_INFO: u8 = 0x15;

/// The whole read.
const BUDGET: Duration = Duration::from_millis(1500);
/// How long a reply may take.
const WAIT: Duration = Duration::from_millis(800);
const WRITE_WAIT: Duration = Duration::from_millis(300);

const NIBBLE: [u8; 16] = [0x0, 0xA, 0x6, 0xF, 0x1, 0xC, 0x2, 0x9, 0xE, 0x3, 0x7, 0xD, 0x4, 0xB, 0x5, 0x8];

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    match (vendor, product) {
        (VENDOR, KP20) => Some("BEITONG KP20"),
        (VENDOR, KP40D) => Some("BEITONG KP40D"),
        _ => None,
    }
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    if model(vendor, product).is_none() {
        return Ok(None);
    }
    let deadline = Instant::now() + BUDGET;
    // An error is returned only when nothing could be asked at all.
    let mut failure = None;
    let mut asked = false;
    let found = collections.iter().filter(|c| {
        c.vendor == vendor && c.product == product && c.usage_page == 0x00FF && c.usage == 0x0003 && c.output_len >= 3 && c.input_len >= 5
    });
    for collection in found {
        match Device::open(collection).and_then(|device| ask(&device, product, deadline)) {
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

/// Sends "get base info" and waits for its reply, [`WAIT`] at most (and not
/// past `deadline`).
fn ask(device: &Device, product: u16, deadline: Instant) -> io::Result<Option<Reading>> {
    device.flush();
    if !device.write(&request(), WRITE_WAIT)? {
        return Ok(None);
    }
    let end = deadline.min(Instant::now() + WAIT);
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) {
        let Some(report) = device.read(left)? else { break };
        if let Some(reply) = parse_base_info(&report, product) {
            return Ok(reply);
        }
    }
    Ok(None)
}

/// A byte as the cipher sends it.
fn encode(x: u8) -> u8 {
    NIBBLE[usize::from(x >> 4)] << 4 | ((x & 0x0F) ^ 0x08)
}

/// A byte the cipher sent.
fn decode(x: u8) -> u8 {
    let hi = NIBBLE.iter().position(|&n| n == x >> 4).unwrap_or(0) as u8;
    hi << 4 | ((x & 0x0F) ^ 0x08)
}

/// The "get base info" request, as sent (the rest is zero padding).
fn request() -> [u8; 2] {
    [REPORT_ID, encode(GET_BASE_INFO)]
}

/// The reply to "get base info" in an input report (`[0]` is its id):
/// `Some(None)` when it is the reply but its battery is out of range.
fn parse_base_info(report: &[u8], product: u16) -> Option<Option<Reading>> {
    if report.len() < 5 || report[0] != REPORT_ID || decode(report[1]) != GET_BASE_INFO {
        return None;
    }
    let percent = decode(report[2]);
    let mut charge = decode(report[4]);
    if product == KP40D {
        charge &= 0x0F;
    }
    Some((percent <= 100).then_some(Reading { percent, charging: charge == 1 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply as received: the plain payload after the report id, enciphered.
    fn wire(plain: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = REPORT_ID;
        for (i, &b) in plain.iter().enumerate() {
            r[1 + i] = encode(b);
        }
        r
    }

    #[test]
    fn the_request_goes_out_enciphered() {
        assert_eq!(request(), [0x02, 0xAD]);
    }

    #[test]
    fn the_cipher_round_trips_every_byte() {
        for x in 0..=u8::MAX {
            assert_eq!(decode(encode(x)), x, "{x:02X}");
            assert_eq!(encode(decode(x)), x, "{x:02X}");
        }
        assert_eq!(encode(0x00), 0x08);
        assert_eq!(decode(0xAD), 0x15);
    }

    #[test]
    fn the_reply_tells_battery_and_charging() {
        let reading = |percent, charging| Some(Some(Reading { percent, charging }));
        assert_eq!(parse_base_info(&wire(&[0x15, 73, 0, 0]), KP20), reading(73, false));
        assert_eq!(parse_base_info(&wire(&[0x15, 40, 0, 1]), KP20), reading(40, true));
        assert_eq!(parse_base_info(&wire(&[0x15, 100, 0, 2]), KP20), reading(100, false));
        assert_eq!(parse_base_info(&wire(&[0x15, 101, 0, 0]), KP20), Some(None), "out of range");
    }

    #[test]
    fn the_kp40d_keeps_the_low_nibble_of_the_charge_state() {
        assert_eq!(parse_base_info(&wire(&[0x15, 55, 0, 0x31]), KP40D), Some(Some(Reading { percent: 55, charging: true })));
        assert_eq!(parse_base_info(&wire(&[0x15, 55, 0, 0x31]), KP20), Some(Some(Reading { percent: 55, charging: false })));
    }

    #[test]
    fn other_frames_are_skipped() {
        assert_eq!(parse_base_info(&wire(&[0x25, 1, 2, 3]), KP20), None, "key state");
        assert_eq!(parse_base_info(&wire(&[0x21, 1, 2, 3]), KP20), None, "sync");
        let mut other = wire(&[0x15, 50, 0, 0]);
        other[0] = 0x01;
        assert_eq!(parse_base_info(&other, KP20), None, "another report");
        assert_eq!(parse_base_info(&wire(&[0x15, 50, 0, 0])[..4], KP20), None, "too short");
    }

    #[test]
    fn models_come_from_the_product_id() {
        assert_eq!(model(VENDOR, KP20), Some("BEITONG KP20"));
        assert_eq!(model(VENDOR, KP40D), Some("BEITONG KP40D"));
        assert_eq!(model(VENDOR, 0x5129), None);
        assert_eq!(model(0x2DC8, KP20), None);
    }
}
