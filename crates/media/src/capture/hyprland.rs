//! Hyprland integration through `hyprctl`. Only the JSON parsing is unit-tested;
//! the functions that run `hyprctl eval` change the compositor's state.

use super::{
    hyprland_bind_expression, hyprland_unbind_expression, HyprlandBind, HyprlandBinding, Monitor,
};
use serde::Deserialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(2);

pub fn is_hyprland_session() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
}

/// `execFileSync('hyprctl', args, { timeout: 2000 })`.
fn hyprctl(args: &[&str]) -> Result<String, String> {
    let mut child = Command::new("hyprctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("hyprctl: {e}"))?;
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
                return Err("hyprctl timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(format!("hyprctl: {e}")),
        }
    };
    let out = reader.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(format!("hyprctl {} failed ({status})", args.join(" ")))
    }
}

/// `hyprctl eval` can answer "ok" or an error text with exit code 0.
fn eval(expression: &str) -> Result<(), String> {
    let out = hyprctl(&["eval", expression])?;
    let t = out.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(format!("hyprctl eval: {t}"))
    }
}

fn binds() -> Result<Vec<HyprlandBind>, String> {
    let out = hyprctl(&["binds", "-j"])?;
    serde_json::from_str(&out).map_err(|e| format!("hyprctl binds: {e}"))
}

pub fn unregister(keys: &str) -> Result<(), String> {
    eval(&hyprland_unbind_expression(keys))
}

/// `registerHyprlandHotkey`: drops a stale bind of ours, refuses a conflicting
/// bind, installs the temporary bind and checks that Hyprland kept it.
pub fn register(binding: &HyprlandBinding, accelerator: &str, pid: u32) -> Result<(), String> {
    let mut current = binds()?;
    if current.iter().any(|b| b.is_ours(binding)) {
        eval(&hyprland_unbind_expression(&binding.keys))?;
        current = binds()?;
    }
    if current.iter().any(|b| b.is_same_trigger(binding)) {
        return Err(format!("The shortcut {accelerator} is already in use"));
    }
    eval(&hyprland_bind_expression(binding, pid))?;
    if !binds()?.iter().any(|b| b.is_ours(binding)) {
        return Err("Hyprland did not retain the shortcut".into());
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
    let monitors = parse_monitors(&hyprctl(&["-j", "monitors"])?)?;
    let pointer = parse_cursor_pos(&hyprctl(&["-j", "cursorpos"])?).ok();
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
}
