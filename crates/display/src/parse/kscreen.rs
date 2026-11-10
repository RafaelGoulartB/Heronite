//! KDE Plasma: `kscreen-doctor -j`, libkscreen's `ConfigSerializer` output.
//!
//! It has no EDID (the serializer leaves it out), so a KDE monitor is known
//! by its connector alone. The main display is the output with priority 1.

use serde::Deserialize;

use crate::model::{tidy_modes, Feature, Mode, Monitor, MonitorId, Refresh};

#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    outputs: Vec<Output>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    name: String,
    /// `KScreen::Output::Type`; 7 is a laptop panel.
    #[serde(default, rename = "type")]
    kind: i32,
    #[serde(default)]
    pos: Pos,
    #[serde(default)]
    current_mode_id: String,
    #[serde(default)]
    preferred_modes: Vec<String>,
    #[serde(default)]
    connected: bool,
    /// Only written when the output can be disabled.
    enabled: Option<bool>,
    #[serde(default)]
    priority: u32,
    #[serde(default)]
    modes: Vec<KMode>,
    /// Only with the VRR capability: 0 never, 1 always, 2 automatic.
    vrr_policy: Option<i32>,
    /// Only with the HDR capability.
    hdr: Option<bool>,
}

#[derive(Deserialize, Default)]
struct Pos {
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KMode {
    id: String,
    size: Size,
    refresh_rate: f64,
}

#[derive(Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

const TYPE_PANEL: i32 = 7;

impl KMode {
    fn mode(&self) -> Mode {
        Mode { width: self.size.width, height: self.size.height, refresh: Refresh::from_hz(self.refresh_rate) }
    }
}

/// The connected outputs in `json`.
pub fn parse(json: &str) -> Result<Vec<Monitor>, serde_json::Error> {
    let config: Config = serde_json::from_str(json)?;
    Ok(config.outputs.into_iter().filter(|o| o.connected).map(monitor).collect())
}

fn monitor(o: Output) -> Monitor {
    let find = |id: &str| o.modes.iter().find(|m| m.id == id).map(KMode::mode);
    let current = find(&o.current_mode_id);
    let preferred = o.preferred_modes.first().and_then(|id| find(id));
    let mut modes: Vec<Mode> = o.modes.iter().map(KMode::mode).collect();
    tidy_modes(&mut modes);
    let enabled = o.enabled.unwrap_or(true);
    let id = MonitorId { vendor: String::new(), product: String::new(), serial: None, connector: o.name.clone() };
    Monitor {
        key: id.key(),
        id,
        name: o.name.clone(),
        enabled,
        primary: enabled && o.priority == 1,
        priority: (o.priority > 0).then_some(o.priority),
        position: (o.pos.x, o.pos.y),
        current: current.filter(|_| enabled),
        preferred,
        modes,
        hdr: o.hdr.map(|on| Feature { supported: true, enabled: on }),
        vrr: o.vrr_policy.map(|p| Feature { supported: true, enabled: p != 0 }),
        builtin: o.kind == TYPE_PANEL,
        virtual_display: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, PrimarySupport, Snapshot};

    const FIXTURE: &str = include_str!("../../fixtures/kscreen-doctor.json");

    #[test]
    fn reads_outputs_modes_and_priority() {
        let snap = Snapshot::new(Backend::Kde, parse(FIXTURE).unwrap());
        // The unplugged HDMI port is left out; the laptop panel is disabled.
        let names: Vec<&str> = snap.monitors.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["DP-1", "DP-2", "eDP-1"]);
        let dp1 = &snap.monitors[0];
        assert_eq!(dp1.key, "@DP-1");
        assert!(dp1.primary);
        assert_eq!(dp1.priority, Some(1));
        assert_eq!(dp1.current, Some(Mode { width: 3840, height: 2160, refresh: Refresh { num: 143_999, den: 1000 } }));
        assert_eq!(dp1.preferred.map(|m| m.refresh.millihertz()), Some(143_999));
        assert_eq!(dp1.modes.len(), 3);
        assert_eq!(dp1.hdr, Some(Feature { supported: true, enabled: true }));
        assert_eq!(dp1.vrr, Some(Feature { supported: true, enabled: true }));

        let dp2 = &snap.monitors[1];
        assert!(!dp2.primary);
        assert_eq!(dp2.position, (3840, 0));
        assert_eq!(dp2.hdr, None);
        assert_eq!(dp2.vrr, Some(Feature { supported: true, enabled: false }));

        let panel = &snap.monitors[2];
        assert!(panel.builtin && !panel.enabled && !panel.primary);
        assert_eq!(panel.current, None);
    }

    #[test]
    fn kde_uses_priority_for_the_main_display() {
        let snap = Snapshot::new(Backend::Kde, parse(FIXTURE).unwrap());
        assert_eq!(snap.capabilities.primary, PrimarySupport::Priority);
        assert!(snap.capabilities.hdr && snap.capabilities.vrr);
    }

    #[test]
    fn an_empty_or_odd_config_is_no_monitors_not_a_crash() {
        assert!(parse(r#"{"outputs":[]}"#).unwrap().is_empty());
        assert!(parse(r#"{"tabletModeAvailable":false}"#).unwrap().is_empty());
        assert!(parse("not json").is_err());
    }
}
