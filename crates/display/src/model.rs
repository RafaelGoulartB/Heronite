//! What a probe finds: the monitors, their modes, and what the desktop lets
//! Heronite change. Plain data, the same on every platform.

use std::fmt;

use serde::Serialize;

/// A refresh rate as a fraction of hertz. Windows reports exact rationals
/// (`144000/1001`); the Linux tools report floats, kept as millihertz
/// (`143999/1000`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Refresh {
    pub num: u32,
    pub den: u32,
}

impl Refresh {
    /// `hz` rounded to the millihertz.
    pub fn from_hz(hz: f64) -> Refresh {
        Refresh { num: (hz.max(0.) * 1000.).round() as u32, den: 1000 }
    }

    pub fn hz(self) -> f64 {
        if self.den == 0 {
            0.
        } else {
            self.num as f64 / self.den as f64
        }
    }

    /// The rate in millihertz, for comparing rates written differently.
    pub fn millihertz(self) -> u32 {
        (self.hz() * 1000.).round() as u32
    }

    /// Whether two rates are the same one written differently (59.94 as
    /// `60000/1001` and as `59940/1000`, or as the tools' two-decimal floats):
    /// within 10 mHz. Tight on purpose: 59.94 and 60 Hz are two modes, 60 mHz
    /// apart.
    pub fn close_to(self, other: Refresh) -> bool {
        self.millihertz().abs_diff(other.millihertz()) <= 10
    }
}

impl fmt::Display for Refresh {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hz = self.hz();
        if (hz - hz.round()).abs() < 0.0005 {
            write!(f, "{} Hz", hz.round())
        } else {
            write!(f, "{hz:.3} Hz")
        }
    }
}

/// One display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh: Refresh,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{} @ {}", self.width, self.height, self.refresh)
    }
}

/// Sorted largest first, then fastest first; exact duplicates (and rates
/// within a millihertz of each other at the same size) dropped.
pub fn tidy_modes(modes: &mut Vec<Mode>) {
    modes.sort_by_key(|m| std::cmp::Reverse((m.width * m.height, m.width, m.refresh.millihertz())));
    modes.dedup_by(|a, b| a.width == b.width && a.height == b.height && a.refresh.millihertz() == b.refresh.millihertz());
}

/// What identifies a monitor across reboots, cable swaps on the same port and
/// driver updates: the EDID's vendor and product (and serial when it has
/// one), with the connector as the tiebreaker.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct MonitorId {
    /// The EDID vendor: a three-letter PNP id (`DEL`) on Windows, the make
    /// the compositor reports on Linux (`Dell Inc.`). Empty when unknown
    /// (KDE does not expose it).
    pub vendor: String,
    /// The EDID product code (`A1D0`) or model name.
    pub product: String,
    /// The EDID serial, when the monitor has a real one.
    pub serial: Option<String>,
    /// Where it is plugged in: `DP-1` on Linux; on Windows the instance part
    /// of the monitor's device path (`5&209d169f&0&UID4353`), which follows
    /// the port.
    pub connector: String,
}

impl MonitorId {
    /// A stable string for settings files: `vendor:product:serial`, or
    /// `vendor:product@connector` when the monitor has no usable serial (two
    /// identical monitors then differ by port), or `@connector` alone when
    /// the desktop says nothing about the monitor.
    pub fn key(&self) -> String {
        match (&self.serial, self.vendor.is_empty() && self.product.is_empty()) {
            (_, true) => format!("@{}", self.connector),
            (Some(serial), false) => format!("{}:{}:{}", self.vendor, self.product, serial),
            (None, false) => format!("{}:{}@{}", self.vendor, self.product, self.connector),
        }
    }
}

/// A serial worth matching on: not empty and not one of the placeholders
/// monitors without one report. `01010101` is the one many LG and Samsung
/// panels and TVs share, so it would make identical twins one monitor.
pub fn real_serial(s: &str) -> Option<String> {
    let s = s.trim();
    // Mutter writes the EDID's serial number field as `0x%08x`.
    let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    let placeholder = s.is_empty()
        || s.eq_ignore_ascii_case("unknown")
        || digits.chars().all(|c| c == '0')
        || s.chars().all(|c| c == '1')
        || digits.eq_ignore_ascii_case("01010101")
        || digits.eq_ignore_ascii_case("ffffffff");
    (!placeholder).then(|| s.to_string())
}

