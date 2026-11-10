//! Hyprland integration through `hyprctl`. Only the JSON parsing is unit-tested;
//! the functions that run `hyprctl eval` change the compositor's state.

use super::{
    hyprland_bind_expression_as, hyprland_signal_command, hyprland_unbind_expression, HyprlandBind, HyprlandBinding, Monitor,
    BIND_DESCRIPTION,
};
use heronite_i18n::tr;
use serde::Deserialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(2);

pub fn is_hyprland_session() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
}

/// Why Hyprland did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Hyprland answered and said no (the keys are taken, it does not know
    /// the key). The text is for the user.
    Refused(String),
    /// Hyprland could not be asked: no `hyprctl`, no answer in time, or a
    /// version without `hyprctl eval`.
    Unavailable(String),
}

impl Failure {
    pub fn message(self) -> String {
        match self {
            Self::Refused(m) | Self::Unavailable(m) => m,
        }
    }
}

/// `execFileSync('hyprctl', args, { timeout: 2000 })`. Hyprland's own answer
/// to a request it refuses is `Refused`.
fn hyprctl(args: &[&str]) -> Result<String, Failure> {
    let unavailable = |e: String| Failure::Unavailable(e);
    let mut child = Command::new("hyprctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| unavailable(format!("hyprctl: {e}")))?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable("hyprctl timed out".into()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(unavailable(format!("hyprctl: {e}"))),
        }
    };
    let out = reader.join().unwrap_or_default();
    if is_unknown_request(&out) {
        return Err(unavailable(format!("hyprctl {}: {}", args.first().copied().unwrap_or_default(), out.trim())));
    }
    if status.success() {
        Ok(out)
    } else {
        Err(Failure::Refused(failure_text(args, &out, &status.to_string())))
    }
}

/// What a Hyprland that does not have the command answers (with exit code 0).
fn is_unknown_request(out: &str) -> bool {
    out.trim().eq_ignore_ascii_case("unknown request")
}

/// The reason Hyprland gave, else which command failed.
fn failure_text(args: &[&str], out: &str, status: &str) -> String {
    let reason = out.trim();
    if reason.is_empty() {
        format!("hyprctl {} failed ({status})", args.join(" "))
    } else {
        format!("hyprctl {}: {reason}", args.first().copied().unwrap_or_default())
    }
}

/// `hyprctl eval` can answer "ok" or an error text with exit code 0.
fn eval(expression: &str) -> Result<(), Failure> {
    let out = hyprctl(&["eval", expression])?;
    let t = out.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(Failure::Refused(format!("hyprctl eval: {t}")))
    }
}

pub(super) fn binds() -> Result<Vec<HyprlandBind>, Failure> {
    let out = hyprctl(&["binds", "-j"])?;
    serde_json::from_str(&out).map_err(|e| Failure::Unavailable(format!("hyprctl binds: {e}")))
}

pub fn unregister(keys: &str) -> Result<(), String> {
    eval(&hyprland_unbind_expression(keys)).map_err(Failure::message)
}

/// The command of a bind that sends this process `signal` (see
/// [`hyprland_signal_command`]).
pub fn signal_command(signal: i32) -> String {
    let comm = std::fs::read_to_string("/proc/self/comm").unwrap_or_default();
    hyprland_signal_command(&signal.to_string(), std::process::id(), comm.trim_end())
}

/// `registerHyprlandHotkey`: the temporary screenshot bind, which sends this
/// process SIGUSR2 (the game gets the keys too).
pub fn register(binding: &HyprlandBinding, accelerator: &str) -> Result<(), Failure> {
    register_as(binding, accelerator, &signal_command(signal_hook::consts::signal::SIGUSR2), BIND_DESCRIPTION, false)
}

/// Installs a temporary bind that runs `command`, described as
/// `description`, and checks that Hyprland kept it. Refuses keys another
/// bind has.
///
/// Each description is one shortcut of one running Heronite, so any bind
/// carrying it by now was left behind by one that crashed (on these keys or
/// on others, if the shortcut changed since): those go first.
pub fn register_as(binding: &HyprlandBinding, accelerator: &str, command: &str, description: &str, consume: bool) -> Result<(), Failure> {
    let mut current = binds()?;
    let stale: Vec<String> =
        current.iter().filter(|b| b.description.as_deref() == Some(description)).filter_map(HyprlandBind::keys).collect();
    if !stale.is_empty() {
        for keys in stale {
            log::info!("removing the Hyprland bind {keys} ({description}) left by an earlier run");
            eval(&hyprland_unbind_expression(&keys))?;
        }
        current = binds()?;
    }
    if current.iter().any(|b| b.is_same_trigger(binding)) {
        return Err(Failure::Refused(tr!("The shortcut {accelerator} is already in use", accelerator)));
    }
    eval(&hyprland_bind_expression_as(binding, command, description, consume))?;
    if !binds()?.iter().any(|b| b.is_ours_as(binding, description)) {
        return Err(Failure::Refused(tr!("Hyprland did not retain the shortcut").into()));
    }
    Ok(())
}

