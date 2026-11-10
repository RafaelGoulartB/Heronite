//! Per-game display settings, phase 1: reading the displays.
//!
//! [`probe`] reports the monitors (identified by EDID, see
//! [`MonitorId::key`]), their current and available modes, HDR and VRR, and
//! what the desktop lets Heronite change ([`Capabilities`], which the
//! settings UI uses to hide what a compositor cannot do).
//!
//! * Windows: `QueryDisplayConfig` and `DisplayConfigGetDeviceInfo`.
//! * KDE: `kscreen-doctor -j`. GNOME: Mutter's `org.gnome.Mutter.DisplayConfig`.
//!   wlroots: `wlr-randr --json`. Hyprland: `hyprctl monitors all -j`.
//!
//! **Nothing in this crate changes a display.** Applying and restoring
//! settings come in later phases (see the design shared on
//! RafaelGoulartB/Heronite#233).

pub mod desktop;
pub mod edid;
#[cfg(target_os = "linux")]
mod linux;
pub mod model;
pub mod parse;
// Its mapping is tested everywhere; only Windows calls it.
#[cfg_attr(not(windows), allow(dead_code))]
mod windows;

pub use model::{Backend, Capabilities, Feature, Mode, Monitor, MonitorId, PrimarySupport, Refresh, Snapshot, Unsupported};

/// Reads the displays. Blocking (a tool runs, or a D-Bus call is made on
/// Linux): call it off the UI thread.
pub fn probe() -> Result<Snapshot, Unsupported> {
    #[cfg(windows)]
    {
        if windows::api::remote_session() {
            return Err(Unsupported::RemoteSession);
        }
        let targets = windows::api::targets()?;
        Ok(Snapshot::new(Backend::Windows, windows::monitors(targets)))
    }
    #[cfg(target_os = "linux")]
    {
        let backend = desktop::detect(|name| std::env::var(name).ok())?;
        Ok(Snapshot::new(backend, linux::monitors(backend)?))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(Unsupported::Platform)
    }
}

/// What `--display-probe` prints: the snapshot, or why there is none, as
/// pretty JSON.
pub fn probe_json() -> String {
    let value = match probe() {
        Ok(snapshot) => serde_json::json!({ "ok": true, "snapshot": snapshot }),
        Err(unsupported) => serde_json::json!({ "ok": false, "unsupported": unsupported, "message": unsupported.to_string() }),
    };
    serde_json::to_string_pretty(&value).unwrap_or_default()
}