/// An on/off feature of a monitor (HDR, VRR) as far as the desktop says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Feature {
    pub supported: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Monitor {
    pub id: MonitorId,
    /// [`MonitorId::key`], precomputed for the JSON.
    pub key: String,
    /// What to show: the EDID's name (`DELL U2723QE`), else the connector.
    pub name: String,
    pub enabled: bool,
    /// The primary monitor (Windows, GNOME, X11) or the one with priority 1
    /// (KDE). Always false where there is no such thing.
    pub primary: bool,
    /// KDE's output priority (1 = the main display).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
    /// Top-left corner on the desktop.
    pub position: (i32, i32),
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Mode>,
    pub modes: Vec<Mode>,
    /// `None`: the desktop does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdr: Option<Feature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vrr: Option<Feature>,
    /// A laptop panel.
    pub builtin: bool,
    /// A display that exists only in software (Sunshine/Apollo's virtual
    /// display, Parsec, Virtual Desktop). Their ids are less stable.
    #[serde(rename = "virtual")]
    pub virtual_display: bool,
}

/// Which tool or API read the state, and later applies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Windows,
    /// KDE Plasma, through `kscreen-doctor`.
    Kde,
    /// GNOME, through Mutter's `org.gnome.Mutter.DisplayConfig`.
    Gnome,
    /// Sway, river, labwc and other wlroots compositors, through `wlr-randr`.
    Wlroots,
    /// Hyprland, through `hyprctl`.
    Hyprland,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Windows => "Windows",
            Backend::Kde => "KDE Plasma (kscreen-doctor)",
            Backend::Gnome => "GNOME (Mutter)",
            Backend::Wlroots => "wlroots (wlr-randr)",
            Backend::Hyprland => "Hyprland (hyprctl)",
        }
    }
}

/// How a desktop names "the monitor games and the taskbar start on".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrimarySupport {
    /// A primary monitor (Windows, GNOME, X11).
    Primary,
    /// An output priority, 1 being the main display (KDE).
    Priority,
    /// No such notion (Hyprland, wlroots): the UI hides the option.
    None,
}

/// What the settings UI may offer on this desktop. Anything false is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub primary: PrimarySupport,
    /// HDR can be switched (on at least one monitor that supports it).
    pub hdr: bool,
    /// VRR can be switched.
    pub vrr: bool,
    /// Refresh rates are exact fractions rather than rounded floats.
    pub exact_refresh: bool,
}

impl Capabilities {
    /// What `backend` can do, given the monitors it reported.
    pub fn of(backend: Backend, monitors: &[Monitor]) -> Capabilities {
        let any = |f: fn(&Monitor) -> Option<Feature>| monitors.iter().any(|m| f(m).is_some_and(|x| x.supported));
        let hdr = any(|m| m.hdr);
        let vrr = any(|m| m.vrr);
        match backend {
            // Windows has no public per-monitor VRR switch.
            Backend::Windows => Capabilities { primary: PrimarySupport::Primary, hdr, vrr: false, exact_refresh: true },
            Backend::Kde => Capabilities { primary: PrimarySupport::Priority, hdr, vrr, exact_refresh: false },
            Backend::Gnome => Capabilities { primary: PrimarySupport::Primary, hdr, vrr, exact_refresh: false },
            // wlr-output-management has no HDR.
            Backend::Wlroots => Capabilities { primary: PrimarySupport::None, hdr: false, vrr, exact_refresh: false },
            Backend::Hyprland => Capabilities { primary: PrimarySupport::None, hdr, vrr, exact_refresh: false },
        }
    }
}

/// Everything one probe found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub backend: Backend,
    pub capabilities: Capabilities,
    /// Left to right, then top to bottom; disabled ones last.
    pub monitors: Vec<Monitor>,
}

impl Snapshot {
    /// Puts `monitors` in display order, keeps their keys apart and works
    /// out the capabilities.
    pub fn new(backend: Backend, mut monitors: Vec<Monitor>) -> Snapshot {
        monitors.sort_by_key(|m| (!m.enabled, m.position.0, m.position.1, m.id.connector.clone()));
        separate_keys(&mut monitors);
        Snapshot { backend, capabilities: Capabilities::of(backend, &monitors), monitors }
    }
}

/// Two monitors with one key (twins reporting the same serial, a placeholder
/// [`real_serial`] does not know): their serial does not tell them apart, so
/// it is dropped and the connector does, as for monitors without a serial.
fn separate_keys(monitors: &mut [Monitor]) {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for m in monitors.iter() {
        *seen.entry(m.key.clone()).or_default() += 1;
    }
    for m in monitors.iter_mut().filter(|m| seen[&m.key] > 1 && m.id.serial.is_some()) {
        m.id.serial = None;
        m.key = m.id.key();
    }
}

