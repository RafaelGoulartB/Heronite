//! Changing a running game's FPS limit (the in-game overlay). Of the
//! limiters of [`crate::fps_limit`], two can do it:
//!
//! - MangoHud watches its config file (inotify) and rebuilds its limiter
//!   from it on every change. It applies `MANGOHUD_CONFIG` over the file on
//!   each reload, so the launch moves the limit out of that variable into a
//!   config file of ours: the user's `MangoHud.conf` (or their
//!   `MANGOHUD_CONFIGFILE`) followed by the limit, in
//!   `$XDG_RUNTIME_DIR/heronite/mangohud/<app>.conf`. Whether the HUD shows
//!   (`no_display`) moves into it too, so the overlay can hide and show it;
//!   hidden, MangoHud still limits the game.
//! - gamescope reads the `GAMESCOPE_FPS_LIMIT` atom of its X root whenever
//!   it changes (the Steam Deck's quick menu sets it); `-r` sets the same
//!   limit. Setting the atom needs X, so the overlay does it
//!   (`heronite_overlay::gamescope`).
//!
//! DXVK / VKD3D-Proton read their variables once, at start.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fps_limit;
use crate::plan::Command;
use crate::probe::Probe;

const MANGOHUD_CONFIG: &str = "MANGOHUD_CONFIG";
const MANGOHUD_CONFIGFILE: &str = "MANGOHUD_CONFIGFILE";
/// The line our part of MangoHud's config file starts with.
const OURS: &str = "# Heronite: the game's FPS limit, changed by the in-game overlay";

