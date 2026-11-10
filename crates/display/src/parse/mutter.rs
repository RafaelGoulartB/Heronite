//! GNOME: Mutter's `org.gnome.Mutter.DisplayConfig.GetCurrentState`, after
//! the D-Bus layer (`crate::linux`) has unpacked it into these plain types.
//!
//! `GetCurrentState` returns
//! `(u serial, a((ssss)a(siiddada{sv})a{sv}) monitors, a(iiduba(ssss)a{sv}) logical_monitors, a{sv} properties)`.
//! A monitor is enabled when a logical monitor holds it; the logical
//! monitor's `primary` flag makes it the primary one.

use crate::model::{real_serial, tidy_modes, Feature, Mode, Monitor, MonitorId, Refresh};
use crate::parse::wlr::virtual_connector;

/// `color-mode` 1: BT.2100, Mutter's HDR.
pub const COLOR_MODE_BT2100: u32 = 1;

/// One entry of `monitors`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MutterMonitor {
    pub connector: String,
    pub vendor: String,
    pub product: String,
    pub serial: String,
    pub modes: Vec<MutterMode>,
    /// `display-name`.
    pub display_name: Option<String>,
    /// `is-builtin`.
    pub builtin: bool,
    /// `color-mode`.
    pub color_mode: Option<u32>,
    /// `supported-color-modes`.
    pub supported_color_modes: Vec<u32>,
}

/// One mode: `(s id, i width, i height, d refresh, d preferred scale, ad scales, a{sv})`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MutterMode {
    pub width: i32,
    pub height: i32,
    pub refresh: f64,
    /// `is-current`.
    pub current: bool,
    /// `is-preferred`.
    pub preferred: bool,
    /// `refresh-rate-mode` is `"variable"`: a VRR mode.
    pub variable: bool,
}

/// One entry of `logical_monitors`, reduced to what places monitors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MutterLogical {
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    /// The connectors of the monitors it shows.
    pub connectors: Vec<String>,
}

fn mode(m: &MutterMode) -> Mode {
    Mode { width: m.width.max(0) as u32, height: m.height.max(0) as u32, refresh: Refresh::from_hz(m.refresh) }
}

/// The monitors Mutter reported.
pub fn monitors(monitors: Vec<MutterMonitor>, logical: &[MutterLogical]) -> Vec<Monitor> {
    monitors
        .into_iter()
        .map(|m| {
            let place = logical.iter().find(|l| l.connectors.contains(&m.connector));
            let enabled = place.is_some();
            let current = m.modes.iter().find(|x| x.current).map(mode).filter(|_| enabled);
            let preferred = m.modes.iter().find(|x| x.preferred).map(mode);
            let any_variable = m.modes.iter().any(|x| x.variable);
            let current_variable = m.modes.iter().any(|x| x.current && x.variable);
            let mut modes: Vec<Mode> = m.modes.iter().map(mode).collect();
            tidy_modes(&mut modes);
            let id = MonitorId {
                vendor: m.vendor.trim().to_string(),
                product: m.product.trim().to_string(),
                serial: real_serial(&m.serial),
                connector: m.connector.clone(),
            };
            let name = m.display_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| m.connector.clone());
            let hdr = (!m.supported_color_modes.is_empty() || m.color_mode.is_some()).then(|| Feature {
                supported: m.supported_color_modes.contains(&COLOR_MODE_BT2100),
                enabled: m.color_mode == Some(COLOR_MODE_BT2100),
            });
            Monitor {
                key: id.key(),
                virtual_display: virtual_connector(&m.connector),
                id,
                name,
                enabled,
                primary: place.is_some_and(|l| l.primary),
                priority: None,
                position: place.map_or((0, 0), |l| (l.x, l.y)),
                current,
                preferred,
                modes,
                hdr,
                vrr: any_variable.then_some(Feature { supported: true, enabled: current_variable }),
                builtin: m.builtin,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, PrimarySupport, Snapshot};

    fn m(width: i32, height: i32, refresh: f64, current: bool, preferred: bool, variable: bool) -> MutterMode {
        MutterMode { width, height, refresh, current, preferred, variable }
    }

    /// A desk monitor with HDR and a VRR mode, and a laptop panel switched off.
    fn fixture() -> (Vec<MutterMonitor>, Vec<MutterLogical>) {
        let desk = MutterMonitor {
            connector: "DP-3".into(),
            vendor: "GSM".into(),
            product: "LG ULTRAGEAR".into(),
            serial: "0x0006d3c1".into(),
            modes: vec![
                m(2560, 1440, 143.998, true, true, false),
                m(2560, 1440, 143.998, false, false, true),
                m(1920, 1080, 60.0, false, false, false),
            ],
            display_name: Some("LG Electronics 27\"".into()),
            builtin: false,
            color_mode: Some(0),
            supported_color_modes: vec![0, 1],
        };
        let panel = MutterMonitor {
            connector: "eDP-1".into(),
            vendor: "BOE".into(),
            product: "0x0bca".into(),
            serial: "0x00000000".into(),
            modes: vec![m(2256, 1504, 59.999, true, true, false)],
            display_name: Some("Built-in display".into()),
            builtin: true,
            ..Default::default()
        };
        let logical = vec![MutterLogical { x: 0, y: 0, primary: true, connectors: vec!["DP-3".into()] }];
        (vec![desk, panel], logical)
    }

    #[test]
    fn reads_monitors_placement_and_primary() {
        let (mons, logical) = fixture();
        let snap = Snapshot::new(Backend::Gnome, monitors(mons, &logical));
        let desk = &snap.monitors[0];
        assert_eq!(desk.key, "GSM:LG ULTRAGEAR:0x0006d3c1");
        assert_eq!(desk.name, "LG Electronics 27\"");
        assert!(desk.enabled && desk.primary);
        assert_eq!(desk.current, Some(Mode { width: 2560, height: 1440, refresh: Refresh { num: 143_998, den: 1000 } }));
        // The fixed and variable 144 Hz modes are one entry in the list.
        assert_eq!(desk.modes.len(), 2);
        assert_eq!(desk.hdr, Some(Feature { supported: true, enabled: false }));
        assert_eq!(desk.vrr, Some(Feature { supported: true, enabled: false }));

        let panel = &snap.monitors[1];
        assert_eq!(panel.key, "BOE:0x0bca@eDP-1");
        assert!(panel.builtin && !panel.enabled && !panel.primary);
        assert_eq!(panel.current, None);
        assert_eq!((panel.hdr, panel.vrr), (None, None));
    }

    #[test]
    fn gnome_has_a_primary_monitor_and_switches_hdr() {
        let (mons, logical) = fixture();
        let snap = Snapshot::new(Backend::Gnome, monitors(mons, &logical));
        assert_eq!(snap.capabilities.primary, PrimarySupport::Primary);
        assert!(snap.capabilities.hdr && snap.capabilities.vrr);
    }

    #[test]
    fn hdr_on_is_color_mode_bt2100() {
        let (mut mons, logical) = fixture();
        mons[0].color_mode = Some(COLOR_MODE_BT2100);
        mons[0].modes[0].current = false;
        mons[0].modes[1].current = true;
        let desk = &monitors(mons, &logical)[0];
        assert_eq!(desk.hdr, Some(Feature { supported: true, enabled: true }));
        assert_eq!(desk.vrr, Some(Feature { supported: true, enabled: true }));
    }
}
