//! Changing a running game's FPS limit (the in-game overlay). Of the
//! limiters of [`crate::fps_limit`], two can do it:
//!
//! - MangoHud watches its config file (inotify) and rebuilds its limiter
//!   from it on every change. It applies `MANGOHUD_CONFIG` over the file on
//!   each reload, so the launch moves the limit out of that variable into a
//!   config file of ours: the user's config followed by the limit, in
//!   `$XDG_RUNTIME_DIR/heronite/mangohud/<app>.conf`. MangoHud reads only
//!   that file then, so the user's config is the one MangoHud would have
//!   picked (a game's own `wine-<exe>.conf` first, see [`user_config`]), and
//!   a change to it while the game runs is copied in ([`follow_user_config`]).
//!   Whether the HUD shows (`no_display`) moves into it too, so the overlay
//!   can hide and show it; hidden, MangoHud still limits the game. Without a
//!   limit of ours the user's own `fps_limit` stays. The Steam Linux
//!   Runtime's container (umu runs Proton in it) has a `$XDG_RUNTIME_DIR` of
//!   its own, so the launch has it mount that folder
//!   (`PRESSURE_VESSEL_FILESYSTEMS_RO`).
//! - gamescope reads the `GAMESCOPE_FPS_LIMIT` atom of its X root whenever
//!   it changes (the Steam Deck's quick menu sets it); `-r` sets the same
//!   limit. Setting the atom needs X, so the overlay does it
//!   (`heronite_overlay::gamescope`).
//!
//! DXVK / VKD3D-Proton read their variables once, at start: a game they
//! limit keeps its limit, but MangoHud's HUD (when it runs) can still be
//! shown and hidden.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::fps_limit;
use crate::plan::Command;
use crate::probe::Probe;

const MANGOHUD_CONFIG: &str = "MANGOHUD_CONFIG";
const MANGOHUD_CONFIGFILE: &str = "MANGOHUD_CONFIGFILE";
/// Read by pressure-vessel: extra paths to mount read-only (`:`-separated).
const CONTAINER_RO: &str = "PRESSURE_VESSEL_FILESYSTEMS_RO";
/// The line our part of MangoHud's config file starts with.
const OURS: &str = "# Heronite: the game's FPS limit, changed by the in-game overlay";

/// One write of MangoHud's file at a time: the overlay's change and the copy
/// of the user's config must not undo each other.
static WRITE: Mutex<()> = Mutex::new(());

/// How a running game's limit can change. Written to `running.json` with the
/// session, so a game the app picks up again keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LiveFps {
    MangoHud {
        /// The config file MangoHud watches.
        file: PathBuf,
        /// The user's config, written first (as it was at launch).
        base: String,
        /// Where the user's config is: read again on every write, so a change
        /// made while the game runs counts. `None`: a session saved before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_file: Option<PathBuf>,
        /// `fps_limit_method=early`.
        smooth: bool,
        /// At launch; `0` = none.
        limit: u32,
        /// The HUD shows at launch (not `no_display`).
        hud: bool,
        /// MangoHud limits the game; `false` when DXVK / VKD3D-Proton does
        /// (only the HUD changes then).
        #[serde(default = "yes")]
        limits: bool,
    },
    Gamescope {
        /// At launch (`-r`); `0` = none.
        limit: u32,
    },
}

impl LiveFps {
    /// The limit the game started with (`0` = none).
    pub fn limit(&self) -> u32 {
        match self {
            LiveFps::MangoHud { limit, .. } | LiveFps::Gamescope { limit } => *limit,
        }
    }

    /// Whether MangoHud's HUD shows at launch; `None` when it cannot be
    /// shown or hidden from here (gamescope).
    pub fn hud(&self) -> Option<bool> {
        match self {
            LiveFps::MangoHud { hud, .. } => Some(*hud),
            LiveFps::Gamescope { .. } => None,
        }
    }

    /// Whether the limit can change while the game runs (not when DXVK /
    /// VKD3D-Proton holds it).
    pub fn changes_limit(&self) -> bool {
        match self {
            LiveFps::MangoHud { limits, .. } => *limits,
            LiveFps::Gamescope { .. } => true,
        }
    }

    pub fn is_gamescope(&self) -> bool {
        matches!(self, LiveFps::Gamescope { .. })
    }

