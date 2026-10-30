//! The supported emulator engines: file-name parsing, state folders, ROM keys
//! and the launch arguments that load a state.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use heronite_launch::emulators::UserEmulator;
use regex::Regex;

use super::layouts::{
    child_dirs, dolphin_user_dir, duckstation_data_dirs, mgba_config_path, pcsx2_data_dirs, ppsspp_state_dir, retroarch_append_config,
    retroarch_config_dir, retroarch_core_name, rpcs3_states_root,
};
use super::{s, LaunchStateArgs, ParsedState, SaveStateKind};
use crate::catalog::guess_definition_id;
use crate::disc_id::{read_nintendo_disc_id, read_playstation_serial, read_ps3_title_id, read_psp_disc_id, serial_from_game_list_cache};
use crate::pathutil::{base_no_ext, basename, dirname, join, unique};
use crate::sysfiles::{find_embedded_png, read_zip_entry, resolve_configured_dir, HostEnv, Ini};

#[derive(Debug, Clone, Default)]
pub struct EngineContext {
    /// Emulator binary on the host; `None` for Flatpak installs.
    pub executable: Option<String>,
    pub flatpak_id: Option<String>,
    /// RetroArch core of the game's profile.
    pub core_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Pcsx2,
    DuckStation,
    Ppsspp,
    Dolphin,
    RetroArch,
    Mgba,
    Rpcs3,
}

/// Emulators without a CLI way to load a state at boot (melonDS, Azahar,
/// Cemu, Ryujinx, Vita3K, shadPS4, xemu) have no engine.
pub fn engine_for(id: &str) -> Option<Engine> {
    Some(match id {
        "pcsx2" => Engine::Pcsx2,
        "duckstation" => Engine::DuckStation,
        "ppsspp" => Engine::Ppsspp,
        "dolphin" => Engine::Dolphin,
        "retroarch" => Engine::RetroArch,
        "mgba" => Engine::Mgba,
        "rpcs3" => Engine::Rpcs3,
        _ => return None,
    })
}

