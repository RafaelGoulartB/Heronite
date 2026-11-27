//! Xbox controllers through Windows.Gaming.Input, which reads their battery
//! in mWh: a percentage and whether it charges, where XInput only has four
//! levels (BatteryHub's `xinput.cpp`).
//!
//! Windows.Gaming.Input also answers for controllers that have no battery
//! report: a pad seen as a wired Xbox 360 one (XUSB) gets a made-up 1000 of
//! 1000 mWh, "discharging". So it is only asked about a controller XInput
//! says runs on batteries ([`xbox`]'s caller checks that first).

use std::io;
use std::time::Duration;

use windows::Gaming::Input::RawGameController;
use windows::System::Power::BatteryStatus;

use super::Reading;

/// The list fills in a moment after Windows.Gaming.Input is first used in
/// the process.
const FIRST_LIST: Duration = Duration::from_millis(300);

/// The makers whose controllers have a real battery report there (BatteryHub's
/// list): Microsoft, and GameSir in Xbox (GIP) mode.
const VENDORS: [u16; 2] = [0x045E, 0x3537];

/// The battery of the controller `vendor:product` by Windows.Gaming.Input:
/// `None` when it lists none such, or two (which one is which is not known
/// then), or has no report for it, or for another maker than [`VENDORS`].
/// Call it only for a controller XInput says runs on batteries (see the
/// module). Blocking.
pub fn xbox(vendor: u16, product: u16) -> io::Result<Option<Reading>> {
    if !VENDORS.contains(&vendor) {
        return Ok(None);
    }
    crate::com::keep_mta();
    let mut list = RawGameController::RawGameControllers().map_err(io::Error::other)?;
    if list.Size().map_err(io::Error::other)? == 0 {
        std::thread::sleep(FIRST_LIST);
        list = RawGameController::RawGameControllers().map_err(io::Error::other)?;
    }
    let mut found =
        (&list).into_iter().filter(|pad| pad.HardwareVendorId().ok() == Some(vendor) && pad.HardwareProductId().ok() == Some(product));
    let (Some(pad), None) = (found.next(), found.next()) else { return Ok(None) };
    let Ok(report) = pad.TryGetBatteryReport() else { return Ok(None) };
    let remaining = report.RemainingCapacityInMilliwattHours().and_then(|r| r.Value()).ok();
    let full = report.FullChargeCapacityInMilliwattHours().and_then(|r| r.Value()).ok();
    let status = report.Status().ok();
    Ok(reading(remaining, full, status))
}

/// A battery report as a reading: `None` without capacities, or when the
/// controller has no battery.
fn reading(remaining: Option<i32>, full: Option<i32>, status: Option<BatteryStatus>) -> Option<Reading> {
    let (remaining, full) = (remaining?, full?);
    if full <= 0 || status == Some(BatteryStatus::NotPresent) {
        return None;
    }
    let percent = (i64::from(remaining.max(0)) * 100 / i64::from(full)).clamp(0, 100) as u8;
    Some(Reading { percent, charging: status == Some(BatteryStatus::Charging) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_make_a_percentage() {
        assert_eq!(reading(Some(1500), Some(2000), Some(BatteryStatus::Discharging)), Some(Reading { percent: 75, charging: false }));
        assert_eq!(reading(Some(2100), Some(2000), Some(BatteryStatus::Charging)), Some(Reading { percent: 100, charging: true }));
        assert_eq!(reading(Some(-5), Some(2000), None), Some(Reading { percent: 0, charging: false }));
    }

    #[test]
    fn other_makers_are_not_asked() {
        assert_eq!(xbox(0x2DC8, 0x6012).unwrap(), None);
    }

    #[test]
    fn no_capacity_or_no_battery_is_no_reading() {
        assert_eq!(reading(None, Some(2000), None), None);
        assert_eq!(reading(Some(1000), Some(0), None), None);
        assert_eq!(reading(Some(1000), Some(1000), Some(BatteryStatus::NotPresent)), None);
    }
}
