//! The Windows calls behind [`super::RawTarget`]. Every one of them only
//! reads: `QueryDisplayConfig`, `DisplayConfigGetDeviceInfo` with `GET_*`
//! requests, `EnumDisplaySettingsExW` and a registry read.

use std::mem::size_of;

use heronite_platform::registry::{self, Hive};
use windows_sys::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DISPLAYCONFIG_TARGET_PREFERRED_MODE, QDC_ONLY_ACTIVE_PATHS,
};
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplaySettingsExW, DEVMODEW, DISPLAYCONFIG_PATH_MODE_IDX_INVALID};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};

use super::{device_instance, hdr_from_info, hdr_from_info2, RawTarget};
use crate::edid;
use crate::model::{Feature, Mode, Refresh, Unsupported};

/// `DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2` (Windows 11 24H2,
/// `wingdi.h` 10.0.26100); not in windows-sys 0.61.
const GET_ADVANCED_COLOR_INFO_2: i32 = 15;

/// `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2`. Only `value` is read; the rest
/// is there for the size Windows expects.
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy)]
struct AdvancedColorInfo2 {
    header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    value: u32,
    color_encoding: i32,
    bits_per_color_channel: u32,
    active_color_mode: i32,
}

/// Whether this process runs in a Remote Desktop session, where the
/// monitors are the remote client's.
pub(crate) fn remote_session() -> bool {
    // SAFETY: no arguments beyond the index.
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// The active paths, read.
pub(crate) fn targets() -> Result<Vec<RawTarget>, Unsupported> {
    let (paths, modes) = query()?;
    Ok(paths.iter().map(|p| target(p, &modes)).collect())
}

/// `QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)`, retried while a monitor
/// arrives between sizing and reading.
fn query() -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), Unsupported> {
    for _ in 0..4 {
        let (mut n_paths, mut n_modes) = (0u32, 0u32);
        // SAFETY: the counts are written to locals.
        let r = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) };
        if r != ERROR_SUCCESS {
            return Err(Unsupported::Failed(format!("GetDisplayConfigBufferSizes: error {r}")));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
        // SAFETY: the arrays hold the counts passed with them.
        let r = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut n_paths,
                paths.as_mut_ptr(),
                &mut n_modes,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        match r {
            ERROR_SUCCESS => {
                paths.truncate(n_paths as usize);
                modes.truncate(n_modes as usize);
                return Ok((paths, modes));
            }
            ERROR_INSUFFICIENT_BUFFER => continue,
            r => return Err(Unsupported::Failed(format!("QueryDisplayConfig: error {r}"))),
        }
    }
    Err(Unsupported::Failed("QueryDisplayConfig: the displays kept changing".into()))
}

fn header(kind: i32, size: usize, adapter: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: kind, size: size as u32, adapterId: adapter, id }
}

