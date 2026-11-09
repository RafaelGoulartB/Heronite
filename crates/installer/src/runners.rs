//! How an installed game starts, from the script's runner and `game`
//! section. Every runner becomes a local game of the Collection:
//!
//! * `wine`: a Windows program run with the chosen Wine/Proton in the
//!   script's prefix;
//! * `linux`: a native program;
//! * `dosbox`, `scummvm`, `libretro`: a command run in the game's folder
//!   with the emulator installed on this computer;
//! * `web`: a browser game.

use std::path::Path;

use heronite_i18n::tr;
use serde_json::{Map, Value};

use crate::fsops::fix_case;
use crate::text::{get_nonempty, truthy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    Wine,
    Linux,
    Dosbox,
    Scummvm,
    Libretro,
    Web,
}

const DOSBOX: &[&str] = &["dosbox", "dosbox-staging", "dosbox-x"];

/// The runner of a script; `Err` explains why it cannot be installed here.
pub fn runner(name: &str) -> Result<Runner, String> {
    Ok(match name {
        "wine" => Runner::Wine,
        "linux" => Runner::Linux,
        "dosbox" => Runner::Dosbox,
        "scummvm" => Runner::Scummvm,
        "libretro" => Runner::Libretro,
        "web" | "browser" => Runner::Web,
        "steam" | "winesteam" => return Err(tr!("Steam games are installed through Steam").into()),
        other => return Err(tr!("Games that run with {runner} are not supported", runner = other)),
    })
}

impl Runner {
    /// Programs the installed game needs on this computer (one of each group).
    pub fn tools(self) -> &'static [&'static [&'static str]] {
        match self {
            Runner::Dosbox => &[DOSBOX],
            Runner::Scummvm => &[&["scummvm"]],
            Runner::Libretro => &[&["retroarch"]],
            Runner::Wine | Runner::Linux | Runner::Web => &[],
        }
    }
}

/// How the game starts, with every path resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Windows {
        exe: String,
        args: String,
        prefix: String,
    },
    Linux {
        exe: String,
        args: String,
    },
    /// A shell command run in `folder`.
    Command {
        folder: String,
        command: String,
    },
    Browser {
        url: String,
    },
}

/// Another way to start the game (`launch_configs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraLaunch {
    pub name: String,
    pub exe: String,
    pub args: String,
}

/// A path of the `game` section: variables substituted, relative to the
/// game's folder, case fixed.
fn path_of(raw: &str, sub: &dyn Fn(&str) -> String, game_dir: &Path) -> String {
    let s = sub(raw.trim());
    let p = Path::new(&s);
    let full = if p.is_absolute() { p.to_path_buf() } else { game_dir.join(p) };
    fix_case(&full).to_string_lossy().into_owned()
}

fn quote(s: &str) -> String {
    shlex::try_quote(s).map(|q| q.into_owned()).unwrap_or_else(|_| format!("'{s}'"))
}

/// The prefix of the game (`game.prefix`, else the game's folder).
pub fn prefix_of(game: &Map<String, Value>, sub: &dyn Fn(&str) -> String, game_dir: &Path) -> String {
    match get_nonempty(game, "prefix") {
        Some(p) => path_of(&p, sub, game_dir),
        None => game_dir.to_string_lossy().into_owned(),
    }
}

