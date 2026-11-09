//! Windows: the active display paths from `QueryDisplayConfig`, named and
//! identified through `DisplayConfigGetDeviceInfo` and the EDID the driver
//! stores, with the modes `EnumDisplaySettingsExW` offers. Only queries: no
//! call here changes a display.
//!
//! The raw facts ([`RawTarget`]) are gathered by [`api`] and turned into
//! [`Monitor`]s here, so that part is tested on every platform.

#[cfg(windows)]
pub(crate) mod api;

use crate::edid::{self, EdidInfo};
use crate::model::{tidy_modes, Feature, Mode, Monitor, MonitorId, Refresh};

/// `DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_VIRTUAL`: a display driver
/// without hardware (Sunshine/Apollo's, Parsec's, Virtual Desktop's).
pub(crate) const TECH_INDIRECT_VIRTUAL: i32 = 17;
/// `DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL`: a laptop panel.
pub(crate) const TECH_INTERNAL: i32 = i32::MIN;
/// `DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED` and `UDI_EMBEDDED`.
const TECH_EMBEDDED: [i32; 2] = [11, 13];

/// One active path, as the API reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RawTarget {
    /// `monitorFriendlyDeviceName` (empty for some virtual displays).
    pub friendly_name: String,
    /// `edidManufactureId`, in Windows' byte order; `None` when the
    /// `edidIdsValid` flag is off.
    pub edid_ids: Option<(u16, u16)>,
    /// `monitorDevicePath`: `\\?\DISPLAY#DELA1D0#5&209d169f&0&UID4353#{e6f07b5f-…}`.
    pub device_path: String,
    /// The EDID block from the registry, when it could be read.
    pub edid: Option<EdidInfo>,
    pub output_technology: i32,
    pub position: (i32, i32),
    pub width: u32,
    pub height: u32,
    pub refresh: Refresh,
    pub preferred: Option<Mode>,
    pub modes: Vec<Mode>,
    pub hdr: Option<Feature>,
}

