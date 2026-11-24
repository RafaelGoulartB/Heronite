//! The game's window on the desktop: whether another one has the focus, and
//! giving it back ("Back to game" in the overlay and the Collection, Enter on
//! a running game in the quick launcher). Heronite's own window too, by its
//! pid (console mode's button, `console::summon`).
//!
//! Only the window manager is asked, never the game:
//! - Hyprland (`hyprctl`) and Sway (`swaymsg`) list the pid of each window;
//! - X11 windows say theirs (`_NET_WM_PID`), and the window manager is asked
//!   to activate one (`_NET_ACTIVE_WINDOW`);
//! - Windows lists every top-level window with its process.
//!
//! KDE and GNOME on Wayland give no way to do it (KWin would need a script),
//! so there nothing is offered. Whether another window has the focus (the
//! overlay's "Back to game" row) is only asked of Hyprland.

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;
use serde_json::Value;

use crate::procs::belongs_to;

/// The window manager asked for the game's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Desktop {
    Hyprland,
    Sway,
    X11,
    Windows,
}

/// Which one runs here, `None` when none can be asked.
fn desktop() -> Option<Desktop> {
    if cfg!(windows) {
        return Some(Desktop::Windows);
    }
    if !cfg!(target_os = "linux") {
        return None;
    }
    desktop_of(|name| std::env::var_os(name).filter(|v| !v.is_empty()).map(|v| v.to_string_lossy().into_owned()))
}

/// [`desktop`] from the session's variables (Linux). A Wayland session
/// other than Hyprland and Sway has none, though its X11 apps see a
/// `DISPLAY` (Xwayland).
fn desktop_of(var: impl Fn(&str) -> Option<String>) -> Option<Desktop> {
    if var("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Some(Desktop::Hyprland)
    } else if var("SWAYSOCK").is_some() {
        Some(Desktop::Sway)
    } else if var("WAYLAND_DISPLAY").is_some() || var("XDG_SESSION_TYPE").is_some_and(|t| t == "wayland") {
        None
    } else if var("DISPLAY").is_some() {
        Some(Desktop::X11)
    } else {
        None
    }
}