/// How a running game's limit can change. Written to `running.json` with the
/// session, so a game the app picks up again keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LiveFps {
    MangoHud {
        /// The config file MangoHud watches.
        file: PathBuf,
        /// The user's config, written first.
        base: String,
        /// `fps_limit_method=early`.
        smooth: bool,
        /// At launch; `0` = none.
        limit: u32,
        /// The HUD shows at launch (not `no_display`).
        hud: bool,
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

    pub fn is_gamescope(&self) -> bool {
        matches!(self, LiveFps::Gamescope { .. })
    }

    /// For a game picked up again: the limit and the HUD as the overlay left
    /// them (MangoHud's file has them), not as they were at launch.
    pub fn current(mut self) -> Self {
        if let LiveFps::MangoHud { file, limit, hud, .. } = &mut self {
            let text = std::fs::read_to_string(&*file).unwrap_or_default();
            let ours = text.rfind(OURS).map_or("", |at| &text[at..]);
            for line in ours.lines() {
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
pub fn prepare(cmd: &mut Command, app_name: &str, probe: &dyn Probe) -> Option<LiveFps> {
    // DXVK / VKD3D-Proton already limit it: a second limiter would fight theirs
    // (see `fps_limit`), and theirs cannot change.
    if crate::env::DXVK_FRAME_RATE.iter().any(|k| cmd.env.get(*k).is_some_and(|v| !v.is_empty() && v != "0")) {
        return None;
    }
    let words: Vec<&str> = std::iter::once(cmd.program.as_str()).chain(cmd.args.iter().map(String::as_str)).collect();
    // The game command follows `--`; only the wrappers before it count.
    let wrappers = &words[..words.iter().position(|w| *w == "--").unwrap_or(words.len())];
    if let Some(i) = wrappers.iter().position(|w| is_program(w, "gamescope")) {
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
    let base_file =
        cmd.env.get(MANGOHUD_CONFIGFILE).cloned().or_else(|| probe.env_var(MANGOHUD_CONFIGFILE)).map(PathBuf::from).unwrap_or_else(|| {
            let config = probe.env_var("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).unwrap_or_else(|| format!("{}/.config", probe.home()));
            Path::new(&config).join("MangoHud/MangoHud.conf")
        });
    let base = std::fs::read_to_string(&base_file).unwrap_or_default();
    // Ours says whether it shows from now on, so the user's choice counts first.
    let hud = hud && !hides_hud(&base);
    let dir = probe.env_var("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).unwrap_or_else(|| "/tmp".into());
    let file = Path::new(&dir).join("heronite/mangohud").join(format!("{}.conf", file_name(app_name)));
    let live = LiveFps::MangoHud { file: file.clone(), base, smooth, limit, hud };
    if let Err(e) = set_mangohud(&live, limit, hud) {
        log::warn!("MangoHud config for {app_name}: {e}");
        return None;
    }
    if limit > 0 || cmd.env.contains_key(MANGOHUD_CONFIG) {
        // `no_display` stays in the file: here it would win over it on every reload.
        cmd.env.insert(MANGOHUD_CONFIG.into(), "read_cfg".into());
    }
    cmd.env.insert(MANGOHUD_CONFIGFILE.into(), file.to_string_lossy().into_owned());
    Some(live)
}

/// Sets the limit of a game MangoHud limits (`0` = none) and whether its HUD
/// shows: it rereads the file a moment later. Gamescope's limit is set by the
/// overlay. Blocking file write.
pub fn set_mangohud(live: &LiveFps, limit: u32, hud: bool) -> Result<(), String> {
    let LiveFps::MangoHud { file, base, smooth, .. } = live else { return Err("not MangoHud".into()) };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Written in place: MangoHud watches this path for changes.
    std::fs::write(file, config_text(base, limit, *smooth, hud)).map_err(|e| e.to_string())
}

/// The user's config, then the limit and the HUD (a later line wins in MangoHud).
fn config_text(base: &str, limit: u32, smooth: bool, hud: bool) -> String {
    let mut text = base.trim_end().to_string();
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(OURS);
    text.push('\n');
    text.push_str(&format!("fps_limit={limit}\n"));
    if smooth {
        text.push_str("fps_limit_method=early\n");
    }
    text.push_str(&format!("no_display={}\n", u8::from(!hud)));
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
        let live = prepare(&mut c, "steam_400/x", &p).unwrap();
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
        assert!(std::fs::read_to_string(&file).unwrap().contains("fps_limit=0\n"), "0 = no limit");
        set_mangohud(&live, 30, true).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.ends_with("fps_limit=30\nfps_limit_method=early\nno_display=0\n"), "shown again, same limit");
    }

    #[test]
    fn a_game_picked_up_again_has_what_the_overlay_left() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "read_cfg,fps_limit=60")]);
        let live = prepare(&mut c, "g", &probe(tmp.path())).unwrap();
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
        let live = prepare(&mut c, "g", &probe(tmp.path())).unwrap();
        assert_eq!((live.limit(), live.hud()), (0, Some(false)));
        assert_eq!(c.env["MANGOHUD_CONFIG"], "read_cfg", "no_display moves into the file");
        let LiveFps::MangoHud { file, .. } = &live else { unreachable!() };
        assert!(std::fs::read_to_string(file).unwrap().ends_with("fps_limit=0\nno_display=1\n"));
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
            prepare(&mut c, "g", &p).unwrap().hud().unwrap()
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
        let live = prepare(&mut c, "g", &probe(tmp.path())).unwrap();
        assert_eq!(live.limit(), 0);
        assert_eq!(live.hud(), Some(true));
        assert!(!c.env.contains_key("MANGOHUD_CONFIG"), "MangoHud reads the file alone");
        assert!(c.env.contains_key("MANGOHUD_CONFIGFILE"));
    }

    #[test]
    fn the_users_own_mangohud_config_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[("MANGOHUD_CONFIG", "fps,frametime,fps_limit=45")]);
        let before = c.clone();
        assert_eq!(prepare(&mut c, "g", &probe(tmp.path())), None);
        assert_eq!(c, before);
        let inherited = probe(tmp.path()).with_env("MANGOHUD_CONFIG", "fps");
        let mut c = cmd("/usr/bin/mangohud", &["/games/run.sh"], &[]);
        assert_eq!(prepare(&mut c, "g", &inherited), None);
    }

    #[test]
    fn gamescope_and_plain_launches() {
        let tmp = tempfile::tempdir().unwrap();
        let p = probe(tmp.path());
        let mut c = cmd("/usr/bin/gamescope", &["-f", "-r", "50", "--mangoapp", "--", "/games/run.sh"], &[]);
        assert_eq!(prepare(&mut c, "g", &p), Some(LiveFps::Gamescope { limit: 50 }));
        let mut c = cmd("/usr/bin/gamescope", &["--", "/games/run.sh"], &[]);
        assert_eq!(prepare(&mut c, "g", &p), Some(LiveFps::Gamescope { limit: 0 }));
        let mut c = cmd("/games/run.sh", &["--", "/x/mangohud"], &[]);
        assert_eq!(prepare(&mut c, "g", &p), None, "a game argument named mangohud");
        let mut c = cmd("/usr/bin/mangohud", &["--dlsym", "/games/run.sh"], &[("DXVK_FRAME_RATE", "60"), ("VKD3D_FRAME_RATE", "60")]);
        assert_eq!(prepare(&mut c, "g", &p), None, "DXVK limits it: no second limiter");
        let mut c = cmd("/usr/bin/gamescope", &["--", "/games/run.sh"], &[("DXVK_FRAME_RATE", "60")]);
        assert_eq!(prepare(&mut c, "g", &p), None);
    }
}
