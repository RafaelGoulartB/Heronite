//! The FPS limit of a game running in gamescope, changed while it runs:
//! gamescope reads the `GAMESCOPE_FPS_LIMIT` atom of its X root whenever it
//! changes (what the Steam Deck's quick menu sets; `-r` sets the same limit).
//!
//! The atom is set with `xprop` on gamescope's own Xwayland, with the display
//! and X authority the game's processes got from gamescope (they carry
//! `GAMESCOPE_WAYLAND_DISPLAY`).

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;

use crate::procs::{env_of, game_processes};

/// Whether `xprop` can be found (without it the limit cannot change).
pub fn available() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join("xprop").is_file()))
}

/// Sets the limit (`0` = none) of the game whose process (group leader) is
/// `game_pid`. Blocking.
pub fn set_fps_limit(game_pid: u32, limit: u32) -> Result<(), String> {
    let (display, xauthority) = game_display(game_pid).ok_or_else(|| "gamescope's display was not found".to_string())?;
    let mut cmd = Command::new("xprop");
    cmd.args(["-root", "-f", "GAMESCOPE_FPS_LIMIT", "32c", "-set", "GAMESCOPE_FPS_LIMIT", &limit.to_string()]);
    cmd.env("DISPLAY", &display);
    match xauthority {
        Some(a) => cmd.env("XAUTHORITY", a),
        None => cmd.env_remove("XAUTHORITY"),
    };
    let out = Run::new().timeout(Duration::from_secs(5)).run(&mut cmd).map_err(|e| format!("xprop: {e}"))?;
    if out.success() {
        log::info!("gamescope FPS limit {limit} on {display}");
        Ok(())
    } else {
        Err(out.stderr_text().trim().to_string())
    }
}

/// `DISPLAY` (and `XAUTHORITY`) of a process gamescope started for the game.
fn game_display(game_pid: u32) -> Option<(String, Option<String>)> {
    game_processes(game_pid).into_iter().find_map(|pid| {
        env_of(pid, "GAMESCOPE_WAYLAND_DISPLAY")?;
        Some((env_of(pid, "DISPLAY")?, env_of(pid, "XAUTHORITY")))
    })
}