/// Whether the game has a window and another window has the focus. `false`
/// when the desktop cannot say (all but Hyprland). Blocking.
pub fn away(game_pid: u32) -> bool {
    if desktop() != Some(Desktop::Hyprland) {
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

/// Whether the game has put a window on the desktop (console mode's launch
/// screen waits for it); `None` when the desktop cannot say. Blocking.
pub fn shown(game_pid: u32) -> Option<bool> {
    if desktop() != Some(Desktop::Hyprland) {
        return None;
    }
    let clients = hyprctl(&["clients", "-j"]).ok()?;
    Some(!game_windows(&clients, |pid| belongs_to(pid, game_pid)).is_empty())
}

/// Gives the focus to the game's window (the one that had it last, when it
/// has several). `Err` says why not: no window manager that can be asked,
/// or no window of the game. Blocking.
pub fn focus(game_pid: u32) -> Result<(), String> {
    match desktop() {
        Some(Desktop::Hyprland) => focus_hyprland(game_pid),
        Some(Desktop::Sway) => focus_sway(game_pid),
        Some(Desktop::X11) => focus_x11(game_pid),
        Some(Desktop::Windows) => focus_windows(game_pid),
        None => Err("this desktop gives no way to focus the game's window".into()),
    }
}

/// Whether the game's window can be looked for and focused here (Hyprland,
/// Sway, X11, Windows). It may still have none.
pub fn supported() -> bool {
    desktop().is_some()
}

fn focus_hyprland(game_pid: u32) -> Result<(), String> {
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

fn focus_sway(game_pid: u32) -> Result<(), String> {
    let tree = swaymsg(&["-t", "get_tree"])?;
    let Some(id) = sway_window(&tree, |pid| belongs_to(pid, game_pid)) else { return Err("the game has no window".into()) };
    let answer = swaymsg(&[&format!("[con_id={id}]"), "focus"])?;
    sway_answer(&answer)
}

#[cfg(target_os = "linux")]
fn focus_x11(game_pid: u32) -> Result<(), String> {
    x11::focus(|pid| belongs_to(pid, game_pid))
}

#[cfg(not(target_os = "linux"))]
fn focus_x11(_game_pid: u32) -> Result<(), String> {
    Err("X11 is only asked on Linux".into())
}

#[cfg(windows)]
fn focus_windows(game_pid: u32) -> Result<(), String> {
    let procs: Vec<(u32, u32)> = heronite_platform::process::snapshot().into_iter().map(|p| (p.pid, p.parent)).collect();
    heronite_platform::window_focus::focus_window_of(&process_tree(&procs, game_pid)).map_err(|e| e.to_string())
}

/// Windows: the game's window being played (the foreground one when it is
/// the game's, else the one [`focus`] gives the focus to), unless it is
/// minimised. Blocking: with the game in the background, a snapshot of the
/// processes (a few ms).
#[cfg(windows)]
pub fn window(game_pid: u32) -> Option<std::num::NonZeroIsize> {
    // The usual case, the game's own process in front, lists nothing.
    if let Some(front) = heronite_platform::window_focus::foreground_of(&[game_pid]) {
        return Some(front);
    }
    let procs: Vec<(u32, u32)> = heronite_platform::process::snapshot().into_iter().map(|p| (p.pid, p.parent)).collect();
    heronite_platform::window_focus::window_of(&process_tree(&procs, game_pid))
}

#[cfg(not(windows))]
fn focus_windows(_game_pid: u32) -> Result<(), String> {
    Err("Windows is only asked on Windows".into())
}

/// `root` and the processes it started, and theirs (`procs`: pid and
/// parent). A launcher stub's game is its child; a pid reused since does
/// not loop.
#[cfg_attr(not(windows), allow(dead_code))]
fn process_tree(procs: &[(u32, u32)], root: u32) -> Vec<u32> {
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        for &(pid, ppid) in procs {
            if ppid == parent && pid != 0 && !tree.contains(&pid) {
                tree.push(pid);
            }
        }
        i += 1;
    }
    tree
}

/// What `hyprctl` printed (its errors too: it prints them on either stream).
fn hyprctl(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("hyprctl");
    cmd.args(args);
    let out = Run::new().timeout(Duration::from_secs(2)).run(&mut cmd).map_err(|e| format!("hyprctl: {e}"))?;
    let text = out.stdout_text();
    Ok(if text.trim().is_empty() { out.stderr_text() } else { text })
}

/// What `swaymsg` printed (an error as JSON too, on either stream).
fn swaymsg(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("swaymsg");
    cmd.args(args);
    let out = Run::new().timeout(Duration::from_secs(2)).run(&mut cmd).map_err(|e| format!("swaymsg: {e}"))?;
    let text = out.stdout_text();
    Ok(if text.trim().is_empty() { out.stderr_text() } else { text })
}

/// The `con_id` of the game's window in `swaymsg -t get_tree`: the one
/// focused last, found by going down each container's focus order.
fn sway_window(tree: &str, is_game: impl Fn(u32) -> bool) -> Option<i64> {
    fn walk(node: &Value, is_game: &dyn Fn(u32) -> bool) -> Option<i64> {
        let pid = node["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok()).filter(|pid| *pid > 0);
        if pid.is_some_and(is_game) {
            return node["id"].as_i64();
        }
        let focus: Vec<i64> = node["focus"].as_array().map(|f| f.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
        let mut children: Vec<&Value> = ["nodes", "floating_nodes"].iter().filter_map(|k| node[*k].as_array()).flatten().collect();
        // Children missing from the focus order come after the others.
        children.sort_by_key(|c| c["id"].as_i64().and_then(|id| focus.iter().position(|f| *f == id)).unwrap_or(usize::MAX));
        children.into_iter().find_map(|c| walk(c, is_game))
    }
    walk(&serde_json::from_str::<Value>(tree).ok()?, &is_game)
}

/// `swaymsg`'s answer to a command: `[{"success": true}]`, or the error.
fn sway_answer(answer: &str) -> Result<(), String> {
    let first = serde_json::from_str::<Value>(answer).ok().and_then(|v| v.as_array().and_then(|a| a.first().cloned()));
    match first {
        Some(r) if r["success"].as_bool() == Some(true) => Ok(()),
        Some(r) => Err(r["error"].as_str().unwrap_or("swaymsg refused").to_string()),
        None => Err(answer.lines().next().unwrap_or("swaymsg gave no answer").trim().to_string()),
    }
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

/// X11: the window manager's list of windows with their pids, then a
/// request to activate one, as a pager or task bar makes it.
#[cfg(target_os = "linux")]
mod x11 {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, Window};
    use x11rb::rust_connection::RustConnection;

    /// Activates the top window whose pid `is_game`.
    pub(super) fn focus(is_game: impl Fn(u32) -> bool) -> Result<(), String> {
        let (conn, screen) = RustConnection::connect(None).map_err(|e| format!("X11: {e}"))?;
        let root = conn.setup().roots.get(screen).ok_or("X11: no screen")?.root;
        let atom = |name: &[u8]| -> Result<u32, String> {
            Ok(conn.intern_atom(false, name).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom)
        };
        let windows_of = |list: u32| -> Vec<Window> {
            let reply = conn.get_property(false, root, list, AtomEnum::WINDOW, 0, u32::MAX).ok().and_then(|c| c.reply().ok());
            reply.and_then(|r| r.value32().map(Iterator::collect)).unwrap_or_default()
        };
        // Bottom to top; the plain list (oldest first) when the other is missing.
        let mut windows = windows_of(atom(b"_NET_CLIENT_LIST_STACKING")?);
        if windows.is_empty() {
            windows = windows_of(atom(b"_NET_CLIENT_LIST")?);
        }
        let pid_atom = atom(b"_NET_WM_PID")?;
        let pid_of = |w: Window| -> Option<u32> {
            let reply = conn.get_property(false, w, pid_atom, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
            let pid = reply.value32()?.next();
            pid.filter(|pid| *pid > 0)
        };
        let Some(window) = windows.iter().rev().copied().find(|w| pid_of(*w).is_some_and(&is_game)) else {
            return Err("the game has no window".into());
        };
        // Source 2: a pager, which window managers obey (an app's own
        // request may be refused); no timestamp, no window of ours active.
        let active = atom(b"_NET_ACTIVE_WINDOW")?;
        let event = ClientMessageEvent::new(32, window, active, [2u32, 0, 0, 0, 0]);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        conn.send_event(false, root, mask, event).map_err(|e| e.to_string())?.check().map_err(|e| format!("X11: {e}"))?;
        conn.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
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

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }

    #[test]
    fn knows_which_window_manager_to_ask() {
        assert_eq!(desktop_of(env(&[("HYPRLAND_INSTANCE_SIGNATURE", "x"), ("WAYLAND_DISPLAY", "wayland-1")])), Some(Desktop::Hyprland));
        assert_eq!(desktop_of(env(&[("SWAYSOCK", "/run/sway.sock"), ("WAYLAND_DISPLAY", "wayland-1")])), Some(Desktop::Sway));
        assert_eq!(desktop_of(env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")])), Some(Desktop::X11));
        // KDE or GNOME on Wayland: their X11 apps see a DISPLAY, but that is Xwayland.
        assert_eq!(desktop_of(env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")])), None);
        assert_eq!(desktop_of(env(&[("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "wayland")])), None);
        assert_eq!(desktop_of(env(&[])), None);
    }

    const SWAY_TREE: &str = r#"{"id": 1, "type": "root", "focus": [3, 2], "nodes": [
        {"id": 2, "type": "output", "focus": [], "nodes": [
            {"id": 20, "type": "con", "pid": 300, "focus": [], "nodes": []}
        ]},
        {"id": 3, "type": "output", "focus": [31, 30], "nodes": [
            {"id": 30, "type": "con", "pid": 200, "focus": [], "nodes": []}
        ], "floating_nodes": [
            {"id": 31, "type": "floating_con", "pid": 201, "focus": [], "nodes": []},
            {"id": 32, "type": "con", "pid": 100, "focus": [], "nodes": []}
        ]}
    ]}"#;

    #[test]
    fn finds_the_games_sway_window_focused_last() {
        let game = |pid: u32| pid == 200 || pid == 201 || pid == 300;
        assert_eq!(sway_window(SWAY_TREE, game), Some(31), "the floating one comes first in its output's focus order");
        assert_eq!(sway_window(SWAY_TREE, |pid| pid == 300), Some(20));
        assert_eq!(sway_window(SWAY_TREE, |_| false), None);
        assert_eq!(sway_window("not json", game), None);
    }

    #[test]
    fn reads_swaymsgs_answer() {
        assert_eq!(sway_answer(r#"[{"success": true}]"#), Ok(()));
        assert_eq!(sway_answer(r#"[{"success": false, "error": "No matching node."}]"#), Err("No matching node.".into()));
        assert!(sway_answer("").is_err());
    }

    #[test]
    fn the_game_is_its_process_and_those_it_started() {
        // 10 starts 11 (a launcher stub), which starts 12; 20 is another program.
        let procs = [(10, 1), (11, 10), (12, 11), (20, 1), (21, 20)];
        let mut tree = process_tree(&procs, 10);
        tree.sort_unstable();
        assert_eq!(tree, [10, 11, 12]);
        assert_eq!(process_tree(&procs, 99), [99], "an exited game is itself alone");
        // A pid reused as its own grandparent does not loop.
        assert_eq!(process_tree(&[(5, 6), (6, 5)], 5), [5, 6]);
    }
}
