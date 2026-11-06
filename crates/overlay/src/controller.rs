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

/// Coarse XInput charge, or `None` for wired/unknown/disconnected devices.
/// XInput does not report charging. Native failures are logged.
#[cfg(windows)]
pub fn battery(id: &ControllerId) -> Option<Battery> {
    let &ControllerId::XInput(index) = id else {
        log::warn!("overlay controller: not an XInput controller: {id:?}");
        return None;
    };
    match heronite_platform::gamepad::battery(index) {
        Ok(percent) => percent.map(|percent| Battery { percent, charging: false }),
        Err(error) => {
            log::warn!("overlay controller: XInputGetBatteryInformation slot {}: {error}", index + 1);
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
