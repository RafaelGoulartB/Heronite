//! The game's window on the desktop: whether another one has the focus, and
//! giving it back ("Back to game" in the overlay, after "Back to Heronite" or
//! a switch to another app). Heronite's own window too, by its pid (console
//! mode's button, `console::summon`).
//!
//! Only Hyprland can be asked (`hyprctl`): its windows carry the pid they
//! belong to, which says which one is the game's. Elsewhere nothing does, so
//! the game never counts as away and the overlay offers nothing.

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;
use serde_json::Value;

use crate::procs::belongs_to;

/// Whether the game has a window and another window has the focus. `false`
/// when the desktop cannot say. Blocking.
pub fn away(game_pid: u32) -> bool {
    if !on_hyprland() {
        return false;
    }
    let Ok(clients) = hyprctl(&["clients", "-j"]) else { return false };
    let windows = game_windows(&clients, |pid| belongs_to(pid, game_pid));
    if windows.is_empty() {
        return false;
    }
    let active = hyprctl(&["activewindow", "-j"]).ok().and_then(|json| address_of(&json));
    !windows.iter().any(|w| Some(w) == active.as_ref())
}

/// Gives the focus to the game's window (the one that had it last, when it
/// has several). Blocking.
pub fn focus(game_pid: u32) -> Result<(), String> {
    if !on_hyprland() {
        return Err("only Hyprland can be asked for the game's window".into());
    }
    let clients = hyprctl(&["clients", "-j"])?;
    let windows = game_windows(&clients, |pid| belongs_to(pid, game_pid));
    let Some(address) = windows.first() else { return Err("the game has no window".into()) };
    // Hyprland with a Lua config takes its dispatchers as Lua, older ones by name.
    let lua = format!("hl.dsp.focus({{ window = \"address:{address}\" }})");
    let by_name = format!("address:{address}");
    let mut answer = String::new();
    for args in [vec!["dispatch", lua.as_str()], vec!["dispatch", "focuswindow", by_name.as_str()]] {
        // It answers "ok", or says why not with any exit code.
        answer = hyprctl(&args).unwrap_or_else(|e| e);
        if answer.trim() == "ok" {
            return Ok(());
        }
    }
    Err(answer.lines().next().unwrap_or("hyprctl gave no answer").trim().to_string())
}

/// Whether the game's window can be found and focused here (Hyprland).
pub fn supported() -> bool {
    on_hyprland()
}

fn on_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// What `hyprctl` printed (its errors too: it prints them on either stream).
fn hyprctl(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("hyprctl");
    cmd.args(args);
    let out = Run::new().timeout(Duration::from_secs(2)).run(&mut cmd).map_err(|e| format!("hyprctl: {e}"))?;
    let text = out.stdout_text();
    Ok(if text.trim().is_empty() { out.stderr_text() } else { text })
}

/// The addresses of the game's windows in `hyprctl clients -j`, the one
/// focused last first.
fn game_windows(clients: &str, is_game: impl Fn(u32) -> bool) -> Vec<String> {
    let Ok(Value::Array(clients)) = serde_json::from_str::<Value>(clients) else { return Vec::new() };
    let mut windows: Vec<(i64, String)> = clients
        .iter()
        .filter(|c| c["mapped"].as_bool().unwrap_or(true))
        .filter(|c| c["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok()).is_some_and(|pid| pid > 0 && is_game(pid)))
        .filter_map(|c| Some((c["focusHistoryID"].as_i64().unwrap_or(i64::MAX), c["address"].as_str()?.to_string())))
        .collect();
    windows.sort();
    windows.into_iter().map(|(_, address)| address).collect()
}

/// The window of `hyprctl activewindow -j` (`{}` when none has the focus).
fn address_of(window: &str) -> Option<String> {
    serde_json::from_str::<Value>(window).ok()?["address"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
        {"address": "0xa", "mapped": true, "pid": 100, "focusHistoryID": 0},
        {"address": "0xb", "mapped": true, "pid": 200, "focusHistoryID": 4},
        {"address": "0xc", "mapped": true, "pid": 201, "focusHistoryID": 2},
        {"address": "0xd", "mapped": false, "pid": 200, "focusHistoryID": 1},
        {"address": "0xe", "mapped": true, "pid": -1, "focusHistoryID": 3}
    ]"#;

    #[test]
    fn the_games_windows_come_with_the_one_focused_last_first() {
        let game = |pid: u32| pid == 200 || pid == 201;
        assert_eq!(game_windows(CLIENTS, game), ["0xc", "0xb"], "not the unmapped one, nor the one without a pid");
        assert!(game_windows(CLIENTS, |_| false).is_empty());
        assert!(game_windows("not json", game).is_empty());
    }

    #[test]
    fn reads_the_focused_window() {
        assert_eq!(address_of(r#"{"address": "0xa", "pid": 100}"#).as_deref(), Some("0xa"));
        assert_eq!(address_of("{}"), None, "no window has the focus");
        assert_eq!(address_of("Invalid"), None);
    }
}
