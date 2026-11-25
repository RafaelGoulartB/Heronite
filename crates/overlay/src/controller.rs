//! Controller battery (sysfs or XInput) and Linux Bluetooth disconnect.

#[cfg(any(not(windows), test))]
use std::path::Path;
#[cfg(not(windows))]
use std::process::Command;
#[cfg(not(windows))]
use std::time::Duration;

#[cfg(not(windows))]
use heronite_data::proc::Run;

use crate::input::ControllerId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// 0..=100.
    pub percent: u8,
    pub charging: bool,
}

/// The battery of the controller behind an evdev node (`/dev/input/event7`),
/// when its driver reports one (hid-playstation, hid-nintendo, xpadneo...).
/// Blocking reads of sysfs.
#[cfg(not(windows))]
pub fn battery(id: &ControllerId) -> Option<Battery> {
    let ControllerId::Evdev(node) = id else { return None };
    let name = node.file_name()?;
    let device = std::fs::canonicalize(Path::new("/sys/class/input").join(name).join("device")).ok()?;
    // The power supply hangs off the HID device, a few levels up the input node.
    device.ancestors().take(5).find_map(|dir| read_supply(&dir.join("power_supply")))
}

#[cfg(any(not(windows), test))]
fn read_supply(dir: &Path) -> Option<Battery> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if let Some(b) = parse_supply(&entry.path()) {
            return Some(b);
        }
    }
    None
}

#[cfg(any(not(windows), test))]
fn parse_supply(dir: &Path) -> Option<Battery> {
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).ok().map(|s| s.trim().to_string());
    let percent = match read("capacity").and_then(|s| s.parse::<u8>().ok()) {
        Some(p) => p.min(100),
        None => level_percent(&read("capacity_level")?)?,
    };
    let charging = read("status").is_some_and(|s| s == "Charging" || s == "Full");
    Some(Battery { percent, charging })
}

/// `capacity_level` of drivers without a percentage.
#[cfg(any(not(windows), test))]
fn level_percent(level: &str) -> Option<u8> {
    Some(match level {
        "Full" => 100,
        "High" => 75,
        "Normal" => 50,
        "Low" => 20,
        "Critical" => 5,
        _ => return None,
    })
}

/// Disconnects a Bluetooth controller (`bluetoothctl disconnect <addr>`).
/// Blocking.
#[cfg(not(windows))]
pub fn disconnect(address: &str) -> Result<(), String> {
    let out = Run::new()
        .timeout(Duration::from_secs(8))
        .run(Command::new("bluetoothctl").args(["disconnect", address]))
        .map_err(|e| format!("bluetoothctl: {e}"))?;
    if out.success() {
        return Ok(());
    }
    let err = out.stderr_text();
    let err = if err.trim().is_empty() { out.stdout_text() } else { err };
    Err(err.trim().to_string())
}

/// The charge of a controller (`None` when it tells none: wired, unknown,
/// asleep). Native failures are logged. Blocking, up to a couple of seconds.
/// - one a battery driver knows (`heronite_platform::pad_battery`): its
///   driver's reading, or none. XInput calls most of these wired, or "full"
///   whatever their charge;
/// - another XInput controller on batteries: its percentage by
///   Windows.Gaming.Input, else XInput's coarse level (0, 33, 67 or 100 %,
///   without charging).
#[cfg(windows)]
pub fn battery(id: &ControllerId) -> Option<Battery> {
    use heronite_platform::{gamepad, pad_battery};
    let index = match *id {
        ControllerId::Hid { vendor, product } => return driver_battery(vendor, product),
        ControllerId::XInput(index) => index,
        ControllerId::Evdev(_) => {
            log::warn!("overlay controller: not a Windows controller: {id:?}");
            return None;
        }
    };
    let ids = match gamepad::product(index) {
        Ok(ids) => ids,
        Err(error) => {
            log::warn!("overlay controller: XInputGetCapabilitiesEx slot {}: {error}", index + 1);
            None
        }
    };
    if let Some((vendor, product)) = ids.filter(|&(v, p)| pad_battery::knows(v, p)) {
        return driver_battery(vendor, product);
    }
    let coarse = match gamepad::battery(index) {
        Ok(percent) => percent?,
        Err(error) => {
            log::warn!("overlay controller: XInputGetBatteryInformation slot {}: {error}", index + 1);
            return None;
        }
    };
    let precise = ids.and_then(|(vendor, product)| match pad_battery::xbox(vendor, product) {
        Ok(reading) => reading,
        Err(error) => {
            log::warn!("overlay controller: Windows.Gaming.Input battery of {vendor:04x}:{product:04x}: {error}");
            None
        }
    });
    Some(match precise {
        Some(r) => Battery { percent: r.percent, charging: r.charging },
        None => Battery { percent: coarse, charging: false },
    })
}

/// The battery driver's reading of `vendor:product`.
#[cfg(windows)]
fn driver_battery(vendor: u16, product: u16) -> Option<Battery> {
    match heronite_platform::pad_battery::read(vendor, product) {
        Ok(reading) => reading.map(|r| Battery { percent: r.percent, charging: r.charging }),
        Err(error) => {
            log::warn!("overlay controller: battery of {vendor:04x}:{product:04x}: {error}");
            None
        }
    }
}

#[cfg(windows)]
pub fn disconnect(_address: &str) -> Result<(), String> {
    Err("XInput does not support disconnecting controllers".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_capacity_or_level() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let supply = dir.join("ps-controller-battery-aa");
        std::fs::create_dir_all(&supply).unwrap();
        std::fs::write(supply.join("capacity"), "62\n").unwrap();
        std::fs::write(supply.join("status"), "Discharging\n").unwrap();
        assert_eq!(read_supply(&dir), Some(Battery { percent: 62, charging: false }));
        std::fs::remove_file(supply.join("capacity")).unwrap();
        std::fs::write(supply.join("capacity_level"), "Low").unwrap();
        std::fs::write(supply.join("status"), "Charging").unwrap();
        assert_eq!(read_supply(&dir), Some(Battery { percent: 20, charging: true }));
    }
}