/// Why there is nothing to show (and, later, nothing Heronite will change).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", content = "detail", rename_all = "kebab-case")]
pub enum Unsupported {
    /// A Remote Desktop session: the monitors are the remote client's.
    RemoteSession,
    /// Running inside a gamescope session (Steam Deck style): gamescope owns
    /// the outputs; only gamescope's own options apply.
    GamescopeSession,
    /// The desktop is known but its tool is missing (`kscreen-doctor`).
    ToolMissing(String),
    /// No backend for this desktop (`XDG_CURRENT_DESKTOP`).
    UnknownDesktop(String),
    /// Not Windows or Linux.
    Platform,
    /// The tool or API failed.
    Failed(String),
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unsupported::RemoteSession => write!(f, "this is a Remote Desktop session"),
            Unsupported::GamescopeSession => write!(f, "gamescope runs the session"),
            Unsupported::ToolMissing(tool) => write!(f, "{tool} is not installed"),
            Unsupported::UnknownDesktop(d) if d.is_empty() => write!(f, "the desktop is unknown"),
            Unsupported::UnknownDesktop(d) => write!(f, "no support for the {d} desktop"),
            Unsupported::Platform => write!(f, "not supported on this system"),
            Unsupported::Failed(e) => write!(f, "reading the displays failed: {e}"),
        }
    }
}

impl std::error::Error for Unsupported {}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(vendor: &str, product: &str, serial: Option<&str>, connector: &str) -> MonitorId {
        MonitorId { vendor: vendor.into(), product: product.into(), serial: serial.map(Into::into), connector: connector.into() }
    }

    #[test]
    fn keys_use_the_serial_then_the_connector() {
        assert_eq!(id("DEL", "A1D0", Some("ABC123"), "DP-1").key(), "DEL:A1D0:ABC123");
        // Identical twins without serials differ by port.
        let left = id("GBT", "273E", None, "DP-1").key();
        let right = id("GBT", "273E", None, "DP-2").key();
        assert_eq!(left, "GBT:273E@DP-1");
        assert_ne!(left, right);
        // KDE: nothing but the connector.
        assert_eq!(id("", "", None, "HDMI-A-1").key(), "@HDMI-A-1");
    }

    #[test]
    fn placeholder_serials_are_not_serials() {
        for s in ["", "  ", "0", "00000000", "Unknown", "0x00000000", "11111111", "01010101", "0x01010101", "FFFFFFFF"] {
            assert_eq!(real_serial(s), None, "{s:?}");
        }
        assert_eq!(real_serial(" 5CD2140XYZ "), Some("5CD2140XYZ".into()));
        assert_eq!(real_serial("0x0006d3c1"), Some("0x0006d3c1".into()));
        // A real serial may hold a run of zeros, or the pattern inside it.
        assert_eq!(real_serial("1010101010"), Some("1010101010".into()));
    }

    #[test]
    fn refresh_rates_compare_across_notations() {
        let ntsc = Refresh { num: 60000, den: 1001 };
        assert!(ntsc.close_to(Refresh::from_hz(59.94)));
        assert!(Refresh::from_hz(143.99).close_to(Refresh::from_hz(143.988)));
        // 59.94 and 60 Hz are two modes.
        assert!(!ntsc.close_to(Refresh::from_hz(60.)));
        assert!(!ntsc.close_to(Refresh::from_hz(50.)));
        assert_eq!(ntsc.to_string(), "59.940 Hz");
        assert_eq!(Refresh { num: 144, den: 1 }.to_string(), "144 Hz");
        assert_eq!(Refresh { num: 1, den: 0 }.hz(), 0.);
    }

    fn monitor(serial: Option<&str>, connector: &str, x: i32) -> Monitor {
        let id = id("GSM", "5B9B", serial, connector);
        Monitor {
            key: id.key(),
            name: connector.into(),
            id,
            enabled: true,
            primary: false,
            priority: None,
            position: (x, 0),
            current: None,
            preferred: None,
            modes: Vec::new(),
            hdr: None,
            vrr: None,
            builtin: false,
            virtual_display: false,
        }
    }

    #[test]
    fn twins_sharing_a_serial_are_told_apart_by_their_port() {
        let snap = Snapshot::new(
            Backend::Hyprland,
            vec![monitor(Some("ABC"), "DP-1", 0), monitor(Some("ABC"), "DP-2", 2560), monitor(Some("XYZ"), "HDMI-A-1", 5120)],
        );
        let keys: Vec<&str> = snap.monitors.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["GSM:5B9B@DP-1", "GSM:5B9B@DP-2", "GSM:5B9B:XYZ"]);
        assert_eq!(snap.monitors[0].id.serial, None);
        // Alone, the same monitor keeps its serial.
        let alone = Snapshot::new(Backend::Hyprland, vec![monitor(Some("ABC"), "DP-1", 0)]);
        assert_eq!(alone.monitors[0].key, "GSM:5B9B:ABC");
    }

    #[test]
    fn modes_are_sorted_and_deduplicated() {
        let m = |w, h, hz| Mode { width: w, height: h, refresh: Refresh::from_hz(hz) };
        let mut modes = vec![m(1920, 1080, 60.), m(3840, 2160, 60.), m(3840, 2160, 144.), m(1920, 1080, 60.0004), m(2560, 1440, 120.)];
        tidy_modes(&mut modes);
        assert_eq!(modes, vec![m(3840, 2160, 144.), m(3840, 2160, 60.), m(2560, 1440, 120.), m(1920, 1080, 60.)]);
    }
}
