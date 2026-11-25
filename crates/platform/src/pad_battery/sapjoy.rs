//! GameSir Tarantula Pro and NovaPro (sold as Super Nova, `T4NPro`), which
//! XInput sees as wired Xbox 360 pads, read with the "device info" frame of
//! GameSir's SAPJoy library (BatteryHub's `gamesir_sapjoy.cpp` and
//! `gamesir_sapjoy_protocol.cpp`, `docs/gamesir_sapjoy.md`):
//!
//! - Vendor collection page `FFA0`: output report `07` (32 bytes), input
//!   report `06` (64 bytes), no feature report.
//! - Request GetDeviceInfo, `07 03 0A 01` (report id, then `len cmd sub`, the
//!   length counting itself), zero padded: it only asks.
//! - Reply `06 len 0A 01`, byte 4 the device id, 5–6 the firmware, 7–12 the
//!   MAC address, 13 the battery in percent (`FF` while charging, with no
//!   percentage then; above 100 is taken as 100, as GameSir's client does),
//!   14 the device type. Gamepad state frames arrive on the same collection
//!   and are told apart by the whole header.
//!
//! Protocol from the traffic of GameSir's own client on a NovaPro and its
//! `main.js` decoder; BatteryHub verified it on a NovaPro on its receiver
//! (96 % as GameSir's client showed). The cable (`100E`) and the Tarantula
//! Pro (`1050`) answer the same frame in GameSir's client, untested there.

use std::io;
use std::time::{Duration, Instant};

use super::gamesir::VENDOR;
use super::{Driver, Reading};
use crate::hid::{Collection, Device};

pub(super) const DRIVERS: &[Driver] = &[Driver { model, read }];

/// The GetDeviceInfo request (report id `07`); [`Device::write`] pads it to
/// the 32 bytes of the report.
const DEVICE_INFO: [u8; 4] = [0x07, 0x03, 0x0A, 0x01];

/// How long the reply may take (BatteryHub's wait). A receiver whose pad
/// went to sleep never answers.
const WAIT: Duration = Duration::from_millis(900);
const WRITE_WAIT: Duration = Duration::from_millis(500);

fn model(vendor: u16, product: u16) -> Option<&'static str> {
    if vendor != VENDOR {
        return None;
    }
    match product {
        // On the cable.
        0x100E => Some("GameSir NovaPro"),
        // The receiver of both, which tell the same frame.
        0x103E => Some("GameSir NovaPro / Tarantula Pro"),
        0x1050 => Some("GameSir Tarantula Pro"),
        _ => None,
    }
}

/// The collection GameSir's library talks to: page `FFA0` with exactly its
/// report lengths (the gamepad and keyboard collections time out or refuse
/// the request).
fn is_sapjoy(c: &Collection, vendor: u16, product: u16) -> bool {
    c.vendor == vendor && c.product == product && c.usage_page == 0xFFA0 && c.output_len == 32 && c.input_len == 64 && c.feature_len == 0
}

fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let Some(collection) = collections.iter().find(|c| is_sapjoy(c, vendor, product)) else { return Ok(None) };
    let device = Device::open(collection)?;
    device.flush();
    if !device.write(&DEVICE_INFO, WRITE_WAIT)? {
        return Ok(None);
    }
    let deadline = Instant::now() + WAIT;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Some(report) = device.read(left)? else { break };
        match parse_device_info(&report) {
            Some(Reply::Battery(reading)) => return Ok(Some(reading)),
            Some(Reply::NoPercentage) => return Ok(None),
            None => {}
        }
    }
    Ok(None)
}

/// A reply to GetDeviceInfo.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Battery(Reading),
    /// Charging: the pad tells no percentage then.
    NoPercentage,
}

