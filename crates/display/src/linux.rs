//! Linux: runs the desktop's read command (through `flatpak-spawn --host` in
//! the Flatpak) or asks Mutter over D-Bus, and hands the answer to
//! [`crate::parse`]. Only reading commands are run.

use std::collections::HashMap;
use std::io::{ErrorKind, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zbus::zvariant::OwnedValue;

use crate::model::{Backend, Monitor, Unsupported};
use crate::parse::{self, mutter};

/// How long a tool may take before the probe gives up on it.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The monitors `backend` reports.
pub(crate) fn monitors(backend: Backend) -> Result<Vec<Monitor>, Unsupported> {
    let parsed = match backend {
        Backend::Kde => parse::kscreen::parse(&run("kscreen-doctor", &["-j"])?),
        Backend::Wlroots => parse::wlr::parse(&run("wlr-randr", &["--json"])?),
        Backend::Hyprland => parse::hyprland::parse(&run("hyprctl", &["monitors", "all", "-j"])?),
        Backend::Gnome => return mutter_monitors(),
        Backend::Windows => return Err(Unsupported::Platform),
    };
    parsed.map_err(|e| Unsupported::Failed(format!("unexpected output: {e}")))
}

/// `program args` on the host, its standard output, within [`TIMEOUT`].
fn run(program: &str, args: &[&str]) -> Result<String, Unsupported> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    let mut cmd = heronite_platform::flatpak::host(cmd);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| match e.kind() {
        ErrorKind::NotFound => Unsupported::ToolMissing(program.into()),
        _ => Unsupported::Failed(format!("{program}: {e}")),
    })?;
    // Read on a thread so a full pipe cannot stall the wait below.
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        stdout.read_to_string(&mut out).map(|_| out)
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Unsupported::Failed(format!("{program} did not answer within {} s", TIMEOUT.as_secs())));
            }
            Err(e) => return Err(Unsupported::Failed(format!("{program}: {e}"))),
        }
    };
    let out = reader.join().map_err(|_| Unsupported::Failed(format!("{program}: reading its output failed")))?;
    let out = out.map_err(|e| Unsupported::Failed(format!("{program}: {e}")))?;
    // flatpak-spawn reports a missing host program as exit status 127.
    match status.code() {
        Some(0) => Ok(out),
        Some(127) => Err(Unsupported::ToolMissing(program.into())),
        _ => Err(Unsupported::Failed(format!("{program} exited with {status}"))),
    }
}

type Props = HashMap<String, OwnedValue>;
type Spec = (String, String, String, String);
type DMode = (String, i32, i32, f64, f64, Vec<f64>, Props);
type DMonitor = (Spec, Vec<DMode>, Props);
type DLogical = (i32, i32, f64, u32, bool, Vec<Spec>, Props);

fn prop<T: TryFrom<OwnedValue>>(props: &Props, key: &str) -> Option<T> {
    props.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| T::try_from(v).ok())
}

/// `GetCurrentState`, unpacked for [`mutter::monitors`].
fn mutter_monitors() -> Result<Vec<Monitor>, Unsupported> {
    let failed = |e: zbus::Error| Unsupported::Failed(format!("Mutter DisplayConfig: {e}"));
    let conn = zbus::blocking::Connection::session().map_err(failed)?;
    let reply = conn
        .call_method(
            Some("org.gnome.Mutter.DisplayConfig"),
            "/org/gnome/Mutter/DisplayConfig",
            Some("org.gnome.Mutter.DisplayConfig"),
            "GetCurrentState",
            &(),
        )
        .map_err(failed)?;
    let (_serial, monitors, logical, _props): (u32, Vec<DMonitor>, Vec<DLogical>, Props) =
        reply.body().deserialize().map_err(failed)?;
    let monitors = monitors
        .into_iter()
        .map(|((connector, vendor, product, serial), modes, props)| mutter::MutterMonitor {
            connector,
            vendor,
            product,
            serial,
            modes: modes
                .into_iter()
                .map(|(_id, width, height, refresh, _scale, _scales, props)| mutter::MutterMode {
                    width,
                    height,
                    refresh,
                    current: prop(&props, "is-current").unwrap_or(false),
                    preferred: prop(&props, "is-preferred").unwrap_or(false),
                    variable: prop::<String>(&props, "refresh-rate-mode").as_deref() == Some("variable"),
                })
                .collect(),
            display_name: prop(&props, "display-name"),
            builtin: prop(&props, "is-builtin").unwrap_or(false),
            color_mode: prop(&props, "color-mode"),
            supported_color_modes: prop(&props, "supported-color-modes").unwrap_or_default(),
        })
        .collect();
    let logical: Vec<mutter::MutterLogical> = logical
        .into_iter()
        .map(|(x, y, _scale, _transform, primary, specs, _props)| mutter::MutterLogical {
            x,
            y,
            primary,
            connectors: specs.into_iter().map(|(connector, ..)| connector).collect(),
        })
        .collect();
    Ok(mutter::monitors(monitors, &logical))
}