#[derive(Deserialize)]
struct RawMonitor {
    name: String,
    x: i32,
    y: i32,
    width: f64,
    height: f64,
    #[serde(default = "one")]
    scale: f64,
    #[serde(default)]
    transform: u32,
    #[serde(default)]
    focused: bool,
}

fn one() -> f64 {
    1.0
}

/// Parses `hyprctl -j monitors` into logical-pixel rectangles.
pub fn parse_monitors(json: &str) -> Result<Vec<Monitor>, String> {
    let raw: Vec<RawMonitor> = serde_json::from_str(json).map_err(|e| format!("monitors: {e}"))?;
    Ok(raw
        .into_iter()
        .map(|m| {
            let scale = if m.scale > 0.0 { m.scale } else { 1.0 };
            let (w, h) = if m.transform % 2 == 1 { (m.height, m.width) } else { (m.width, m.height) };
            Monitor {
                name: m.name,
                x: m.x,
                y: m.y,
                width: (w / scale).round() as i32,
                height: (h / scale).round() as i32,
                focused: m.focused,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct CursorPos {
    x: f64,
    y: f64,
}

pub fn parse_cursor_pos(json: &str) -> Result<(i32, i32), String> {
    let p: CursorPos = serde_json::from_str(json).map_err(|e| format!("cursorpos: {e}"))?;
    Ok((p.x.floor() as i32, p.y.floor() as i32))
}

/// Name of the output under the pointer (read-only queries).
pub fn output_under_pointer() -> Result<String, String> {
    let monitors = parse_monitors(&hyprctl(&["-j", "monitors"]).map_err(Failure::message)?)?;
    let pointer = parse_cursor_pos(&hyprctl(&["-j", "cursorpos"]).map_err(Failure::message)?).ok();
    let found = match pointer {
        Some((x, y)) => super::monitor_under_pointer(&monitors, x, y),
        None => monitors.iter().find(|m| m.focused).or(monitors.first()),
    };
    found.map(|m| m.name.clone()).ok_or_else(|| "Hyprland reported no monitors".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_monitors_and_cursor() {
        let json = r#"[
          {"id":0,"name":"DP-1","width":3840,"height":2160,"refreshRate":143.9,"x":0,"y":0,"scale":1.5,"transform":0,"focused":true},
          {"id":1,"name":"HDMI-A-1","width":1920,"height":1080,"x":2560,"y":0,"scale":1.00,"transform":1,"focused":false}
        ]"#;
        let ms = parse_monitors(json).unwrap();
        assert_eq!(ms.len(), 2);
        assert_eq!((ms[0].width, ms[0].height), (2560, 1440));
        assert!(ms[0].focused);
        // Rotated 90 degrees: width and height swap.
        assert_eq!((ms[1].width, ms[1].height), (1080, 1920));
        assert_eq!(super::super::monitor_under_pointer(&ms, 2600, 1500).unwrap().name, "HDMI-A-1");
        assert_eq!(parse_cursor_pos(r#"{"x":1920.5,"y":10.0}"#).unwrap(), (1920, 10));
        assert!(parse_monitors("nope").is_err());
    }

    #[test]
    fn hyprctl_answers() {
        assert!(is_unknown_request("unknown request\n"));
        assert!(!is_unknown_request("ok"));
        // Hyprland's own words when it has some.
        let lua = "error: hl.bind: failed to parse key string: Unknown keysym: \"Nope\"\n";
        assert_eq!(
            failure_text(&["eval", "hl.bind(...)"], lua, "exit status: 7"),
            "hyprctl eval: error: hl.bind: failed to parse key string: Unknown keysym: \"Nope\""
        );
        assert_eq!(failure_text(&["binds", "-j"], "", "exit status: 1"), "hyprctl binds -j failed (exit status: 1)");
    }
}
