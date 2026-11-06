//! Documented XInput 1.4 queries. XInput does not expose Guide, the controller's
//! transport, exclusive input, or a Bluetooth disconnect operation.

use std::io;

use windows_sys::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS};
use windows_sys::Win32::UI::Input::XboxController::{
    XInputGetBatteryInformation, XInputGetState, BATTERY_DEVTYPE_GAMEPAD, BATTERY_LEVEL_EMPTY, BATTERY_LEVEL_FULL, BATTERY_LEVEL_LOW,
    BATTERY_LEVEL_MEDIUM, BATTERY_TYPE_ALKALINE, BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_NIMH, BATTERY_TYPE_UNKNOWN, BATTERY_TYPE_WIRED,
    XINPUT_BATTERY_INFORMATION, XINPUT_STATE, XUSER_MAX_COUNT,
};

pub const CONTROLLER_COUNT: u32 = XUSER_MAX_COUNT;

/// A snapshot of the documented controls; stick axes are signed and positive Y
/// points up. Trigger values span 0..=255.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_x: i16,
    pub left_y: i16,
}

/// `None` means the slot is disconnected, not that a native call failed.
pub fn state(index: u32) -> io::Result<Option<State>> {
    validate_index(index)?;
    let mut state = XINPUT_STATE::default();
    // SAFETY: the slot is valid and `state` is writable for the duration of the call.
    let status = unsafe { XInputGetState(index, &mut state) };
    match status {
        ERROR_SUCCESS => Ok(Some(from_native(&state))),
        ERROR_DEVICE_NOT_CONNECTED => Ok(None),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

/// Coarse wireless charge (0, 33, 67, 100 percent). Wired, unknown and
/// disconnected controllers have no battery percentage. Charging is not reported.
pub fn battery(index: u32) -> io::Result<Option<u8>> {
    validate_index(index)?;
    let mut battery = XINPUT_BATTERY_INFORMATION::default();
    // SAFETY: the slot is valid and `battery` is a writable native structure.
    let status = unsafe { XInputGetBatteryInformation(index, BATTERY_DEVTYPE_GAMEPAD, &mut battery) };
    match status {
        ERROR_SUCCESS => battery_percent(battery.BatteryType, battery.BatteryLevel),
        ERROR_DEVICE_NOT_CONNECTED => Ok(None),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

fn validate_index(index: u32) -> io::Result<()> {
    if index < CONTROLLER_COUNT {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "XInput controller index must be between 0 and 3"))
    }
}

fn from_native(state: &XINPUT_STATE) -> State {
    let pad = &state.Gamepad;
    State {
        buttons: pad.wButtons,
        left_trigger: pad.bLeftTrigger,
        right_trigger: pad.bRightTrigger,
        left_x: pad.sThumbLX,
        left_y: pad.sThumbLY,
    }
}

fn battery_percent(kind: u8, level: u8) -> io::Result<Option<u8>> {
    match kind {
        BATTERY_TYPE_DISCONNECTED | BATTERY_TYPE_WIRED | BATTERY_TYPE_UNKNOWN => Ok(None),
        BATTERY_TYPE_ALKALINE | BATTERY_TYPE_NIMH => match level {
            BATTERY_LEVEL_EMPTY => Ok(Some(0)),
            BATTERY_LEVEL_LOW => Ok(Some(33)),
            BATTERY_LEVEL_MEDIUM => Ok(Some(67)),
            BATTERY_LEVEL_FULL => Ok(Some(100)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown XInput battery level {level}"))),
        },
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown XInput battery type {kind}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::XboxController::{XINPUT_GAMEPAD, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_START};

    #[test]
    fn native_controls_are_preserved() {
        let native = XINPUT_STATE {
            dwPacketNumber: 42,
            Gamepad: XINPUT_GAMEPAD {
                wButtons: XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_BACK | XINPUT_GAMEPAD_START,
                bLeftTrigger: 31,
                bRightTrigger: 255,
                sThumbLX: i16::MIN,
                sThumbLY: i16::MAX,
                sThumbRX: 123,
                sThumbRY: -456,
            },
        };
        assert_eq!(
            from_native(&native),
            State { buttons: 0x1030, left_trigger: 31, right_trigger: 255, left_x: i16::MIN, left_y: i16::MAX }
        );
    }

    #[test]
    fn battery_is_coarse_and_unknown_is_not_a_charge() {
        for kind in [BATTERY_TYPE_ALKALINE, BATTERY_TYPE_NIMH] {
            for (level, percent) in
                [(BATTERY_LEVEL_EMPTY, 0), (BATTERY_LEVEL_LOW, 33), (BATTERY_LEVEL_MEDIUM, 67), (BATTERY_LEVEL_FULL, 100)]
            {
                assert_eq!(battery_percent(kind, level).unwrap(), Some(percent));
            }
        }
        for kind in [BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_WIRED, BATTERY_TYPE_UNKNOWN] {
            assert_eq!(battery_percent(kind, BATTERY_LEVEL_FULL).unwrap(), None);
        }
        assert_eq!(battery_percent(42, 0).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(battery_percent(BATTERY_TYPE_NIMH, 42).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn invalid_slots_fail_before_native_calls() {
        assert_eq!(state(CONTROLLER_COUNT).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(battery(u32::MAX).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