/// `DisplayConfigGetDeviceInfo` into a `T` that starts with its header.
///
/// # Safety
/// `T` must be the `#[repr(C)]` packet for `kind`, starting with a
/// `DISPLAYCONFIG_DEVICE_INFO_HEADER`, and valid when zeroed.
unsafe fn device_info<T: Copy>(kind: i32, adapter: LUID, id: u32) -> Option<T> {
    // SAFETY: the caller vouches that zero is a valid `T`.
    let mut packet: T = unsafe { std::mem::zeroed() };
    let head = &mut packet as *mut T as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER;
    // SAFETY: `T` starts with the header; the size tells the API how much it may write.
    unsafe {
        *head = header(kind, size_of::<T>(), adapter, id);
        (DisplayConfigGetDeviceInfo(head) == 0).then_some(packet)
    }
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn target(path: &DISPLAYCONFIG_PATH_INFO, modes: &[DISPLAYCONFIG_MODE_INFO]) -> RawTarget {
    let (t_adapter, t_id) = (path.targetInfo.adapterId, path.targetInfo.id);
    let (s_adapter, s_id) = (path.sourceInfo.adapterId, path.sourceInfo.id);
    let mut out = RawTarget {
        output_technology: path.targetInfo.outputTechnology,
        refresh: Refresh { num: path.targetInfo.refreshRate.Numerator, den: path.targetInfo.refreshRate.Denominator },
        ..Default::default()
    };

    // SAFETY: without QDC_VIRTUAL_MODE_AWARE the union holds `modeInfoIdx`.
    let source_idx = unsafe { path.sourceInfo.Anonymous.modeInfoIdx };
    if source_idx != DISPLAYCONFIG_PATH_MODE_IDX_INVALID {
        if let Some(m) = modes.get(source_idx as usize).filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) {
            // SAFETY: `infoType` says the union holds a source mode.
            let s = unsafe { m.Anonymous.sourceMode };
            out.width = s.width;
            out.height = s.height;
            out.position = (s.position.x, s.position.y);
        }
    }

    // SAFETY: each packet type matches its request and is plain data.
    unsafe {
        if let Some(name) = device_info::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, t_adapter, t_id) {
            out.friendly_name = wide_to_string(&name.monitorFriendlyDeviceName);
            out.device_path = wide_to_string(&name.monitorDevicePath);
            // edidIdsValid is bit 1 of the flags.
            if name.flags.Anonymous.value & 0b10 != 0 {
                out.edid_ids = Some((name.edidManufactureId, name.edidProductCodeId));
            }
        }
        if let Some(p) =
            device_info::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>(DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE, t_adapter, t_id)
        {
            let v = p.targetMode.targetVideoSignalInfo.vSyncFreq;
            out.preferred = Some(Mode { width: p.width, height: p.height, refresh: Refresh { num: v.Numerator, den: v.Denominator } });
        }
        out.hdr = hdr(t_adapter, t_id);
        if let Some(source) = device_info::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, s_adapter, s_id) {
            out.modes = enum_modes(&source.viewGdiDeviceName);
        }
    }

    // The serial is only in the EDID the driver keeps under the device's key.
    if let Some((key, _)) = device_instance(&out.device_path) {
        let key = format!(r"SYSTEM\CurrentControlSet\Enum\{key}\Device Parameters");
        out.edid = registry::binary(Hive::LocalMachine, &key, "EDID").and_then(|raw| edid::parse(&raw));
    }
    out
}

/// HDR as the 24H2 query reports it, else as the older one does.
///
/// # Safety
/// Plain `DisplayConfigGetDeviceInfo` reads.
unsafe fn hdr(adapter: LUID, id: u32) -> Option<Feature> {
    // SAFETY: both packets are `#[repr(C)]`, start with the header and are valid zeroed.
    unsafe {
        // Older Windows rejects the request type; then the old one answers.
        if let Some(info) = device_info::<AdvancedColorInfo2>(GET_ADVANCED_COLOR_INFO_2, adapter, id) {
            return Some(hdr_from_info2(info.value));
        }
        device_info::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, adapter, id)
            .map(|info| hdr_from_info(info.Anonymous.value))
    }
}

/// The modes the driver offers for the GDI device `gdi_name` (32-bit colour
/// only; the rest are leftovers from 8 and 16-bit desktops).
fn enum_modes(gdi_name: &[u16; 32]) -> Vec<Mode> {
    let mut out = Vec::new();
    for i in 0.. {
        let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        // SAFETY: `gdi_name` is NUL-terminated (32 characters at most, as the API writes it); `dm` has its size set.
        if unsafe { EnumDisplaySettingsExW(gdi_name.as_ptr(), i, &mut dm, 0) } == 0 {
            break;
        }
        if dm.dmBitsPerPel == 32 && dm.dmPelsWidth > 0 && dm.dmDisplayFrequency > 1 {
            out.push(Mode { width: dm.dmPelsWidth, height: dm.dmPelsHeight, refresh: Refresh { num: dm.dmDisplayFrequency, den: 1 } });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// Reads this machine's displays (read-only) and prints them, past the
    /// Remote Desktop check: `cargo test -p heronite-display -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real displays; run by hand"]
    fn reads_this_machines_displays() {
        let targets = super::targets().expect("QueryDisplayConfig");
        for t in &targets {
            println!("{t:?}");
        }
        let snap = crate::Snapshot::new(crate::Backend::Windows, super::super::monitors(targets));
        println!("{}", serde_json::to_string_pretty(&snap).unwrap());
    }
}