pub fn resolve(runner: Runner, game: &Map<String, Value>, sub: &dyn Fn(&str) -> String, game_dir: &Path) -> Result<Launch, String> {
    let path = |key: &str| get_nonempty(game, key).map(|v| path_of(&v, sub, game_dir));
    let args = get_nonempty(game, "args").map(|a| sub(&a)).unwrap_or_default();
    let folder = |fallback: &str| {
        path("working_dir").unwrap_or_else(|| Path::new(fallback).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
    };
    let missing = || tr!("The script does not say which file starts the game").to_string();
    match runner {
        Runner::Wine => {
            let exe = path("exe").ok_or_else(missing)?;
            Ok(Launch::Windows { exe, args, prefix: prefix_of(game, sub, game_dir) })
        }
        Runner::Linux => {
            let exe = path("exe").ok_or_else(missing)?;
            let dir = folder(&exe);
            let exe_dir = Path::new(&exe).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            if dir != exe_dir && !dir.is_empty() {
                let command = [quote(&exe), args].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                return Ok(Launch::Command { folder: dir, command });
            }
            Ok(Launch::Linux { exe, args })
        }
        Runner::Dosbox => {
            let main = path("main_file").ok_or_else(missing)?;
            let bin = crate::exec::which_any(DOSBOX).map(|b| crate::steps::short(&b)).unwrap_or_else(|| "dosbox".into());
            let mut parts = vec![bin];
            if main.to_ascii_lowercase().ends_with(".conf") {
                parts.extend(["-conf".into(), quote(&main)]);
            } else {
                parts.push(quote(&main));
                if truthy(game.get("exit")).unwrap_or(true) {
                    parts.push("-exit".into());
                }
            }
            if let Some(conf) = path("config_file") {
                parts.extend(["-conf".into(), quote(&conf)]);
            }
            if !args.is_empty() {
                parts.push(args);
            }
            Ok(Launch::Command { folder: folder(&main), command: parts.join(" ") })
        }
        Runner::Scummvm => {
            let id =
                get_nonempty(game, "game_id").map(|g| sub(&g)).ok_or_else(|| tr!("The script does not say which ScummVM game this is"))?;
            let dir = path("path").unwrap_or_else(|| game_dir.to_string_lossy().into_owned());
            let mut parts = vec!["scummvm".to_string(), format!("--path={}", quote(&dir))];
            if !args.is_empty() {
                parts.push(args);
            }
            parts.push(quote(&id));
            Ok(Launch::Command { folder: dir, command: parts.join(" ") })
        }
        Runner::Libretro => {
            let main = path("main_file").ok_or_else(missing)?;
            let core =
                get_nonempty(game, "core").map(|c| sub(&c)).ok_or_else(|| tr!("The script does not say which RetroArch core to use"))?;
            let name = if core.ends_with("_libretro") || core.ends_with(".so") { core.clone() } else { format!("{core}_libretro") };
            let core_path = heronite_emulation::detect::find_core_path(&heronite_emulation::SystemEnv, &name, None)
                .ok_or_else(|| tr!("The RetroArch core {core} is not installed", core))?;
            let command = format!("retroarch -L {} {}", quote(&core_path), quote(&main));
            Ok(Launch::Command { folder: folder(&main), command })
        }
        Runner::Web => {
            let url = get_nonempty(game, "main_file").map(|u| sub(&u)).ok_or_else(|| tr!("The script does not give the game's address"))?;
            Ok(Launch::Browser { url })
        }
    }
}

/// `launch_configs` of a Windows or Linux game.
pub fn extra_launches(game: &Map<String, Value>, sub: &dyn Fn(&str) -> String, game_dir: &Path) -> Vec<ExtraLaunch> {
    let Some(Value::Array(items)) = game.get("launch_configs") else { return Vec::new() };
    items
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|c| {
            let exe = path_of(&get_nonempty(c, "exe")?, sub, game_dir);
            let name = get_nonempty(c, "name").map(|n| n.trim().to_string()).unwrap_or_else(|| crate::steps::short(&exe));
            Some(ExtraLaunch { name, exe, args: get_nonempty(c, "args").map(|a| sub(&a)).unwrap_or_default() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::vars::Vars;
    #[cfg(unix)]
    use serde_json::json;

    #[cfg(unix)]
    fn sub_with(dir: &str) -> impl Fn(&str) -> String {
        let mut v = Vars::new();
        v.set("GAMEDIR", dir);
        v.set("VERSION", "26");
        move |s: &str| v.substitute(s)
    }

    #[cfg(unix)]
    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn runners() {
        assert_eq!(runner("wine"), Ok(Runner::Wine));
        assert!(runner("steam").is_err());
        assert!(runner("mame").unwrap_err().contains("mame"));
    }

    // A Wine prefix on a file system where case matters.
    #[cfg(unix)]
    #[test]
    fn windows_games_find_their_program_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("drive_c/Program Files (x86)/SP Football Life 2026")).unwrap();
        std::fs::write(d.join("drive_c/Program Files (x86)/SP Football Life 2026/FL 2026 Start.exe"), "").unwrap();
        let sub = sub_with(&d.to_string_lossy());
        let game = map(
            json!({"exe": "$GAMEDIR/drive_c/Program Files (x86)/SP Football Life 2026/FL 2026 start.exe", "args": "-x", "prefix": "$GAMEDIR"}),
        );
        let l = resolve(Runner::Wine, &game, &sub, d).unwrap();
        let Launch::Windows { exe, args, prefix } = l else { panic!() };
        assert!(exe.ends_with("FL 2026 Start.exe"), "{exe}");
        assert_eq!((args.as_str(), prefix.as_str()), ("-x", &*d.to_string_lossy()));
        let extra = extra_launches(
            &map(
                json!({"launch_configs": [{"exe": "drive_c/Program Files (x86)/SP Football Life 20${VERSION}/Settings.exe", "name": "Settings"}, {"name": "broken"}]}),
            ),
            &sub,
            d,
        );
        assert_eq!(extra.len(), 1);
        assert_eq!(extra[0].name, "Settings");
        assert!(extra[0].exe.starts_with(&*d.to_string_lossy()) && extra[0].exe.ends_with("SP Football Life 2026/Settings.exe"));
        assert!(resolve(Runner::Wine, &Map::new(), &sub, d).is_err());
    }

    // A Linux runner's paths.
    #[cfg(unix)]
    #[test]
    fn linux_games_and_commands() {
        let sub = sub_with("/g");
        let l = resolve(Runner::Linux, &map(json!({"exe": "bin/game.x86_64", "args": "--fs"})), &sub, Path::new("/g")).unwrap();
        assert_eq!(l, Launch::Linux { exe: "/g/bin/game.x86_64".into(), args: "--fs".into() });
        let l = resolve(Runner::Linux, &map(json!({"exe": "bin/my game", "working_dir": "$GAMEDIR"})), &sub, Path::new("/g")).unwrap();
        assert_eq!(l, Launch::Command { folder: "/g".into(), command: "'/g/bin/my game'".into() });
        let l = resolve(Runner::Scummvm, &map(json!({"game_id": "monkey", "path": "$GAMEDIR/data"})), &sub, Path::new("/g")).unwrap();
        assert_eq!(l, Launch::Command { folder: "/g/data".into(), command: "scummvm --path=/g/data monkey".into() });
        let l = resolve(Runner::Web, &map(json!({"main_file": "https://play.example.com"})), &sub, Path::new("/g")).unwrap();
        assert_eq!(l, Launch::Browser { url: "https://play.example.com".into() });
    }

    // A Linux runner's paths.
    #[cfg(unix)]
    #[test]
    fn dosbox_commands() {
        let sub = sub_with("/g");
        let l =
            resolve(Runner::Dosbox, &map(json!({"main_file": "$GAMEDIR/dosbox.conf", "config_file": "game.conf"})), &sub, Path::new("/g"))
                .unwrap();
        let Launch::Command { folder, command } = l else { panic!() };
        assert_eq!(folder, "/g");
        assert!(command.ends_with(" -conf /g/dosbox.conf -conf /g/game.conf"), "{command}");
        let l = resolve(Runner::Dosbox, &map(json!({"main_file": "GAME/RUN.EXE", "exit": false})), &sub, Path::new("/g")).unwrap();
        let Launch::Command { folder, command } = l else { panic!() };
        assert_eq!(folder, "/g/GAME");
        assert!(command.ends_with(" /g/GAME/RUN.EXE"), "{command}");
    }
}
