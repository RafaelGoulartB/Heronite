//! Which monitor to record: the one the game's window is on. Hyprland says
//! (`hyprctl -j clients`); elsewhere the focused monitor or the first.

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;
use serde_json::Value;

use crate::procs::belongs_to;

/// The monitor (connector name) the game whose process (group leader) is
/// `game_pid` shows on, else the focused one. `None` outside Hyprland.
/// Blocking.
pub fn game_monitor(game_pid: Option<u32>) -> Option<String> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let monitors = hyprctl("monitors")?;
    let clients = game_pid.and_then(|_| hyprctl("clients")).unwrap_or(Value::Null);
    pick(&monitors, &clients, |pid| game_pid.is_some_and(|g| belongs_to(pid, g)))
}

fn hyprctl(what: &str) -> Option<Value> {
    let out = Run::new().timeout(Duration::from_secs(3)).run(Command::new("hyprctl").args(["-j", what])).ok()?;
    out.success().then(|| serde_json::from_str(&out.stdout_text()).ok()).flatten()
}

/// The monitor of the game's window (a fullscreen one first), else the
/// focused monitor, else the first.
fn pick(monitors: &Value, clients: &Value, is_game: impl Fn(u32) -> bool) -> Option<String> {
    let monitors = monitors.as_array()?;
    let name_of = |id: i64| monitors.iter().find(|m| m["id"].as_i64() == Some(id)).and_then(|m| m["name"].as_str());
    let game = clients.as_array().into_iter().flatten().filter(|c| {
        c["mapped"].as_bool().unwrap_or(true)
            && !c["hidden"].as_bool().unwrap_or(false)
            && c["pid"].as_u64().and_then(|p| u32::try_from(p).ok()).is_some_and(&is_game)
    });
    let game = game.max_by_key(|c| c["fullscreen"].as_i64().unwrap_or(0) > 0);
    game.and_then(|c| c["monitor"].as_i64())
        .and_then(name_of)
        .or_else(|| monitors.iter().find(|m| m["focused"].as_bool() == Some(true)).and_then(|m| m["name"].as_str()))
        .or_else(|| monitors.first().and_then(|m| m["name"].as_str()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_game_window_picks_the_monitor() {
        let monitors = json!([{"id": 0, "name": "DP-1", "focused": true}, {"id": 1, "name": "HDMI-A-1", "focused": false}]);
        let clients = json!([
            {"pid": 10, "monitor": 0, "mapped": true, "hidden": false, "fullscreen": 0},
            {"pid": 20, "monitor": 1, "mapped": true, "hidden": false, "fullscreen": 0},
            {"pid": 21, "monitor": 1, "mapped": true, "hidden": false, "fullscreen": 2},
        ]);
        assert_eq!(pick(&monitors, &clients, |p| p == 20 || p == 21).as_deref(), Some("HDMI-A-1"));
        assert_eq!(pick(&monitors, &clients, |p| p == 99).as_deref(), Some("DP-1"), "no window yet: the focused monitor");
        assert_eq!(pick(&json!([{"id": 3, "name": "eDP-1"}]), &Value::Null, |_| false).as_deref(), Some("eDP-1"));
        assert_eq!(pick(&Value::Null, &clients, |_| true), None);
    }
}