    /// For a game picked up again: the limit and the HUD as the overlay left
    /// them (MangoHud's file has them), not as they were at launch.
    pub fn current(mut self) -> Self {
        if let LiveFps::MangoHud { file, limit, hud, limits, .. } = &mut self {
            let text = std::fs::read_to_string(&*file).unwrap_or_default();
            let Some(at) = text.rfind(OURS) else { return self };
            // No `fps_limit` line of ours: none (the user's config decides).
            if *limits {
                *limit = 0;
            }
            for line in text[at..].lines() {
                match line.split_once('=') {
                    Some(("fps_limit", v)) => *limit = v.parse().unwrap_or(*limit),
                    Some(("no_display", v)) => *hud = v == "0",
                    _ => {}
                }
            }
        }
        self
    }
}

/// Right before a game starts: when gamescope or MangoHud runs it, makes its
/// limit changeable and says how. Writes MangoHud's config file and moves the
/// limit into it; leaves `cmd` alone when it cannot (a `MANGOHUD_CONFIG` of
/// the user's own, an inherited one, a file that cannot be written).
/// `game_program`: the game's own program (see [`user_config`]).
pub fn prepare(cmd: &mut Command, app_name: &str, game_program: Option<&str>, probe: &dyn Probe) -> Option<LiveFps> {
    // DXVK / VKD3D-Proton already limit it: a second limiter would fight theirs
    // (see `fps_limit`), and theirs cannot change. MangoHud's HUD still can.
    let dxvk_limits = crate::env::DXVK_FRAME_RATE.iter().any(|k| cmd.env.get(*k).is_some_and(|v| !v.is_empty() && v != "0"));
    let words = command_words(cmd);
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    // The game command follows `--`; only the wrappers before it count.
    let wrappers = &words[..words.iter().position(|w| *w == "--").unwrap_or(words.len())];
    if let Some(i) = wrappers.iter().position(|w| is_program(w, "gamescope")) {
        // gamescope only offers its limit here, and DXVK holds the limit.
        if dxvk_limits {
            return None;
        }
        let limit = wrappers[i..].windows(2).find(|p| p[0] == "-r").and_then(|p| p[1].parse().ok()).unwrap_or(0);
        return Some(LiveFps::Gamescope { limit });
    }
    if !wrappers.iter().any(|w| is_program(w, "mangohud")) {
        return None;
    }
    // Ours, or none at all (the HUD without a limit): anything else is the
    // user's own and stays as it is.
    let (limit, smooth, hud) = match cmd.env.get(MANGOHUD_CONFIG) {
        Some(c) => fps_limit::read_mangohud_config(c).map(|(l, s, hud)| (l, s, hud == fps_limit::Hud::Shown))?,
        None if probe.env_var(MANGOHUD_CONFIG).is_some() => return None,
        None => (0, false, true),
    };
    let base_file = cmd
        .env
        .get(MANGOHUD_CONFIGFILE)
        .cloned()
        .or_else(|| probe.env_var(MANGOHUD_CONFIGFILE))
        .map(PathBuf::from)
        .unwrap_or_else(|| user_config(game_program, probe));
    let base = std::fs::read_to_string(&base_file).unwrap_or_default();
    // Ours says whether it shows from now on, so the user's choice counts first.
    let hud = hud && !hides_hud(&base);
    let dir = probe.env_var("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).unwrap_or_else(|| "/tmp".into());
    let file = Path::new(&dir).join("heronite/mangohud").join(format!("{}.conf", file_name(app_name)));
    let live = LiveFps::MangoHud { file: file.clone(), base, base_file: Some(base_file), smooth, limit, hud, limits: !dxvk_limits };
    if let Err(e) = set_mangohud(&live, limit, hud) {
        log::warn!("MangoHud config for {app_name}: {e}");
        return None;
    }
    if limit > 0 || cmd.env.contains_key(MANGOHUD_CONFIG) {
        // `no_display` stays in the file: here it would win over it on every reload.
        cmd.env.insert(MANGOHUD_CONFIG.into(), "read_cfg".into());
    }
    cmd.env.insert(MANGOHUD_CONFIGFILE.into(), file.to_string_lossy().into_owned());
    share_with_container(cmd, &file, probe);
    Some(live)
}

/// Sets the limit of a game MangoHud limits (`0` = none of ours: the user's
/// config decides) and whether its HUD shows: it rereads the file a moment
/// later. Gamescope's limit is set by the overlay. Blocking file write.
pub fn set_mangohud(live: &LiveFps, limit: u32, hud: bool) -> Result<(), String> {
    let LiveFps::MangoHud { file, smooth, limits, .. } = live else { return Err("not MangoHud".into()) };
    let _one = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Written in place: MangoHud watches this path for changes.
    let limit = limits.then_some(limit);
    std::fs::write(file, config_text(&base_now(live), limit, *smooth, hud)).map_err(|e| e.to_string())
}

/// While the game runs: copies a change of the user's config (GOverlay saved
/// it) into MangoHud's file, which MangoHud watches; ours stays as the
/// overlay left it. Nothing to do when the user's config is as it was, the
/// file is gone (the game ended) or not one of ours. Blocking file read.
pub fn follow_user_config(live: &LiveFps) {
    let LiveFps::MangoHud { file, base_file: Some(_), .. } = live else { return };
    let _one = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(text) = std::fs::read_to_string(file) else { return };
    let Some(at) = text.rfind(OURS) else { return };
    let now = with_ours(&base_now(live), &text[at..]);
    if now != text {
        if let Err(e) = std::fs::write(file, now) {
            log::warn!("MangoHud config {}: {e}", file.display());
        }
    }
}

/// The user's config as it is now (as it was at launch for a session saved
/// before Heronite kept its path).
fn base_now(live: &LiveFps) -> String {
    match live {
        LiveFps::MangoHud { base_file: Some(f), .. } => std::fs::read_to_string(f).unwrap_or_default(),
        LiveFps::MangoHud { base, .. } => base.clone(),
        LiveFps::Gamescope { .. } => String::new(),
    }
}

/// The config file MangoHud would read for the game without ours (with
/// `MANGOHUD_CONFIGFILE` it reads no other): the first that exists of its
/// own search, a game's file before the shared ones. `wine-<exe>.conf` for a
/// Windows program; the program folder's `MangoHud.conf`, then
/// `<program>.conf` for a Linux one; then `MangoHud.conf`, the user's before
/// the system's (`/etc`, which MangoHud reads first, as Heronite did before).
/// None: the user's `MangoHud.conf`, so one saved while the game runs counts.
/// Picked once, at launch. `game_program` is what the plan starts: for a
/// script (`start.sh`) MangoHud's own names are the binary's, so the shared
/// files count then.
fn user_config(game_program: Option<&str>, probe: &dyn Probe) -> PathBuf {
    let config = probe.env_var("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).unwrap_or_else(|| format!("{}/.config", probe.home()));
    let dir = Path::new(&config).join("MangoHud");
    let shared = dir.join("MangoHud.conf");
    let mut paths = Vec::new();
    if let Some(program) = game_program.filter(|p| !p.is_empty()) {
        let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
        if name.to_ascii_lowercase().ends_with(".exe") {
            // MangoHud names a Wine program by its `.exe` without the extension.
            paths.push(dir.join(format!("wine-{}.conf", &name[..name.len() - 4])));
        } else if !name.is_empty() {
            if let Some(folder) = Path::new(program).parent().filter(|f| !f.as_os_str().is_empty()) {
                paths.push(folder.join("MangoHud.conf"));
            }
            paths.push(dir.join(format!("{name}.conf")));
        }
    }
    paths.push(shared.clone());
    paths.push(PathBuf::from("/etc/MangoHud.conf"));
    paths.into_iter().find(|p| p.is_file()).unwrap_or(shared)
}

/// The program and its arguments, with the wrappers legendary starts the game
/// through (`--wrapper '<wrappers>'`, one argument) as words of their own.
fn command_words(cmd: &Command) -> Vec<String> {
    let mut words = vec![cmd.program.clone()];
    let mut args = cmd.args.iter();
    let mut game = false;
    while let Some(a) = args.next() {
        words.push(a.clone());
        game |= a == "--";
        if a == "--wrapper" && !game {
            if let Some(w) = args.next() {
                words.extend(shlex::split(w).unwrap_or_else(|| vec![w.clone()]));
            }
        }
    }
    words
}

/// A game in the Steam Linux Runtime's container (umu's Proton) sees a
/// `$XDG_RUNTIME_DIR` of its own, without our file: MangoHud would start with
/// its defaults and the overlay could not show or hide it. The container
/// mounts the folder (read-only: MangoHud only reads it), so each rewrite
/// reaches it. Only pressure-vessel reads the variable.
fn share_with_container(cmd: &mut Command, file: &Path, probe: &dyn Probe) {
    let Some(dir) = file.parent().map(|d| d.to_string_lossy().into_owned()) else { return };
    let existing = [cmd.env.get(CONTAINER_RO).cloned().or_else(|| probe.env_var(CONTAINER_RO))];
    if let Some(v) = crate::shared_folders::container_env(&[dir], &existing) {
        cmd.env.insert(CONTAINER_RO.into(), v);
    }
}

fn yes() -> bool {
    true
}

/// The user's config, then the limit (`None`: DXVK holds it, MangoHud's is
/// left as the user's config says) and the HUD (a later line wins in MangoHud).
fn config_text(base: &str, limit: Option<u32>, smooth: bool, hud: bool) -> String {
    let mut ours = format!("{OURS}\n");
    // No limit of ours: the user's `fps_limit` (and method) stay.
    if let Some(limit) = limit.filter(|l| *l > 0) {
        ours.push_str(&format!("fps_limit={limit}\n"));
        if smooth {
            ours.push_str("fps_limit_method=early\n");
        }
    }
    ours.push_str(&format!("no_display={}\n", u8::from(!hud)));
    with_ours(base, &ours)
}

/// The user's config, then our part (a later line wins in MangoHud).
fn with_ours(base: &str, ours: &str) -> String {
    let mut text = base.trim_end().to_string();
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(ours);
    text
}

/// Whether a MangoHud config hides the HUD: its last `no_display` line, bare
/// or not `0`.
fn hides_hud(config: &str) -> bool {
    config
        .lines()
        .rev()
        .find_map(|l| {
            let (key, value) = l.split_once('=').map_or((l, None), |(k, v)| (k, Some(v)));
            (key.trim() == "no_display").then(|| value.is_none_or(|v| v.trim() != "0"))
        })
        .unwrap_or(false)
}

fn is_program(word: &str, name: &str) -> bool {
    Path::new(word).file_name().is_some_and(|f| f == name)
}

fn file_name(app_name: &str) -> String {
    app_name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;

    fn cmd(program: &str, args: &[&str], env: &[(&str, &str)]) -> Command {
        Command {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            cwd: None,
        }
    }

    fn probe(dir: &Path) -> FakeProbe {
        FakeProbe::new(&dir.join("home").to_string_lossy()).with_env("XDG_RUNTIME_DIR", &dir.join("run").to_string_lossy())
    }

    // MangoHud: Linux only.
    #[cfg(unix)]
    #[test]
    fn mangohud_limit_moves_into_a_file_with_the_users_config() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let user = tmp.path().join("home/.config/MangoHud");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("MangoHud.conf"), "fps\ncpu_temp\nfps_limit=144\n").unwrap();
        let mut c = cmd(
            "/usr/bin/mangohud",
            &["--dlsym", "/games/run.sh"],
            &[("MANGOHUD_CONFIG", "read_cfg,no_display,fps_limit=60,fps_limit_method=early")],
        );
        let live = prepare(&mut c, "steam_400/x", None, &p).unwrap();
        assert_eq!(live.limit(), 60);
        assert_eq!(c.env["MANGOHUD_CONFIG"], "read_cfg", "the limit and no_display leave the variable");
        assert_eq!(live.hud(), Some(false));
        let file = tmp.path().join("run/heronite/mangohud/steam_400_x.conf");
        assert_eq!(c.env["MANGOHUD_CONFIGFILE"], file.to_string_lossy());
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("fps\ncpu_temp\nfps_limit=144\n"), "the user's look stays");
        assert!(text.ends_with("fps_limit=60\nfps_limit_method=early\nno_display=1\n"), "ours comes last and wins");

        set_mangohud(&live, 30, false).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().ends_with("fps_limit=30\nfps_limit_method=early\nno_display=1\n"));
        set_mangohud(&live, 0, false).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.ends_with(&format!("fps_limit=144\n{OURS}\nno_display=1\n")), "0 = no limit of ours: the user's counts");
        set_mangohud(&live, 30, true).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.ends_with("fps_limit=30\nfps_limit_method=early\nno_display=0\n"), "shown again, same limit");
    }

    // Unix paths: `C:` would split the `:`-separated list.
    #[cfg(unix)]
    #[test]
    fn the_steam_runtime_container_sees_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("run/heronite/mangohud").to_string_lossy().into_owned();
        let umu = ["--dlsym", "/usr/bin/gamemoderun", "/tools/umu/umu_run.py", "/games/game.exe"];
        let mut c = cmd("/usr/bin/mangohud", &umu, &[("MANGOHUD_CONFIG", "read_cfg,no_display=1")]);
        prepare(&mut c, "g", None, &probe(tmp.path())).unwrap();
        assert_eq!(c.env["PRESSURE_VESSEL_FILESYSTEMS_RO"], dir);
        // The user's own folders stay first, ours is not added twice.
        let mut c = cmd("/usr/bin/mangohud", &umu, &[("PRESSURE_VESSEL_FILESYSTEMS_RO", &format!("/srv:{dir}"))]);
        prepare(&mut c, "g", None, &probe(tmp.path())).unwrap();
        assert_eq!(c.env["PRESSURE_VESSEL_FILESYSTEMS_RO"], format!("/srv:{dir}"));
        let inherited = probe(tmp.path()).with_env("PRESSURE_VESSEL_FILESYSTEMS_RO", "/data");
        let mut c = cmd("/usr/bin/mangohud", &umu, &[]);
        prepare(&mut c, "g", None, &inherited).unwrap();
        assert_eq!(c.env["PRESSURE_VESSEL_FILESYSTEMS_RO"], format!("/data:{dir}"));
    }

    #[test]
    fn a_game_picked_up_again_has_what_the_overlay_left() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "read_cfg,fps_limit=60")]);
        let live = prepare(&mut c, "g", None, &probe(tmp.path())).unwrap();
        set_mangohud(&live, 90, false).unwrap();
        // As `running.json` keeps it.
        let saved: LiveFps = serde_json::from_value(serde_json::to_value(&live).unwrap()).unwrap();
        assert_eq!(saved, live);
        let now = saved.current();
        assert_eq!((now.limit(), now.hud()), (90, Some(false)));
        assert_eq!(LiveFps::Gamescope { limit: 50 }.current(), LiveFps::Gamescope { limit: 50 });
    }

    #[test]
    fn mangohud_started_hidden_can_be_shown_from_the_overlay() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "read_cfg,no_display=1")]);
        let live = prepare(&mut c, "g", None, &probe(tmp.path())).unwrap();
        assert_eq!((live.limit(), live.hud()), (0, Some(false)));
        assert_eq!(c.env["MANGOHUD_CONFIG"], "read_cfg", "no_display moves into the file");
        let LiveFps::MangoHud { file, .. } = &live else { unreachable!() };
        assert_eq!(std::fs::read_to_string(file).unwrap(), format!("{OURS}\nno_display=1\n"));
    }

    #[test]
    fn the_hud_starts_as_the_user_left_it() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let user = tmp.path().join("home/.config/MangoHud");
        std::fs::create_dir_all(&user).unwrap();
        let shown = |conf: &str| {
            std::fs::write(user.join("MangoHud.conf"), conf).unwrap();
            let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "read_cfg,fps_limit=60")]);
            prepare(&mut c, "g", None, &p).unwrap().hud().unwrap()
        };
        assert!(shown("fps\n"));
        assert!(!shown("fps\nno_display\n"), "hidden in the user's config");
        assert!(!shown("no_display = 1\n"));
        assert!(shown("no_display\nno_display=0\n"), "the last line wins");
    }

    #[test]
    fn hud_without_a_limit_can_get_one() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["--dlsym", "/games/run.sh"], &[]);
        let live = prepare(&mut c, "g", None, &probe(tmp.path())).unwrap();
        assert_eq!(live.limit(), 0);
        assert_eq!(live.hud(), Some(true));
        assert!(!c.env.contains_key("MANGOHUD_CONFIG"), "MangoHud reads the file alone");
        assert!(c.env.contains_key("MANGOHUD_CONFIGFILE"));
    }

    #[test]
    fn without_a_limit_of_ours_the_users_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let user = tmp.path().join("home/.config/MangoHud");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("MangoHud.conf"), "fps\nfps_limit=144\n").unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["--dlsym", "/games/run.sh"], &[]);
        let live = prepare(&mut c, "g", None, &p).unwrap();
        let LiveFps::MangoHud { file, .. } = &live else { unreachable!() };
        assert_eq!(std::fs::read_to_string(file).unwrap(), format!("fps\nfps_limit=144\n{OURS}\nno_display=0\n"));
        // The overlay sets one, then none again: the user's comes back.
        set_mangohud(&live, 60, true).unwrap();
        assert_eq!(live.clone().current().limit(), 60);
        set_mangohud(&live, 0, true).unwrap();
        assert_eq!(std::fs::read_to_string(file).unwrap(), format!("fps\nfps_limit=144\n{OURS}\nno_display=0\n"));
        assert_eq!(live.current().limit(), 0, "picked up again: none of ours");
    }

    #[test]
    fn the_config_is_the_one_mangohud_would_read() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let user = tmp.path().join("home/.config/MangoHud");
        std::fs::create_dir_all(&user).unwrap();
        let base = |program: Option<&str>| {
            let mut c = cmd("/usr/bin/mangohud", &["--dlsym", "/games/run.sh"], &[]);
            let live = prepare(&mut c, "g", program, &p).unwrap();
            let LiveFps::MangoHud { base, base_file, .. } = live else { unreachable!() };
            (base, base_file.unwrap())
        };
        // The system's config, when this machine has one, comes before none.
        if !Path::new("/etc/MangoHud.conf").exists() {
            assert_eq!(base(None), (String::new(), user.join("MangoHud.conf")), "none yet: the user's, once saved");
        }
        std::fs::write(user.join("MangoHud.conf"), "shared\n").unwrap();
        std::fs::write(user.join("wine-Control.conf"), "control\n").unwrap();
        assert_eq!(base(Some(r"Binaries\Win64\Control.EXE")).0, "control\n", "a Windows game's own, named as MangoHud does");
        assert_eq!(base(Some("/games/Other.exe")).0, "shared\n");
        // A Linux game: its folder's file, then its own in the user's folder.
        let game = tmp.path().join("games/native");
        std::fs::create_dir_all(&game).unwrap();
        let program = game.join("game.x86_64").to_string_lossy().into_owned();
        assert_eq!(base(Some(&program)).0, "shared\n");
        std::fs::write(user.join("game.x86_64.conf"), "own\n").unwrap();
        assert_eq!(base(Some(&program)).0, "own\n");
        std::fs::write(game.join("MangoHud.conf"), "folder\n").unwrap();
        assert_eq!(base(Some(&program)).0, "folder\n");
        // The user's MANGOHUD_CONFIGFILE is the only one MangoHud reads.
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIGFILE", &program)]);
        let LiveFps::MangoHud { base_file, .. } = prepare(&mut c, "g", Some("/games/Control.exe"), &p).unwrap() else { unreachable!() };
        assert_eq!(base_file.unwrap(), Path::new(&program));
    }

    #[test]
    fn a_change_to_the_users_config_reaches_the_running_game() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let user = tmp.path().join("home/.config/MangoHud/MangoHud.conf");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        std::fs::write(&user, "fps\n").unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "read_cfg,fps_limit=60")]);
        let live = prepare(&mut c, "g", None, &p).unwrap();
        let LiveFps::MangoHud { file, .. } = &live else { unreachable!() };
        set_mangohud(&live, 90, false).unwrap();
        let ours = format!("{OURS}\nfps_limit=90\nno_display=1\n");
        // GOverlay saves while the game runs.
        std::fs::write(&user, "fps\ncpu_temp\n").unwrap();
        follow_user_config(&live);
        assert_eq!(std::fs::read_to_string(file).unwrap(), format!("fps\ncpu_temp\n{ours}"), "the overlay's choice stays");
        let modified = std::fs::metadata(file).unwrap().modified().unwrap();
        follow_user_config(&live);
        assert_eq!(std::fs::metadata(file).unwrap().modified().unwrap(), modified, "unchanged: not written again");
        // The overlay's next change keeps it too.
        set_mangohud(&live, 30, true).unwrap();
        assert!(std::fs::read_to_string(file).unwrap().starts_with("fps\ncpu_temp\n"));
        // The game ended: nothing comes back.
        std::fs::remove_file(file).unwrap();
        follow_user_config(&live);
        assert!(!file.exists());
    }

    #[test]
    fn legendarys_wrapper_counts_its_gamescope() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/lg/legendary", &["launch", "App", "--no-wine", "--wrapper", "/usr/bin/gamescope -r 40 -- /umu/umu_run.py"], &[]);
        assert_eq!(prepare(&mut c, "g", None, &probe(tmp.path())), Some(LiveFps::Gamescope { limit: 40 }));
        let mut c = cmd("/games/run.sh", &["--", "--wrapper", "/usr/bin/mangohud"], &[]);
        assert_eq!(prepare(&mut c, "g", None, &probe(tmp.path())), None, "a game argument");
    }

    #[test]
    fn the_users_own_mangohud_config_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "fps,frametime,fps_limit=45")]);
        let before = c.clone();
        assert_eq!(prepare(&mut c, "g", None, &probe(tmp.path())), None);
        assert_eq!(c, before);
        let inherited = probe(tmp.path()).with_env("MANGOHUD_CONFIG", "fps");
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[]);
        assert_eq!(prepare(&mut c, "g", None, &inherited), None);
    }

    #[test]
    fn dxvk_keeps_its_limit_and_the_hud_still_shows_and_hides() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let user = tmp.path().join("home/.config/MangoHud");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("MangoHud.conf"), "fps\nfps_limit=0\n").unwrap();
        let dxvk = [("DXVK_FRAME_RATE", "60"), ("VKD3D_FRAME_RATE", "60")];
        let mut c = cmd("/usr/bin/mangohud", &["--dlsym", "/games/run.sh"], &dxvk);
        let live = prepare(&mut c, "g", None, &p).expect("the HUD can still change");
        assert!(!live.changes_limit(), "DXVK limits it: no second limiter");
        assert_eq!((live.limit(), live.hud()), (0, Some(true)));
        assert_eq!(c.env["DXVK_FRAME_RATE"], "60", "DXVK's limit stays");
        assert!(!c.env.contains_key("MANGOHUD_CONFIG"));
        let LiveFps::MangoHud { file, .. } = &live else { unreachable!() };
        assert_eq!(c.env["MANGOHUD_CONFIGFILE"], file.to_string_lossy());
        assert_eq!(std::fs::read_to_string(file).unwrap(), format!("fps\nfps_limit=0\n{OURS}\nno_display=0\n"), "no limit of ours");
        set_mangohud(&live, 0, false).unwrap();
        assert!(std::fs::read_to_string(file).unwrap().ends_with(&format!("{OURS}\nno_display=1\n")));
        // Picked up again from `running.json`.
        let saved: LiveFps = serde_json::from_value(serde_json::to_value(&live).unwrap()).unwrap();
        assert_eq!(saved.current().hud(), Some(false));

        let mut c =
            cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("DXVK_FRAME_RATE", "60"), ("MANGOHUD_CONFIG", "read_cfg,no_display=1")]);
        let live = prepare(&mut c, "g", None, &p).unwrap();
        assert_eq!((live.hud(), live.changes_limit()), (Some(false), false), "started hidden");
        let mut c = cmd("/games/run.sh", &[], &dxvk);
        assert_eq!(prepare(&mut c, "g", None, &p), None, "no MangoHud");
    }

    #[test]
    fn a_session_saved_before_dxvk_kept_the_hud_still_limits() {
        let old = serde_json::json!({"kind": "mangoHud", "file": "/run/x.conf", "base": "", "smooth": false, "limit": 60, "hud": true});
        let live: LiveFps = serde_json::from_value(old).unwrap();
        assert!(live.changes_limit());
    }

    #[test]
    fn gamescope_and_plain_launches() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let mut c = cmd("/usr/bin/gamescope", &["-f", "-r", "50", "--mangoapp", "--", "/games/run.sh"], &[]);
        assert_eq!(prepare(&mut c, "g", None, &p), Some(LiveFps::Gamescope { limit: 50 }));
        let mut c = cmd("/usr/bin/gamescope", &["--", "/games/run.sh"], &[]);
        assert_eq!(prepare(&mut c, "g", None, &p), Some(LiveFps::Gamescope { limit: 0 }));
        let mut c = cmd("/games/run.sh", &["--", "/x/mangohud"], &[]);
        assert_eq!(prepare(&mut c, "g", None, &p), None, "a game argument named mangohud");
        let mut c = cmd("/usr/bin/gamescope", &["--", "/games/run.sh"], &[("DXVK_FRAME_RATE", "60")]);
        assert_eq!(prepare(&mut c, "g", None, &p), None);
    }
}
