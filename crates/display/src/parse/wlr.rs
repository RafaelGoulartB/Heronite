//! wlroots compositors (Sway, river, labwc, niri…): `wlr-randr --json`.
//! No primary monitor and no HDR in wlr-output-management.

use serde::Deserialize;

use crate::model::{real_serial, tidy_modes, Feature, Mode, Monitor, MonitorId, Refresh};

#[derive(Deserialize)]
struct Head {
    name: String,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    modes: Vec<WMode>,
    /// Only for enabled heads.
    position: Option<Pos>,
    /// `null` before protocol version 4, and for disabled heads.
    adaptive_sync: Option<bool>,
}

#[derive(Deserialize)]
struct WMode {
    width: u32,
    height: u32,
    /// Hz.
    refresh: f64,
    #[serde(default)]
    preferred: bool,
    #[serde(default)]
    current: bool,
}

#[derive(Deserialize)]
struct Pos {
    x: i32,
    y: i32,
}

impl WMode {
    fn mode(&self) -> Mode {
        Mode { width: self.width, height: self.height, refresh: Refresh::from_hz(self.refresh) }
    }
}

/// Whether a connector name is a laptop panel (`eDP-1`, `LVDS-1`, `DSI-1`).
pub(crate) fn builtin_connector(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p))
}

/// Whether a connector is a compositor's software output.
pub(crate) fn virtual_connector(name: &str) -> bool {
    ["HEADLESS", "VIRTUAL", "WL-", "X11-"].iter().any(|p| name.starts_with(p))
}

/// The heads in `json`.
pub fn parse(json: &str) -> Result<Vec<Monitor>, serde_json::Error> {
    let heads: Vec<Head> = serde_json::from_str(json)?;
    Ok(heads.into_iter().map(monitor).collect())
}

fn monitor(h: Head) -> Monitor {
    let text = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (make, model) = (text(&h.make), text(&h.model));
    let name = match (&make, &model) {
        (Some(make), Some(model)) => format!("{make} {model}"),
        _ => text(&h.description).unwrap_or_else(|| h.name.clone()),
    };
    let id = MonitorId {
        vendor: make.unwrap_or_default(),
        product: model.unwrap_or_default(),
        serial: h.serial.as_deref().and_then(real_serial),
        connector: h.name.clone(),
    };
    let current = h.modes.iter().find(|m| m.current).map(WMode::mode);
    let preferred = h.modes.iter().find(|m| m.preferred).map(WMode::mode);
    let mut modes: Vec<Mode> = h.modes.iter().map(WMode::mode).collect();
    tidy_modes(&mut modes);
    Monitor {
        key: id.key(),
        id,
        name,
        enabled: h.enabled,
        primary: false,
        priority: None,
        position: h.position.map_or((0, 0), |p| (p.x, p.y)),
        current: current.filter(|_| h.enabled),
        preferred,
        modes,
        hdr: None,
        vrr: h.adaptive_sync.map(|on| Feature { supported: true, enabled: on }),
        builtin: builtin_connector(&h.name),
        virtual_display: virtual_connector(&h.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, PrimarySupport, Snapshot};

    const FIXTURE: &str = include_str!("../../fixtures/wlr-randr.json");

    #[test]
    fn reads_heads_and_their_modes() {
        let snap = Snapshot::new(Backend::Wlroots, parse(FIXTURE).unwrap());
        let keys: Vec<&str> = snap.monitors.iter().map(|m| m.key.as_str()).collect();
        // The LG has no serial: its connector tells twins apart.
        assert_eq!(keys, ["Dell Inc.:DELL U2723QE:5CD2140XYZ", "LG Electronics:LG ULTRAGEAR@DP-2", "BOE:0x0BCA@eDP-1"]);
        let dell = &snap.monitors[0];
        assert_eq!(dell.name, "Dell Inc. DELL U2723QE");
        assert_eq!(dell.current, Some(Mode { width: 3840, height: 2160, refresh: Refresh { num: 59_997, den: 1000 } }));
        assert_eq!(dell.vrr, Some(Feature { supported: true, enabled: false }));
        assert!(!dell.primary && dell.hdr.is_none());
        let lg = &snap.monitors[1];
        assert_eq!(lg.position, (3840, 0));
        assert_eq!(lg.vrr, Some(Feature { supported: true, enabled: true }));
        assert_eq!(lg.modes.len(), 2);
        // Disabled: no position, no current mode, no VRR state.
        let panel = &snap.monitors[2];
        assert!(panel.builtin && !panel.enabled);
        assert_eq!((panel.current, panel.vrr), (None, None));
        assert_eq!(panel.preferred.map(|m| m.width), Some(2256));
    }

    #[test]
    fn wlroots_has_no_primary_and_no_hdr() {
        let snap = Snapshot::new(Backend::Wlroots, parse(FIXTURE).unwrap());
        assert_eq!(snap.capabilities.primary, PrimarySupport::None);
        assert!(!snap.capabilities.hdr && snap.capabilities.vrr);
    }

    #[test]
    fn names_connectors() {
        assert!(builtin_connector("eDP-1") && builtin_connector("LVDS-1") && !builtin_connector("DP-1"));
        assert!(virtual_connector("HEADLESS-1") && !virtual_connector("HDMI-A-1"));
    }
}
