//! The battery of a game controller no driver knows, by the battery usages
//! of the HID standard (BatteryHub's `generic.cpp`, `PollGenericHid`):
//! Generic Device Controls "Battery Strength" (`06:20`) or Battery System
//! "Absolute State Of Charge" (`85:65`), in a feature or an input report,
//! scaled from the logical range the descriptor declares. The charging state
//! comes from the same report, when it has Battery System "Charging" (`44`),
//! "Discharging" (`45`), "Fully Charged" (`46`) or "AC Present" (`D0`), as
//! buttons or values.
//!
//! Only controllers (a Generic Desktop joystick or game pad collection, and
//! no mouse one, as BatteryHub tells a controller) are read, and only through
//! reports whose descriptor declares a battery usage: drawing tablets and
//! power supplies never (reading a tablet flipped its Y axis, BatteryHub
//! issue #232). Reports are read with HID GET_REPORT requests, nothing is
//! written.
//!
//! BatteryHub's other fallback, the level Windows keeps for Bluetooth
//! devices, is not ported: that device property needs `windows-sys`'s
//! `Win32_Devices_Properties`, and its devices go by Bluetooth container,
//! not by vendor and product.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetFeature, HidD_GetInputReport, HidD_GetPreparsedData, HidP_Feature, HidP_GetCaps, HidP_GetUsageValue,
    HidP_GetUsages, HidP_GetValueCaps, HidP_Input, HidP_MaxUsageListLength, HIDP_CAPS, HIDP_REPORT_TYPE, HIDP_STATUS_SUCCESS,
    HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};

use super::Reading;
use crate::hid::Collection;

/// The Battery System usage page, with the charging usages.
const BATTERY_SYSTEM: u16 = 0x85;