/// `DISPLAY\DELA1D0\5&209d169f&0&UID4353` from a monitor device path: the
/// device's key under `SYSTEM\CurrentControlSet\Enum`, and its last part,
/// the instance that follows the port.
pub(crate) fn device_instance(path: &str) -> Option<(String, String)> {
    let trimmed = path.strip_prefix(r"\\?\").unwrap_or(path);
    let parts: Vec<&str> = trimmed.split('#').collect();
    match parts.as_slice() {
        [class, hardware, instance, ..] if !hardware.is_empty() && !instance.is_empty() => {
            Some((format!(r"{class}\{hardware}\{instance}"), instance.to_string()))
        }
        _ => None,
    }
}

/// HDR from `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2` (Windows 11 24H2 and
/// later): the `value` bit field.
pub(crate) fn hdr_from_info2(value: u32) -> Feature {
    // highDynamicRangeSupported, highDynamicRangeUserEnabled.
    Feature { supported: value & (1 << 4) != 0, enabled: value & (1 << 5) != 0 }
}

/// HDR from the older `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO`. Before 24H2
/// advanced colour meant HDR; on 24H2 [`hdr_from_info2`] is used instead,
/// because Auto Color Management also turns advanced colour on.
pub(crate) fn hdr_from_info(value: u32) -> Feature {
    let supported = value & 1 != 0;
    let force_disabled = value & (1 << 3) != 0;
    Feature { supported: supported && !force_disabled, enabled: value & (1 << 1) != 0 }
}

/// The whole-hertz rate `EnumDisplaySettings` lists for the exact `rate`.
/// The 1000/1001 rates of TV timings (23.976, 59.94, 119.88, 143.86) are
/// listed one hertz down (`59`), so that they stay apart from the integer
/// rate next to them (`60`); any other rate is rounded (159.975 as `160`).
///
/// Should Windows list some rate otherwise, the current mode only shows up
/// twice; no other mode is dropped.
pub(crate) fn listed_rate(rate: Refresh) -> Refresh {
    let hz = rate.hz();
    // The rate with the 1000/1001 pulldown undone: 59.94 → 60.
    let base = hz * 1.001;
    let tv = (hz - hz.round()).abs() > 0.005 && (base - base.round()).abs() < 0.005;
    let listed = if tv { (base.round() - 1.).max(0.) } else { hz.round() };
    Refresh { num: listed as u32, den: 1 }
}

/// The monitors behind `targets`.
pub(crate) fn monitors(targets: Vec<RawTarget>) -> Vec<Monitor> {
    targets.into_iter().map(monitor).collect()
}

fn monitor(t: RawTarget) -> Monitor {
    let (registry_vendor, registry_product) = match &t.edid {
        Some(e) => (Some(e.vendor.clone()), Some(e.product.clone())),
        None => (None, None),
    };
    // A Remote Desktop or generic display says its ids are valid and reports
    // zeros; a vendor that does not decode to letters is no vendor either.
    let ids = t.edid_ids.filter(|&(v, p)| v != 0 || p != 0);
    let vendor = ids.map(|(v, _)| edid::pnp_vendor_from_windows(v)).or(registry_vendor).filter(|v| !v.contains('?'));
    let product = ids.map(|(_, p)| format!("{p:04X}")).or(registry_product);
    let (vendor, product) = match vendor {
        Some(vendor) => (vendor, product.unwrap_or_default()),
        None => (String::new(), String::new()),
    };
    let connector = device_instance(&t.device_path).map(|(_, instance)| instance).unwrap_or_else(|| t.device_path.clone());
    let id = MonitorId { vendor, product, serial: t.edid.as_ref().and_then(|e| e.serial.clone()), connector };
    let name = [Some(t.friendly_name.trim().to_string()), t.edid.as_ref().and_then(|e| e.name.clone())]
        .into_iter()
        .flatten()
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| id.connector.clone());
    let current = Mode { width: t.width, height: t.height, refresh: t.refresh };
    let mut modes = t.modes;
    // The current mode, with its exact rate, stands in for the whole-hertz
    // one EnumDisplaySettings lists, and only for that one: at 59.94 Hz the
    // list's 60 is another mode.
    let listed = listed_rate(current.refresh);
    modes.retain(|m| !(m.width == current.width && m.height == current.height && m.refresh == listed));
    modes.push(current);
    tidy_modes(&mut modes);
    Monitor {
        key: id.key(),
        id,
        name,
        enabled: true,
        // The primary monitor is the one at the desktop's origin.
        primary: t.position == (0, 0),
        priority: None,
        position: t.position,
        current: Some(current),
        preferred: t.preferred,
        modes,
        hdr: t.hdr,
        vrr: None,
        builtin: t.output_technology == TECH_INTERNAL || TECH_EMBEDDED.contains(&t.output_technology),
        virtual_display: t.output_technology == TECH_INDIRECT_VIRTUAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edid::tests::edid;
    use crate::model::{Backend, PrimarySupport, Snapshot};

    fn mode(w: u32, h: u32, hz: u32) -> Mode {
        Mode { width: w, height: h, refresh: Refresh { num: hz, den: 1 } }
    }

    /// Two desk monitors and a virtual display, as on a streaming PC.
    fn fixture() -> Vec<RawTarget> {
        vec![
            RawTarget {
                friendly_name: "M27U".into(),
                edid_ids: Some((0x541c, 0x273e)),
                device_path: r"\\?\DISPLAY#GBT273E#5&209d169f&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}".into(),
                edid: crate::edid::parse(&edid("GBT", 0x273e, 0, None, Some("M27U"))),
                output_technology: 10, // DisplayPort
                position: (0, 0),
                width: 3840,
                height: 2160,
                refresh: Refresh { num: 159_975, den: 1000 },
                preferred: Some(mode(3840, 2160, 160)),
                modes: vec![mode(3840, 2160, 160), mode(3840, 2160, 60), mode(2560, 1440, 144)],
                hdr: Some(hdr_from_info2(0b0011_0011)),
            },
            RawTarget {
                friendly_name: "DELL U2723QE".into(),
                edid_ids: Some((0xac10, 0xa1d0)),
                device_path: r"\\?\DISPLAY#DELA1D0#5&209d169f&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}".into(),
                edid: crate::edid::parse(&edid("DEL", 0xa1d0, 0, Some("5CD2140XYZ"), Some("DELL U2723QE"))),
                output_technology: 5, // HDMI
                position: (-3840, 0),
                width: 3840,
                height: 2160,
                refresh: Refresh { num: 60000, den: 1001 },
                // Windows lists 59.94 Hz as 59, next to the real 60.
                modes: vec![mode(3840, 2160, 60), mode(3840, 2160, 59), mode(1920, 1080, 60)],
                hdr: Some(hdr_from_info2(0b0000_0001)),
                ..Default::default()
            },
            RawTarget {
                friendly_name: String::new(),
                edid_ids: None,
                device_path: r"\\?\DISPLAY#VDD1234#1&2b3c4d5e&0&UID256#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}".into(),
                output_technology: TECH_INDIRECT_VIRTUAL,
                position: (3840, 0),
                width: 1920,
                height: 1080,
                refresh: Refresh { num: 60, den: 1 },
                ..Default::default()
            },
        ]
    }

    #[test]
    fn identifies_and_names_each_monitor() {
        let snap = Snapshot::new(Backend::Windows, monitors(fixture()));
        let keys: Vec<&str> = snap.monitors.iter().map(|m| m.key.as_str()).collect();
        // Left to right; the Dell has a serial, the Gigabyte only its port.
        assert_eq!(keys, ["DEL:A1D0:5CD2140XYZ", "GBT:273E@5&209d169f&0&UID4352", "@1&2b3c4d5e&0&UID256"]);
        let names: Vec<&str> = snap.monitors.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["DELL U2723QE", "M27U", "1&2b3c4d5e&0&UID256"]);
        assert_eq!(snap.monitors.iter().map(|m| m.primary).collect::<Vec<_>>(), [false, true, false]);
        assert!(snap.monitors[2].virtual_display);
        assert!(!snap.monitors[0].virtual_display && !snap.monitors[0].builtin);
    }

    #[test]
    fn keeps_the_exact_current_rate_and_the_mode_list() {
        let snap = Snapshot::new(Backend::Windows, monitors(fixture()));
        let dell = &snap.monitors[0];
        assert_eq!(dell.current.unwrap().refresh, Refresh { num: 60000, den: 1001 });
        // The listed 59 gave way to the exact current rate; the 60 Hz mode stays.
        assert_eq!(
            dell.modes,
            vec![
                mode(3840, 2160, 60),
                Mode { width: 3840, height: 2160, refresh: Refresh { num: 60000, den: 1001 } },
                mode(1920, 1080, 60)
            ]
        );
        let gigabyte = &snap.monitors[1];
        assert_eq!(gigabyte.modes.len(), 3);
        assert_eq!(gigabyte.modes[0].refresh, Refresh { num: 159_975, den: 1000 });
        assert_eq!(gigabyte.preferred, Some(mode(3840, 2160, 160)));
    }

    #[test]
    fn reads_hdr_and_what_windows_can_change() {
        let snap = Snapshot::new(Backend::Windows, monitors(fixture()));
        assert_eq!(snap.monitors[1].hdr, Some(Feature { supported: true, enabled: true }));
        assert_eq!(snap.monitors[0].hdr, Some(Feature { supported: false, enabled: false }));
        assert_eq!(snap.monitors[2].hdr, None);
        assert_eq!(snap.capabilities.primary, PrimarySupport::Primary);
        assert!(snap.capabilities.hdr && !snap.capabilities.vrr && snap.capabilities.exact_refresh);
    }

    #[test]
    fn exact_rates_map_to_the_whole_hertz_windows_lists() {
        let listed = |num, den| listed_rate(Refresh { num, den }).num;
        assert_eq!(listed(60000, 1001), 59);
        assert_eq!(listed(24000, 1001), 23);
        assert_eq!(listed(120_000, 1001), 119);
        assert_eq!(listed(144_000, 1001), 143);
        assert_eq!(listed(60, 1), 60);
        assert_eq!(listed(59_997, 1000), 60);
        assert_eq!(listed(159_975, 1000), 160);
        assert_eq!(listed(165_080, 1000), 165);
        assert_eq!(listed(0, 0), 0);
    }

    #[test]
    fn at_60_hz_the_listed_59_stays() {
        let mut t = fixture().remove(1);
        t.refresh = Refresh { num: 60, den: 1 };
        let dell = &monitors(vec![t])[0];
        assert_eq!(dell.modes, vec![mode(3840, 2160, 60), mode(3840, 2160, 59), mode(1920, 1080, 60)]);
    }

    #[test]
    fn hdr_bits_old_and_new() {
        // 24H2: an SDR panel with Auto Color Management has advanced colour
        // on (bits 0-1) but HDR neither supported nor on.
        assert_eq!(hdr_from_info2(0b0000_0011), Feature { supported: false, enabled: false });
        assert_eq!(hdr_from_info2(0b0001_0001), Feature { supported: true, enabled: false });
        assert_eq!(hdr_from_info(0b0011), Feature { supported: true, enabled: true });
        // Supported but forced off (by policy, or a driver).
        assert_eq!(hdr_from_info(0b1001), Feature { supported: false, enabled: false });
    }

    #[test]
    fn a_remote_desktop_display_with_zero_ids_is_known_by_its_port() {
        // What Windows reported for an RDP session's display; zero ids are
        // no ids even when flagged valid.
        let rdp = RawTarget {
            edid_ids: Some((0, 0)),
            device_path: r"\\?\DISPLAY#Default_Monitor#1&c528b8a&4&UID256#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}".into(),
            output_technology: -1,
            width: 600,
            height: 1244,
            refresh: Refresh { num: 32, den: 1 },
            ..Default::default()
        };
        let m = &monitors(vec![rdp])[0];
        assert_eq!(m.key, "@1&c528b8a&4&UID256");
        assert_eq!((m.id.vendor.as_str(), m.id.product.as_str()), ("", ""));
    }

    #[test]
    fn splits_device_paths() {
        let (key, instance) = device_instance(r"\\?\DISPLAY#DELA1D0#5&209d169f&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}").unwrap();
        assert_eq!(key, r"DISPLAY\DELA1D0\5&209d169f&0&UID4353");
        assert_eq!(instance, "5&209d169f&0&UID4353");
        assert_eq!(device_instance(""), None);
        assert_eq!(device_instance(r"\\?\DISPLAY#"), None);
    }
}