/// `engineIdFor`: custom emulators are matched by name/binary/flatpak id.
pub fn engine_id_for(emulator: &UserEmulator) -> String {
    if emulator.definition_id != "custom" {
        return emulator.definition_id.clone();
    }
    let hints = [
        Some(emulator.name.clone()),
        Some(emulator.executable.clone()).filter(|e| !e.is_empty()).map(|e| basename(&e)),
        emulator.flatpak_id.clone(),
    ];
    for hint in hints {
        let id = guess_definition_id(hint.as_deref());
        if id != "custom" {
            return id;
        }
    }
    "custom".into()
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

fn slot(key: &str, n: u32) -> ParsedState {
    ParsedState { key: key.into(), kind: SaveStateKind::Slot, slot: Some(n) }
}

fn resume(key: &str) -> ParsedState {
    ParsedState { key: key.into(), kind: SaveStateKind::Resume, slot: None }
}

impl Engine {
    /// Serial-keyed engines learn the key from states written in a session.
    pub fn learns_keys(self) -> bool {
        !matches!(self, Engine::RetroArch | Engine::Mgba)
    }

    /// Directories that hold state files for these keys / this ROM.
    pub fn state_dirs(self, env: &HostEnv, ctx: &EngineContext, rom: &str, keys: &[String]) -> Vec<String> {
        match self {
            Engine::Pcsx2 => pcsx2_data_dirs(env, ctx)
                .into_iter()
                .map(|data| {
                    let ini = Ini::read(&data.join("inis").join("PCSX2.ini"));
                    resolve_configured_dir(ini.as_ref().and_then(|i| i.get("Folders", "Savestates")), &s(&data), &env.home)
                        .unwrap_or_else(|| s(&data.join("sstates")))
                })
                .collect(),
            Engine::DuckStation => duckstation_data_dirs(env, ctx)
                .into_iter()
                .map(|data| {
                    let ini = Ini::read(&data.join("settings.ini"));
                    resolve_configured_dir(ini.as_ref().and_then(|i| i.get("Folders", "SaveStates")), &s(&data), &env.home)
                        .unwrap_or_else(|| s(&data.join("savestates")))
                })
                .collect(),
            Engine::Ppsspp => vec![s(&ppsspp_state_dir(env, ctx))],
            Engine::Dolphin => vec![s(&dolphin_user_dir(env, ctx).join("StateSaves"))],
            Engine::RetroArch => {
                let config_dir = retroarch_config_dir(env, ctx);
                let cfg_dir = s(&config_dir);
                let cfg = Ini::read(&config_dir.join("retroarch.cfg"));
                let configured = cfg.as_ref().and_then(|c| c.get("", "savestate_directory")).map(|v| match v.strip_prefix(':') {
                    Some(rest) => format!("{cfg_dir}{rest}"),
                    None => v.to_string(),
                });
                let root = resolve_configured_dir(configured.as_deref(), &cfg_dir, &env.home).unwrap_or_else(|| join(&cfg_dir, "states"));
                if let Some(core) = retroarch_core_name(env, ctx) {
                    if Path::new(&root).join(&core).exists() {
                        return vec![root.clone(), join(&root, &core)];
                    }
                }
                let mut out = vec![root.clone()];
                out.extend(child_dirs(Path::new(&root)));
                out
            }
            Engine::Mgba => {
                let ini = Ini::read(&mgba_config_path(env, ctx));
                let configured =
                    resolve_configured_dir(ini.as_ref().and_then(|i| i.get("ports.qt", "savestatePath")), &dirname(rom), &env.home);
                unique([configured, Some(dirname(rom))])
            }
            Engine::Rpcs3 => {
                let root = rpcs3_states_root(env, ctx);
                if keys.is_empty() {
                    child_dirs(&root)
                } else {
                    keys.iter().map(|k| s(&root.join(k))).collect()
                }
            }
        }
    }

    /// Keys the emulator uses in state file names for this ROM.
    pub fn rom_keys(self, env: &HostEnv, ctx: &EngineContext, rom: &str) -> Vec<String> {
        let rom_path = Path::new(rom);
        match self {
            Engine::Pcsx2 => {
                let mut keys: Vec<Option<String>> = pcsx2_data_dirs(env, ctx)
                    .iter()
                    .map(|d| serial_from_game_list_cache(&d.join("cache").join("gamelist.cache"), rom))
                    .collect();
                keys.push(read_playstation_serial(rom_path));
                unique(keys)
            }
            Engine::DuckStation => {
                let mut keys: Vec<Option<String>> = duckstation_data_dirs(env, ctx)
                    .iter()
                    .map(|d| serial_from_game_list_cache(&d.join("cache").join("gamelist.cache"), rom))
                    .collect();
                keys.push(read_playstation_serial(rom_path));
                unique(keys)
            }
            Engine::Ppsspp => unique([read_psp_disc_id(rom_path)]),
            Engine::Dolphin => unique([read_nintendo_disc_id(rom_path)]),
            Engine::RetroArch | Engine::Mgba => vec![base_no_ext(rom)],
            Engine::Rpcs3 => unique([read_ps3_title_id(rom_path)]),
        }
    }

    /// Parses a file name in a state folder (`None` = not a state file).
    pub fn parse(self, file_name: &str) -> Option<ParsedState> {
        static PCSX2: OnceLock<Regex> = OnceLock::new();
        static DUCK: OnceLock<Regex> = OnceLock::new();
        static PSP: OnceLock<Regex> = OnceLock::new();
        static DOLPHIN: OnceLock<Regex> = OnceLock::new();
        static RA: OnceLock<Regex> = OnceLock::new();
        static MGBA: OnceLock<Regex> = OnceLock::new();
        static RPCS3: OnceLock<Regex> = OnceLock::new();
        match self {
            Engine::Pcsx2 => {
                let c = re(&PCSX2, r"(?i)^(.+?) \([0-9a-f]{8}\)\.(\d{2}|resume)\.p2s$").captures(file_name)?;
                Some(if c[2].eq_ignore_ascii_case("resume") { resume(&c[1]) } else { slot(&c[1], c[2].parse().ok()?) })
            }
            Engine::DuckStation => {
                let c = re(&DUCK, r"(?i)^(.+)_(\d+|resume)\.sav$").captures(file_name)?;
                if &c[1] == "savestate" {
                    return None;
                }
                Some(if c[2].eq_ignore_ascii_case("resume") { resume(&c[1]) } else { slot(&c[1], c[2].parse().ok()?) })
            }
            Engine::Ppsspp => {
                let c = re(&PSP, r"(?i)^([A-Z0-9]+)_[\d.]+_(\d+)\.ppst$").captures(file_name)?;
                Some(slot(&c[1], c[2].parse::<u32>().ok()? + 1))
            }
            Engine::Dolphin => {
                let c = re(&DOLPHIN, r"^([A-Z0-9]{6})\.s(\d{2})$").captures(file_name)?;
                Some(slot(&c[1], c[2].parse().ok()?))
            }
            Engine::RetroArch => {
                let c = re(&RA, r"^(.+)\.state(\d*|\.auto)$").captures(file_name)?;
                if &c[2] == ".auto" {
                    return Some(resume(&c[1]));
                }
                Some(slot(&c[1], if c[2].is_empty() { 0 } else { c[2].parse().ok()? }))
            }
            Engine::Mgba => {
                let c = re(&MGBA, r"^(.+)\.ss(\d)$").captures(file_name)?;
                Some(if &c[2] == "0" { resume(&c[1]) } else { slot(&c[1], c[2].parse().ok()?) })
            }
            Engine::Rpcs3 => {
                let c = re(&RPCS3, r"(?i)^([A-Z]{4}\d{5})_.*\.SAVESTAT(\.zst|\.gz)?$").captures(file_name)?;
                Some(ParsedState { key: c[1].to_string(), kind: SaveStateKind::Slot, slot: None })
            }
        }
    }

    /// Arguments that boot into this state.
    pub fn load_args(
        self,
        env: &HostEnv,
        ctx: &EngineContext,
        path: &str,
        kind: SaveStateKind,
        slot: Option<u32>,
    ) -> Result<LaunchStateArgs> {
        Ok(match self {
            Engine::Pcsx2 | Engine::DuckStation => LaunchStateArgs { args: vec!["-statefile".into(), path.into()], replaces_image: false },
            Engine::Ppsspp => LaunchStateArgs { args: vec![format!("--state={path}")], replaces_image: false },
            Engine::Dolphin => LaunchStateArgs { args: vec!["--save_state".into(), path.into()], replaces_image: false },
            Engine::RetroArch => {
                if kind == SaveStateKind::Resume {
                    let cfg = retroarch_append_config(env, ctx, true)?;
                    LaunchStateArgs { args: vec![format!("--appendconfig={cfg}")], replaces_image: false }
                } else {
                    LaunchStateArgs { args: vec!["-e".into(), slot.unwrap_or(0).to_string()], replaces_image: false }
                }
            }
            Engine::Mgba => LaunchStateArgs { args: vec!["-t".into(), path.into()], replaces_image: false },
            Engine::Rpcs3 => LaunchStateArgs { args: vec!["--savestate".into(), path.into()], replaces_image: true },
        })
    }

    /// Arguments that stop the emulator from auto-loading a state.
    pub fn fresh_args(self, env: &HostEnv, ctx: &EngineContext) -> Result<Vec<String>> {
        Ok(match self {
            Engine::RetroArch => {
                let cfg = Ini::read(&retroarch_config_dir(env, ctx).join("retroarch.cfg"));
                if cfg.as_ref().and_then(|c| c.get("", "savestate_auto_load")) != Some("true") {
                    return Ok(vec![]);
                }
                vec![format!("--appendconfig={}", retroarch_append_config(env, ctx, false)?)]
            }
            Engine::Mgba => vec!["-C".into(), "autoload=0".into()],
            _ => vec![],
        })
    }

    /// The screenshot stored with the state, if the emulator keeps one.
    pub fn thumbnail(self, state_path: &str) -> Option<Vec<u8>> {
        let p = Path::new(state_path);
        match self {
            Engine::Pcsx2 => read_zip_entry(p, "Screenshot.png"),
            Engine::DuckStation | Engine::Mgba => find_embedded_png(p),
            Engine::Ppsspp => {
                let jpg = Regex::new(r"(?i)\.ppst$").unwrap().replace(state_path, ".jpg").into_owned();
                std::fs::read(jpg).ok()
            }
            Engine::RetroArch => std::fs::read(format!("{state_path}.png")).ok(),
            Engine::Dolphin | Engine::Rpcs3 => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ctx() -> EngineContext {
        EngineContext::default()
    }

    #[test]
    fn parses_every_engine_file_name() {
        let p = |e: Engine, n: &str| e.parse(n);
        assert_eq!(p(Engine::Pcsx2, "SLUS-20731 (538ABF5D).resume.p2s"), Some(resume("SLUS-20731")));
        assert_eq!(p(Engine::Pcsx2, "SLUS-20731 (538ABF5D).03.p2s"), Some(slot("SLUS-20731", 3)));
        assert_eq!(p(Engine::Pcsx2, "memcard.ps2"), None);
        assert_eq!(p(Engine::DuckStation, "SLUS-00594_resume.sav"), Some(resume("SLUS-00594")));
        assert_eq!(p(Engine::DuckStation, "SLUS-00594_2.sav"), Some(slot("SLUS-00594", 2)));
        assert_eq!(p(Engine::DuckStation, "savestate_1.sav"), None);
        assert_eq!(p(Engine::Ppsspp, "ULUS10041_1.00_0.ppst"), Some(slot("ULUS10041", 1)));
        assert_eq!(p(Engine::Ppsspp, "ULUS10041_1.00_0.jpg"), None);
        assert_eq!(p(Engine::Dolphin, "GALE01.s01"), Some(slot("GALE01", 1)));
        assert_eq!(p(Engine::Dolphin, "GALE01.sav"), None);
        assert_eq!(p(Engine::RetroArch, "Zelda.state"), Some(slot("Zelda", 0)));
        assert_eq!(p(Engine::RetroArch, "Zelda.state3"), Some(slot("Zelda", 3)));
        assert_eq!(p(Engine::RetroArch, "Zelda.state.auto"), Some(resume("Zelda")));
        assert_eq!(p(Engine::RetroArch, "Zelda.state.png"), None);
        assert_eq!(p(Engine::Mgba, "Pokemon.ss0"), Some(resume("Pokemon")));
        assert_eq!(p(Engine::Mgba, "Pokemon.ss4"), Some(slot("Pokemon", 4)));
        assert_eq!(p(Engine::Rpcs3, "BCUS98114_2024-01-01.SAVESTAT.zst").map(|s| s.key), Some("BCUS98114".to_string()));
        assert_eq!(p(Engine::Rpcs3, "notes.txt"), None);
    }

    #[test]
    fn engine_ids_for_custom_emulators() {
        let mut e = UserEmulator {
            definition_id: "custom".into(),
            name: "PCSX2".into(),
            executable: "/x/pcsx2-v2.AppImage".into(),
            ..Default::default()
        };
        assert_eq!(engine_id_for(&e), "pcsx2");
        e.name = "My emu".into();
        assert_eq!(engine_id_for(&e), "pcsx2");
        e.executable = "/x/thing".into();
        e.flatpak_id = Some("org.DolphinEmu.dolphin-emu".into());
        assert_eq!(engine_id_for(&e), "dolphin");
        e.flatpak_id = None;
        assert_eq!(engine_id_for(&e), "custom");
        e.definition_id = "mgba".into();
        assert_eq!(engine_id_for(&e), "mgba");
        assert!(engine_for("melonds").is_none());
    }

    // The XDG and Flatpak folders are Linux's.
    #[cfg(unix)]
    #[test]
    fn state_dirs_follow_each_emulator_layout() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let env = HostEnv::sandbox(&home, d.path().join("tmp"));
        // PCSX2: default and configured.
        let dirs = Engine::Pcsx2.state_dirs(&env, &ctx(), "/r/a.iso", &[]);
        assert_eq!(dirs, [s(&home.join(".config/PCSX2/sstates"))]);
        fs::create_dir_all(home.join(".config/PCSX2/inis")).unwrap();
        fs::write(home.join(".config/PCSX2/inis/PCSX2.ini"), "[Folders]\nSavestates = /data/states\n").unwrap();
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &ctx(), "/r/a.iso", &[]), ["/data/states"]);
        // Flatpak PCSX2.
        let fp = EngineContext { flatpak_id: Some("net.pcsx2.PCSX2".into()), ..ctx() };
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &fp, "/r/a.iso", &[]), [s(&home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/sstates"))]);
        // Portable AppImage dir.
        let portable = d.path().join("portable");
        fs::create_dir_all(&portable).unwrap();
        fs::write(portable.join("portable.txt"), "").unwrap();
        let pc = EngineContext { executable: Some(s(&portable.join("pcsx2.AppImage"))), ..ctx() };
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &pc, "/r/a.iso", &[]), [s(&portable.join("sstates"))]);
        // PPSSPP / Dolphin / RPCS3 / mGBA.
        assert_eq!(Engine::Ppsspp.state_dirs(&env, &ctx(), "", &[]), [s(&home.join(".config/ppsspp/PSP/PPSSPP_STATE"))]);
        assert_eq!(Engine::Dolphin.state_dirs(&env, &ctx(), "", &[]), [s(&home.join(".local/share/dolphin-emu/StateSaves"))]);
        fs::create_dir_all(home.join(".dolphin-emu")).unwrap();
        assert_eq!(Engine::Dolphin.state_dirs(&env, &ctx(), "", &[]), [s(&home.join(".dolphin-emu/StateSaves"))]);
        assert_eq!(
            Engine::Rpcs3.state_dirs(&env, &ctx(), "", &["BCUS98114".into()]),
            [s(&home.join(".config/rpcs3/savestates/BCUS98114"))]
        );
        assert_eq!(Engine::Mgba.state_dirs(&env, &ctx(), "/roms/gba/p.gba", &[]), ["/roms/gba"]);
        fs::create_dir_all(home.join(".config/mgba")).unwrap();
        fs::write(home.join(".config/mgba/config.ini"), "[ports.qt]\nsavestatePath=states\n").unwrap();
        assert_eq!(Engine::Mgba.state_dirs(&env, &ctx(), "/roms/gba/p.gba", &[]), ["/roms/gba/states", "/roms/gba"]);
        // DuckStation: only existing candidate roots.
        assert!(Engine::DuckStation.state_dirs(&env, &ctx(), "", &[]).is_empty());
        fs::create_dir_all(home.join(".local/share/duckstation")).unwrap();
        assert_eq!(Engine::DuckStation.state_dirs(&env, &ctx(), "", &[]), [s(&home.join(".local/share/duckstation/savestates"))]);
    }

    // The XDG and Flatpak folders are Linux's.
    #[cfg(unix)]
    #[test]
    fn retroarch_dirs_and_launch_args() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let tmp = d.path().join("tmp");
        let env = HostEnv::sandbox(&home, &tmp);
        let cfg = home.join(".config/retroarch");
        fs::create_dir_all(cfg.join("states/Snes9x")).unwrap();
        fs::create_dir_all(cfg.join("states/Other")).unwrap();
        fs::create_dir_all(cfg.join("cores")).unwrap();
        fs::write(cfg.join("cores/snes9x_libretro.info"), "corename = \"Snes9x\"\n").unwrap();
        fs::write(cfg.join("retroarch.cfg"), "savestate_auto_load = \"true\"\n").unwrap();
        let rc = EngineContext { core_path: Some(s(&cfg.join("cores/snes9x_libretro.so"))), ..ctx() };
        let states = s(&cfg.join("states"));
        assert_eq!(Engine::RetroArch.state_dirs(&env, &rc, "/r/z.sfc", &[]), [states.clone(), format!("{states}/Snes9x")]);
        // Without a core: root plus every child folder.
        assert_eq!(
            Engine::RetroArch.state_dirs(&env, &ctx(), "/r/z.sfc", &[]),
            [states.clone(), format!("{states}/Other"), format!("{states}/Snes9x")]
        );
        // Slot loads by number, auto state goes through appendconfig.
        let slot3 = Engine::RetroArch.load_args(&env, &rc, "/p", SaveStateKind::Slot, Some(3)).unwrap();
        assert_eq!(slot3.args, ["-e", "3"]);
        let auto = Engine::RetroArch.load_args(&env, &rc, "/p", SaveStateKind::Resume, None).unwrap();
        let file = auto.args[0].strip_prefix("--appendconfig=").unwrap();
        assert!(file.ends_with("heronite-retroarch-autoload-true.cfg"));
        assert_eq!(fs::read_to_string(file).unwrap(), "savestate_auto_load = \"true\"\n");
        let fresh = Engine::RetroArch.fresh_args(&env, &rc).unwrap();
        assert!(fresh[0].ends_with("heronite-retroarch-autoload-false.cfg"));
        fs::write(cfg.join("retroarch.cfg"), "savestate_auto_load = \"false\"\n").unwrap();
        assert!(Engine::RetroArch.fresh_args(&env, &rc).unwrap().is_empty());
    }

    #[test]
    fn launch_args_per_engine() {
        let env = HostEnv::sandbox("/h", "/t");
        let la = |e: Engine, kind, slot| e.load_args(&env, &ctx(), "/st/file", kind, slot).unwrap();
        assert_eq!(la(Engine::Pcsx2, SaveStateKind::Slot, Some(1)).args, ["-statefile", "/st/file"]);
        assert_eq!(la(Engine::DuckStation, SaveStateKind::Resume, None).args, ["-statefile", "/st/file"]);
        assert_eq!(la(Engine::Ppsspp, SaveStateKind::Slot, Some(1)).args, ["--state=/st/file"]);
        assert_eq!(la(Engine::Dolphin, SaveStateKind::Slot, Some(1)).args, ["--save_state", "/st/file"]);
        assert_eq!(la(Engine::Mgba, SaveStateKind::Slot, Some(1)).args, ["-t", "/st/file"]);
        let rp = la(Engine::Rpcs3, SaveStateKind::Slot, None);
        assert_eq!(rp.args, ["--savestate", "/st/file"]);
        assert!(rp.replaces_image);
        assert_eq!(Engine::Mgba.fresh_args(&env, &ctx()).unwrap(), ["-C", "autoload=0"]);
        assert!(Engine::Pcsx2.fresh_args(&env, &ctx()).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn state_dirs_follow_each_emulator_layout_on_windows() {
        let d = tempfile::tempdir().unwrap();
        let env = HostEnv::sandbox(d.path().join("home"), d.path().join("tmp"));
        let exe = |dir: &std::path::Path, name: &str| EngineContext { executable: Some(s(&dir.join(name))), ..ctx() };
        // PCSX2: Documents, configured, portable.
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &ctx(), r"D:\r\a.iso", &[]), [s(&env.documents.join(r"PCSX2\sstates"))]);
        fs::create_dir_all(env.documents.join(r"PCSX2\inis")).unwrap();
        fs::write(env.documents.join(r"PCSX2\inis\PCSX2.ini"), "[Folders]\r\nSavestates = D:\\States\r\n").unwrap();
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &ctx(), r"D:\r\a.iso", &[]), [r"D:\States"]);
        let portable = d.path().join("PCSX2");
        fs::create_dir_all(&portable).unwrap();
        fs::write(portable.join("portable.ini"), "").unwrap();
        assert_eq!(Engine::Pcsx2.state_dirs(&env, &exe(&portable, "pcsx2-qt.exe"), "", &[]), [s(&portable.join("sstates"))]);
        // DuckStation: only the folders that exist, today's first.
        assert!(Engine::DuckStation.state_dirs(&env, &ctx(), "", &[]).is_empty());
        fs::create_dir_all(env.documents.join("DuckStation")).unwrap();
        fs::create_dir_all(env.local_appdata.join("DuckStation")).unwrap();
        assert_eq!(
            Engine::DuckStation.state_dirs(&env, &ctx(), "", &[]),
            [s(&env.local_appdata.join(r"DuckStation\savestates")), s(&env.documents.join(r"DuckStation\savestates"))]
        );
        // PPSSPP: the installer's Documents, or the zip build's memstick.
        assert_eq!(Engine::Ppsspp.state_dirs(&env, &ctx(), "", &[]), [s(&env.documents.join(r"PPSSPP\PSP\PPSSPP_STATE"))]);
        let zip = d.path().join("ppsspp");
        fs::create_dir_all(zip.join("memstick")).unwrap();
        assert_eq!(
            Engine::Ppsspp.state_dirs(&env, &exe(&zip, "PPSSPPWindows64.exe"), "", &[]),
            [s(&zip.join(r"memstick\PSP\PPSSPP_STATE"))]
        );
        // Dolphin: AppData, Documents of older versions, a portable `User`.
        assert_eq!(Engine::Dolphin.state_dirs(&env, &ctx(), "", &[]), [s(&env.appdata.join(r"Dolphin Emulator\StateSaves"))]);
        fs::create_dir_all(env.documents.join("Dolphin Emulator")).unwrap();
        assert_eq!(Engine::Dolphin.state_dirs(&env, &ctx(), "", &[]), [s(&env.documents.join(r"Dolphin Emulator\StateSaves"))]);
        let dolphin = d.path().join("Dolphin-x64");
        fs::create_dir_all(&dolphin).unwrap();
        fs::write(dolphin.join("portable.txt"), "").unwrap();
        assert_eq!(Engine::Dolphin.state_dirs(&env, &exe(&dolphin, "Dolphin.exe"), "", &[]), [s(&dolphin.join(r"User\StateSaves"))]);
        // RPCS3: its own folder.
        let rpcs3 = EngineContext { executable: Some(r"C:\Emu\RPCS3\rpcs3.exe".into()), ..ctx() };
        assert_eq!(Engine::Rpcs3.state_dirs(&env, &rpcs3, "", &["BCUS98114".into()]), [r"C:\Emu\RPCS3\savestates\BCUS98114"]);
        // mGBA: next to the ROM, or where AppData's config says.
        assert_eq!(Engine::Mgba.state_dirs(&env, &ctx(), r"D:\roms\gba\p.gba", &[]), [r"D:\roms\gba"]);
        fs::create_dir_all(env.appdata.join("mGBA")).unwrap();
        fs::write(env.appdata.join(r"mGBA\config.ini"), "[ports.qt]\nsavestatePath=states\n").unwrap();
        assert_eq!(Engine::Mgba.state_dirs(&env, &ctx(), r"D:\roms\gba\p.gba", &[]), [r"D:\roms\gba\states", r"D:\roms\gba"]);
    }

    #[cfg(windows)]
    #[test]
    fn retroarch_keeps_its_config_and_states_next_to_it_on_windows() {
        let d = tempfile::tempdir().unwrap();
        let ra = d.path().join("RetroArch-Win64");
        fs::create_dir_all(ra.join(r"states\Snes9x")).unwrap();
        fs::create_dir_all(ra.join("cores")).unwrap();
        fs::write(ra.join(r"cores\snes9x_libretro.info"), "corename = \"Snes9x\"\n").unwrap();
        // `:` is RetroArch's own folder.
        fs::write(ra.join("retroarch.cfg"), "savestate_directory = \":\\states\"\nsavestate_auto_load = \"true\"\n").unwrap();
        let env = HostEnv::sandbox(d.path().join("home"), d.path().join("tmp"));
        let rc = EngineContext {
            executable: Some(s(&ra.join("retroarch.exe"))),
            core_path: Some(s(&ra.join(r"cores\snes9x_libretro.dll"))),
            ..ctx()
        };
        assert_eq!(Engine::RetroArch.state_dirs(&env, &rc, r"D:\r\z.sfc", &[]), [s(&ra.join("states")), s(&ra.join(r"states\Snes9x"))]);
        let fresh = Engine::RetroArch.fresh_args(&env, &rc).unwrap();
        assert!(fresh[0].ends_with("heronite-retroarch-autoload-false.cfg"));
    }
}