pub(super) fn read(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>> {
    let mine: Vec<&Collection> = collections.iter().filter(|c| c.vendor == vendor && c.product == product).collect();
    if !is_controller(&mine) {
        return Ok(None);
    }
    // UPS and power supply collections are not the controller's battery.
    for c in mine.into_iter().filter(|c| !matches!(c.usage_page, 0x84 | BATTERY_SYSTEM)) {
        match read_collection(c) {
            Ok(Some(reading)) => return Ok(Some(reading)),
            Ok(None) => {}
            // Gone since, or refused: another collection may still tell it.
            Err(e) => log::debug!("HID battery of {}: {e}", c.path()),
        }
    }
    Ok(None)
}

/// Whether the collections of a device make a game controller, as
/// BatteryHub tells one: a Generic Desktop joystick (`01:04`) or game pad
/// (`01:05`) collection, no mouse (`01:02`) one, and no digitizer (page `0D`).
fn is_controller(collections: &[&Collection]) -> bool {
    let has =
        |page: u16, usages: &[u16]| collections.iter().any(|c| c.usage_page == page && (usages.is_empty() || usages.contains(&c.usage)));
    has(0x01, &[0x04, 0x05]) && !has(0x01, &[0x02]) && !has(0x0D, &[])
}

/// Whether `page:usage` is a battery level.
fn is_battery_usage(page: u16, usage: u16) -> bool {
    matches!((page, usage), (0x06, 0x20) | (BATTERY_SYSTEM, 0x65))
}

/// A battery value in `min..=max`, as a percentage; without a range, a
/// value up to 100 is taken as one.
fn percent(value: u32, min: i32, max: i32) -> Option<u8> {
    if max > min {
        // As BatteryHub (and the field's bits) read it: signed.
        let value = i64::from(value as i32);
        let (min, max) = (i64::from(min), i64::from(max));
        Some(((value - min) * 100 / (max - min)).clamp(0, 100) as u8)
    } else {
        u8::try_from(value).ok().filter(|&p| p <= 100)
    }
}

/// The Battery System charging usages set in a report.
#[derive(Debug, Default)]
struct Charge {
    charging: bool,
    discharging: bool,
    full: bool,
    ac: bool,
}

impl Charge {
    fn note(&mut self, usage: u16) {
        match usage {
            0x44 => self.charging = true,
            0x45 => self.discharging = true,
            0x46 => self.full = true,
            0xD0 => self.ac = true,
            _ => {}
        }
    }

    /// Charging, full on its charger, or on AC power without discharging.
    fn is_charging(&self) -> bool {
        self.charging || self.full || (self.ac && !self.discharging)
    }
}

/// The preparsed data of an opened collection, freed on drop.
struct Preparsed(PHIDP_PREPARSED_DATA);

impl Drop for Preparsed {
    fn drop(&mut self) {
        // SAFETY: data HidD_GetPreparsedData returned, freed once.
        unsafe { HidD_FreePreparsedData(self.0) };
    }
}

/// Opens `collection` without read or write access (enough for its
/// descriptor and GET_REPORT requests, and never refused because another
/// program holds it).
fn open(collection: &Collection) -> io::Result<OwnedHandle> {
    let path: Vec<u16> = collection.path().encode_utf16().chain([0]).collect();
    // SAFETY: `path` is NUL-terminated; the handle is owned below.
    let raw = unsafe {
        CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a handle CreateFileW just opened, owned once.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// The battery of `collection`, from the first report its descriptor puts a
/// battery level in, feature reports first.
fn read_collection(collection: &Collection) -> io::Result<Option<Reading>> {
    let handle = open(collection)?;
    let raw = handle.as_raw_handle() as HANDLE;
    let mut data = 0;
    // SAFETY: the handle is open; `data` is freed by `Preparsed`.
    if !unsafe { HidD_GetPreparsedData(raw, &mut data) } {
        return Err(io::Error::last_os_error());
    }
    let preparsed = Preparsed(data);
    let mut caps = HIDP_CAPS::default();
    // SAFETY: the collection's preparsed data, alive while `preparsed` is.
    if unsafe { HidP_GetCaps(preparsed.0, &mut caps) } != HIDP_STATUS_SUCCESS {
        return Ok(None);
    }
    let kinds = [
        (HidP_Feature, caps.NumberFeatureValueCaps, collection.feature_len),
        (HidP_Input, caps.NumberInputValueCaps, collection.input_len),
    ];
    for (kind, count, len) in kinds {
        if count == 0 || len == 0 {
            continue;
        }
        let values = value_caps(kind, count, &preparsed);
        for v in values.iter().filter(|v| is_battery_usage(v.UsagePage, usage_of(v))) {
            let mut report = vec![0u8; len];
            report[0] = v.ReportID;
            let (ptr, size) = (report.as_mut_ptr().cast(), report.len() as u32);
            // SAFETY: the handle is open; `report` is writable for its length.
            let got = unsafe {
                if kind == HidP_Feature {
                    HidD_GetFeature(raw, ptr, size)
                } else {
                    HidD_GetInputReport(raw, ptr, size)
                }
            };
            if !got {
                continue;
            }
            let Some(value) = usage_value(kind, v, usage_of(v), &preparsed, &report) else { continue };
            let Some(percent) = percent(value, v.LogicalMin, v.LogicalMax) else { continue };
            let charging = charge(kind, &values, &preparsed, &mut report).is_charging();
            return Ok(Some(Reading { percent, charging }));
        }
    }
    Ok(None)
}

/// The value capabilities of the `kind` reports (`count` of them).
fn value_caps(kind: HIDP_REPORT_TYPE, count: u16, preparsed: &Preparsed) -> Vec<HIDP_VALUE_CAPS> {
    let mut values = vec![HIDP_VALUE_CAPS::default(); count.into()];
    let mut n = count;
    // SAFETY: `values` holds `n` entries; the preparsed data is alive.
    if unsafe { HidP_GetValueCaps(kind, values.as_mut_ptr(), &mut n, preparsed.0) } != HIDP_STATUS_SUCCESS {
        return Vec::new();
    }
    values.truncate(n.into());
    values
}

/// The usage of a value capability (the first of a range).
fn usage_of(v: &HIDP_VALUE_CAPS) -> u16 {
    // SAFETY: both members of the union start with the usage, and IsRange
    // tells which one Windows filled.
    unsafe {
        if v.IsRange {
            v.Anonymous.Range.UsageMin
        } else {
            v.Anonymous.NotRange.Usage
        }
    }
}

/// The value of `usage` (of `v`'s page and link collection) in `report`.
fn usage_value(kind: HIDP_REPORT_TYPE, v: &HIDP_VALUE_CAPS, usage: u16, preparsed: &Preparsed, report: &[u8]) -> Option<u32> {
    let mut value = 0;
    // SAFETY: `report` is readable for its length; the preparsed data is alive.
    let status = unsafe {
        HidP_GetUsageValue(kind, v.UsagePage, v.LinkCollection, usage, &mut value, preparsed.0, report.as_ptr(), report.len() as u32)
    };
    (status == HIDP_STATUS_SUCCESS).then_some(value)
}

/// The charging usages set in `report`, as buttons or as values (BatteryHub
/// PR #632). A report without them tells "not charging".
fn charge(kind: HIDP_REPORT_TYPE, values: &[HIDP_VALUE_CAPS], preparsed: &Preparsed, report: &mut [u8]) -> Charge {
    let mut charge = Charge::default();
    // SAFETY: the preparsed data is alive.
    let max = unsafe { HidP_MaxUsageListLength(kind, BATTERY_SYSTEM, preparsed.0) };
    if max > 0 {
        let mut on = vec![0u16; max as usize];
        let mut count = max;
        // SAFETY: `on` holds `count` usages; `report` is writable for its
        // length (Windows only reads it).
        let status = unsafe {
            HidP_GetUsages(kind, BATTERY_SYSTEM, 0, on.as_mut_ptr(), &mut count, preparsed.0, report.as_mut_ptr(), report.len() as u32)
        };
        if status == HIDP_STATUS_SUCCESS {
            on.iter().take(count as usize).for_each(|&u| charge.note(u));
        }
    }
    for v in values.iter().filter(|v| v.UsagePage == BATTERY_SYSTEM) {
        let usage = usage_of(v);
        if !matches!(usage, 0x44 | 0x45 | 0x46 | 0xD0) {
            continue;
        }
        if usage_value(kind, v, usage, preparsed, report).is_some_and(|value| value != 0) {
            charge.note(usage);
        }
    }
    charge
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller(collections: &[Collection]) -> bool {
        is_controller(&collections.iter().collect::<Vec<_>>())
    }

    #[test]
    fn only_controllers_are_read() {
        let pad = Collection::new(0x1234, 0x5678, 0x01, 0x05, (15, 0, 0));
        let joystick = Collection::new(0x1234, 0x5678, 0x01, 0x04, (15, 0, 0));
        let vendor = Collection::new(0x1234, 0x5678, 0xFF00, 0x01, (64, 64, 0));
        let mouse = Collection::new(0x1234, 0x5678, 0x01, 0x02, (8, 0, 0));
        let keyboard = Collection::new(0x1234, 0x5678, 0x01, 0x06, (8, 1, 0));
        let pen = Collection::new(0x1234, 0x5678, 0x0D, 0x02, (10, 0, 0));
        assert!(controller(&[pad.clone(), vendor.clone()]));
        assert!(controller(&[joystick]));
        assert!(controller(&[keyboard.clone(), pad.clone()]));
        assert!(!controller(&[vendor]), "no game input");
        assert!(!controller(&[keyboard]));
        assert!(!controller(&[mouse, pad.clone()]), "a mouse, as BatteryHub tells it");
        assert!(!controller(&[pen, pad]), "a tablet is never read");
        assert!(!controller(&[]));
    }

    #[test]
    fn the_battery_usages_are_the_standard_ones() {
        assert!(is_battery_usage(0x06, 0x20), "Battery Strength");
        assert!(is_battery_usage(0x85, 0x65), "Absolute State Of Charge");
        assert!(!is_battery_usage(0x85, 0x66), "Remaining Capacity");
        assert!(!is_battery_usage(0x06, 0x21));
        assert!(!is_battery_usage(0x01, 0x20));
    }

    #[test]
    fn values_scale_from_their_logical_range() {
        assert_eq!(percent(255, 0, 255), Some(100));
        assert_eq!(percent(128, 0, 255), Some(50));
        assert_eq!(percent(0, 0, 255), Some(0));
        assert_eq!(percent(73, 0, 100), Some(73));
        assert_eq!(percent(3, 1, 5), Some(50), "five levels");
        assert_eq!(percent(0, 1, 5), Some(0), "below the range");
        assert_eq!(percent(300, 0, 255), Some(100), "above the range");
        assert_eq!(percent(u32::MAX, 0, i32::MAX), Some(0), "no overflow");
        assert_eq!(percent(5, i32::MIN, i32::MAX), Some(50));
        // Without a range, a percentage or nothing.
        assert_eq!(percent(64, 0, 0), Some(64));
        assert_eq!(percent(100, 0, 0), Some(100));
        assert_eq!(percent(101, 0, 0), None);
        assert_eq!(percent(1000, 5, 5), None);
    }

    fn charging(usages: &[u16]) -> bool {
        let mut charge = Charge::default();
        usages.iter().for_each(|&u| charge.note(u));
        charge.is_charging()
    }

    #[test]
    fn the_charging_usages_combine() {
        assert!(!charging(&[]), "nothing told: not charging");
        assert!(charging(&[0x44]), "charging");
        assert!(!charging(&[0x45]), "discharging");
        assert!(charging(&[0x46]), "fully charged");
        assert!(charging(&[0xD0]), "on AC");
        assert!(!charging(&[0xD0, 0x45]), "on AC yet discharging");
        assert!(charging(&[0xD0, 0x45, 0x44]));
        assert!(!charging(&[0x65, 0x20]), "other usages");
    }
}