/// Input report `06`, the GetDeviceInfo reply (`[0]` is its id); `None` for
/// anything else.
fn parse_device_info(report: &[u8]) -> Option<Reply> {
    // Byte 14 (the type) is the last field: a shorter report is no reply.
    if report.len() < 15 || report[0] != 0x06 || report[2] != 0x0A || report[3] != 0x01 {
        return None;
    }
    // The length counts itself and leaves the report id out.
    let len = usize::from(report[1]);
    if len < 14 || len + 1 > report.len() {
        return None;
    }
    Some(match report[13] {
        0xFF => Reply::NoPercentage,
        percent => Reply::Battery(Reading { percent: percent.min(100), charging: false }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply as BatteryHub's tests build it (the MAC a placeholder).
    fn reply(battery: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[..7].copy_from_slice(&[0x06, 0x0E, 0x0A, 0x01, 0x03, 0x01, 0x16]);
        r[7..13].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        r[13] = battery;
        r
    }

    fn battery(percent: u8) -> Option<Reply> {
        Some(Reply::Battery(Reading { percent, charging: false }))
    }

    #[test]
    fn the_request_only_asks_for_the_device_info() {
        assert_eq!(DEVICE_INFO, [0x07, 0x03, 0x0A, 0x01]);
    }

    #[test]
    fn the_device_info_tells_the_battery() {
        assert_eq!(parse_device_info(&reply(0x60)), battery(96));
        assert_eq!(parse_device_info(&reply(0)), battery(0));
        assert_eq!(parse_device_info(&reply(100)), battery(100));
        assert_eq!(parse_device_info(&reply(0xD0)), battery(100), "above 100 is clamped, not rejected");
        assert_eq!(parse_device_info(&reply(0xFF)), Some(Reply::NoPercentage), "charging");
    }

    #[test]
    fn other_frames_are_skipped() {
        let mut state = reply(96);
        state[0] = 0x12;
        assert_eq!(parse_device_info(&state), None, "a gamepad state frame");
        let mut command = reply(96);
        command[2] = 0x01;
        assert_eq!(parse_device_info(&command), None, "another command");
        let mut sub = reply(96);
        sub[3] = 0x02;
        assert_eq!(parse_device_info(&sub), None, "another sub-command");
        assert_eq!(parse_device_info(&reply(96)[..14]), None, "too short");
        for len in [0, 13, 64, 255] {
            let mut bad = reply(96);
            bad[1] = len;
            assert_eq!(parse_device_info(&bad), None, "length {len}");
        }
        assert_eq!(parse_device_info(&[]), None);
        assert_eq!(parse_device_info(&[0u8; 5]), None);
    }

    #[test]
    fn only_the_reply_among_the_noise_is_taken() {
        // What BatteryHub's driver test queues after the request.
        let mut other_command = reply(96);
        other_command[2] = 0x03;
        let mut too_long = reply(96);
        too_long[1] = 64;
        let frames = [vec![0u8; 5], other_command, too_long, reply(96)];
        let first = frames.iter().find_map(|f| parse_device_info(f));
        assert_eq!(first, battery(96));
    }

    #[test]
    fn only_the_sapjoy_collection_is_used() {
        let good = Collection::new(VENDOR, 0x103E, 0xFFA0, 0x0001, (64, 32, 0));
        assert!(is_sapjoy(&good, VENDOR, 0x103E));
        let wrong = [
            Collection::new(0x0001, 0x103E, 0xFFA0, 0x0001, (64, 32, 0)),
            Collection::new(VENDOR, 0x1098, 0xFFA0, 0x0001, (64, 32, 0)),
            Collection::new(VENDOR, 0x103E, 0x0001, 0x0005, (64, 32, 0)),
            Collection::new(VENDOR, 0x103E, 0xFFA0, 0x0001, (15, 32, 0)),
            Collection::new(VENDOR, 0x103E, 0xFFA0, 0x0001, (64, 64, 0)),
            Collection::new(VENDOR, 0x103E, 0xFFA0, 0x0001, (64, 32, 32)),
        ];
        for c in &wrong {
            assert!(!is_sapjoy(c, VENDOR, 0x103E), "{c:?}");
        }
    }

    #[test]
    fn the_sapjoy_pads_are_known() {
        assert_eq!(model(VENDOR, 0x100E), Some("GameSir NovaPro"));
        assert_eq!(model(VENDOR, 0x103E), Some("GameSir NovaPro / Tarantula Pro"));
        assert_eq!(model(VENDOR, 0x1050), Some("GameSir Tarantula Pro"));
        // The Nova and Cyclone families belong to `gamesir`.
        assert_eq!(model(VENDOR, 0x1098), None);
        assert_eq!(model(VENDOR, 0x100B), None);
        assert_eq!(model(0x045E, 0x103E), None);
    }
}
