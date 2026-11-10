//! Hyprland: `hyprctl monitors all -j`. No primary monitor. HDR is a colour
//! management preset (`hdr`, `hdredid`); `hardwareDetails` (newer releases)
//! says what the monitor supports.

use serde::Deserialize;

use super::wlr::{builtin_connector, virtual_connector};
use crate::model::{real_serial, tidy_modes, Feature, Mode, Monitor, MonitorId, Refresh};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HMonitor {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    serial: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    refresh_rate: f64,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    vrr: bool,
    #[serde(default)]
    disabled: bool,
    /// `"3840x2160@143.99Hz"`.
    #[serde(default)]
    available_modes: Vec<String>,
    color_management_preset: Option<String>,
    hardware_details: Option<Hardware>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hardware {
    #[serde(default)]
    hdr: bool,
    #[serde(default)]
    vrr_capable: bool,
}

/// `"3840x2160@143.99Hz"` as a mode.
pub(crate) fn parse_mode(s: &str) -> Option<Mode> {
    let (size, rate) = s.trim().split_once('@')?;
    let (w, h) = size.split_once('x')?;
    let hz: f64 = rate.trim_end_matches("Hz").parse().ok()?;
    Some(Mode { width: w.parse().ok()?, height: h.parse().ok()?, refresh: Refresh::from_hz(hz) })
}

/// The monitors in `json`.
pub fn parse(json: &str) -> Result<Vec<Monitor>, serde_json::Error> {
    let monitors: Vec<HMonitor> = serde_json::from_str(json)?;
    Ok(monitors.into_iter().map(monitor).collect())
}

fn monitor(m: HMonitor) -> Monitor {
    let trimmed = |s: &str| s.trim().to_string();
    let id = MonitorId { vendor: trimmed(&m.make), product: trimmed(&m.model), serial: real_serial(&m.serial), connector: m.name.clone() };
    let name = match (id.vendor.is_empty(), id.product.is_empty()) {
        (false, false) => format!("{} {}", id.vendor, id.product),
        _ if !m.description.trim().is_empty() => trimmed(&m.description),
        _ => m.name.clone(),
    };
    let enabled = !m.disabled;
    let current = (enabled && m.width > 0 && m.height > 0).then(|| Mode {
        width: m.width,
        height: m.height,
        refresh: Refresh::from_hz(m.refresh_rate),
    });
    let mut modes: Vec<Mode> = m.available_modes.iter().filter_map(|s| parse_mode(s)).collect();
    tidy_modes(&mut modes);
    let hdr_on = m.color_management_preset.as_deref().is_some_and(|p| p.starts_with("hdr"));
    let (hdr, vrr) = match &m.hardware_details {
        Some(hw) => (Some(Feature { supported: hw.hdr, enabled: hdr_on }), Some(Feature { supported: hw.vrr_capable, enabled: m.vrr })),
        // Older releases: only what is on says anything.
        None => (hdr_on.then_some(Feature { supported: true, enabled: true }), m.vrr.then_some(Feature { supported: true, enabled: true })),
    };
    Monitor {
        key: id.key(),
        builtin: builtin_connector(&m.name),
        virtual_display: virtual_connector(&m.name),
        id,
        name,
        enabled,
        primary: false,
        priority: None,
        position: (m.x, m.y),
        current,
        preferred: None,
        modes,
        hdr,
        vrr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, PrimarySupport, Snapshot};

    const FIXTURE: &str = include_str!("../../fixtures/hyprctl-monitors.json");

    #[test]
    fn reads_monitors_modes_hdr_and_vrr() {
        let snap = Snapshot::new(Backend::Hyprland, parse(FIXTURE).unwrap());
        let keys: Vec<&str> = snap.monitors.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["GIGA-BYTE TECHNOLOGY CO., LTD.:M27U:23110B000123", "Samsung Electric Company:LS32BG85@HDMI-A-1"]);
        let m27u = &snap.monitors[0];
        assert_eq!(m27u.name, "GIGA-BYTE TECHNOLOGY CO., LTD. M27U");
        assert_eq!(m27u.current, Some(Mode { width: 3840, height: 2160, refresh: Refresh { num: 159_975, den: 1000 } }));
        assert_eq!(m27u.modes.first().map(|m| m.refresh.millihertz()), Some(159_980));
        assert_eq!(m27u.hdr, Some(Feature { supported: true, enabled: true }));
        assert_eq!(m27u.vrr, Some(Feature { supported: true, enabled: true }));
        let tv = &snap.monitors[1];
        assert!(!tv.enabled);
        assert_eq!(tv.current, None);
        // An older Hyprland without hardwareDetails, VRR and HDR off: unknown.
        assert_eq!((tv.hdr, tv.vrr), (None, None));
    }

    #[test]
    fn hyprland_has_no_primary_monitor() {
        let snap = Snapshot::new(Backend::Hyprland, parse(FIXTURE).unwrap());
        assert_eq!(snap.capabilities.primary, PrimarySupport::None);
        assert!(snap.capabilities.hdr && snap.capabilities.vrr);
    }

    #[test]
    fn parses_mode_strings() {
        assert_eq!(parse_mode("2560x1440@143.97Hz"), Some(Mode { width: 2560, height: 1440, refresh: Refresh::from_hz(143.97) }));
        assert_eq!(parse_mode("1920x1080@60.00Hz").map(|m| m.refresh.millihertz()), Some(60_000));
        assert_eq!(parse_mode("garbage"), None);
        assert_eq!(parse_mode("1920x@60Hz"), None);
    }
}
