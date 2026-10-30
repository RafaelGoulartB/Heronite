//! Emulation set up by itself for a new setup: registers the emulators
//! found on this machine and, in the usual ROM folders (EmuDeck's,
//! RetroDECK's, `~/ROMs` with one folder per system; on Windows `ROMs` and
//! `Documents\ROMs` of the home and EmuDeck's `C:\Emulation\roms`), adds a
//! ROM folder for each system an emulator can play and imports its games.
//! Only the empty Collection runs it; elsewhere adding folders stays the
//! user's call.
//!
//! Nothing here executes a file; the folder walk is bounded.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use heronite_data::DataPaths;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};

use crate::catalog::emulator_definitions;
use crate::detect::DetectEnv;
use crate::discover::{add_discovered, discover_emulators};
use crate::import::run_rom_scanners;
use crate::store::{list_rom_scanners, list_user_emulators, upsert_rom_scanner};
use crate::types::UpsertRomScannerArgs;

/// Where ROM collections usually live, under the home folder.
#[cfg(not(windows))]
const ROOTS: &[&str] = &[
    "ROMs",
    "Roms",
    "roms",
    "Emulation/roms",
    "Emulation/ROMs",
    "Games/ROMs",
    "Games/roms",
    "Games/Emulation/roms",
    "retrodeck/roms",
    "RetroDECK/roms",
];
/// The same, at the top of other disks (a Steam Deck's SD card).
#[cfg(not(windows))]
const DISK_ROOTS: &[&str] = &["Emulation/roms", "roms", "retrodeck/roms"];
/// Windows: under the home folder (names there ignore case, `ROMs` is also
/// `roms`).
#[cfg(windows)]
const ROOTS: &[&str] =
    &["ROMs", "Emulation/roms", "Games/ROMs", "Games/Emulation/roms", "Documents/ROMs", "OneDrive/Documents/ROMs", "Emulators/ROMs"];
/// Windows: at the top of a drive (EmuDeck's `Emulation\roms`).
#[cfg(windows)]
const DISK_ROOTS: &[&str] = &["Emulation/roms", "ROMs", "Games/ROMs", "RetroArch-Win64/roms"];
/// Entries looked at per system folder before giving up on finding a game.
const WALK_BUDGET: usize = 4000;

/// What [`auto_setup`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupReport {
    /// The emulators on this machine (found now or added before).
    pub emulators: Vec<String>,
    /// ROM folders added.
    pub folders: usize,
    pub imported: usize,
    /// Systems with games but no emulator for them (their names).
    pub unplayable: Vec<String>,
}

/// A system folder: `<root>/<system>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFolder {
    pub dir: PathBuf,
    pub platform_id: &'static str,
}

/// The catalog platform of a system folder named as EmuDeck, RetroDECK and
/// ES-DE name them.
pub fn platform_of(folder: &str) -> Option<&'static str> {
    Some(match folder.to_ascii_lowercase().as_str() {
        "psx" | "ps1" | "playstation" => "sony_playstation",
        "ps2" | "playstation2" => "sony_playstation2",
        "ps3" | "playstation3" => "sony_playstation3",
        "ps4" => "sony_playstation4",
        "psp" => "sony_psp",
        "psvita" | "vita" => "sony_psvita",
        "gc" | "gamecube" | "ngc" => "nintendo_gamecube",
        "wii" => "nintendo_wii",
        "wiiu" => "nintendo_wiiu",
        "switch" => "nintendo_switch",
        "3ds" | "n3ds" => "nintendo_3ds",
        "nds" | "ds" => "nintendo_ds",
        "gb" | "gbc" | "gameboy" | "gameboycolor" => "nintendo_gameboy",
        "gba" | "gameboyadvance" => "nintendo_gameboyadvance",
        "snes" | "sfc" | "supernintendo" => "nintendo_super_nes",
        "nes" | "famicom" => "nintendo_nes",
        "n64" => "nintendo_64",
        "genesis" | "megadrive" | "md" => "sega_genesis",
        "mastersystem" | "sms" => "sega_master_system",
        "gamegear" | "gg" => "sega_gamegear",
        "saturn" => "sega_saturn",
        "dreamcast" | "dc" => "sega_dreamcast",
        "xbox" => "microsoft_xbox",
        "arcade" | "mame" | "fbneo" => "arcade",
        "pcengine" | "tg16" | "turbografx16" => "nec_turbografx_16",
        "atari2600" => "atari_2600",
        "lynx" | "atarilynx" => "atari_lynx",
        "ngp" | "ngpc" => "snk_neogeopocket",
        "wonderswan" | "wonderswancolor" | "wswan" | "wswanc" => "bandai_wonderswan",
        _ => return None,
    })
}

/// `base` joined with each `/`-separated part of `rel`.
fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, part| p.join(part))
}

/// The tops of the other disks ROM folders may be on: the disks mounted in
/// `/run/media/<user>` (a Steam Deck's SD card); on Windows the system drive.
pub fn disks() -> Vec<PathBuf> {
    if cfg!(windows) {
        let drive = std::env::var("SystemDrive").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "C:".into());
        return vec![PathBuf::from(format!("{drive}\\"))];
    }
    let media = PathBuf::from("/run/media").join(std::env::var("USER").unwrap_or_default());
    std::fs::read_dir(media).into_iter().flatten().flatten().map(|e| e.path()).collect()
}

/// The ROM roots that exist: under `home`, and at the top of `disks`
/// ([`disks`]).
pub fn rom_roots(home: &Path, disks: &[PathBuf]) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = ROOTS.iter().map(|r| under(home, r)).collect();
    all.extend(disks.iter().flat_map(|d| DISK_ROOTS.iter().map(move |r| under(d, r))));
    let mut seen = BTreeSet::new();
    all.into_iter().filter(|p| p.is_dir() && seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))).collect()
}

/// The system folders of the roots (empty ones included: [`has_game`]
/// tells).
pub fn find_rom_folders(roots: &[PathBuf]) -> Vec<RomFolder> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for e in entries.flatten() {
            let Some(platform_id) = platform_of(&e.file_name().to_string_lossy()) else { continue };
            if e.path().is_dir() {
                out.push(RomFolder { dir: e.path(), platform_id });
            }
        }
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// Whether `dir` (or a folder under it, two levels down) has a file with
/// one of `extensions`. EmuDeck creates every system folder with only a
/// `systeminfo.txt` inside, so an empty folder must not count.
pub fn has_game(dir: &Path, extensions: &[String]) -> bool {
    fn walk(dir: &Path, exts: &[String], depth: u8, budget: &mut usize) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        for e in entries.flatten() {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            let path = e.path();
            if path.is_dir() {
                if depth > 0 && walk(&path, exts, depth - 1, budget) {
                    return true;
                }
            } else if path.extension().is_some_and(|x| exts.iter().any(|w| x.eq_ignore_ascii_case(w))) {
                return true;
            }
        }
        false
    }
    let mut budget = WALK_BUDGET;
    walk(dir, extensions, 2, &mut budget)
}

/// A RetroArch profile only plays with its core.
fn usable(emulator: &UserEmulator, profile: &UserEmulatorProfile) -> bool {
    emulator.definition_id != "retroarch" || profile.core_path.as_deref().is_some_and(|c| Path::new(c).is_file())
}

/// The emulator and profile to play `folder` with: a dedicated emulator
/// before RetroArch, and only one whose files are in the folder.
pub fn pick<'a>(emulators: &'a [UserEmulator], folder: &RomFolder) -> Option<(&'a UserEmulator, &'a UserEmulatorProfile)> {
    let mut found: Vec<_> = emulators
        .iter()
        .flat_map(|e| e.profiles.iter().map(move |p| (e, p)))
        .filter(|(e, p)| p.platform_id.as_deref() == Some(folder.platform_id) && usable(e, p))
        .collect();
    found.sort_by_key(|(e, _)| e.definition_id == "retroarch");
    found.into_iter().find(|(_, p)| has_game(&folder.dir, &p.extensions))
}

/// The catalog's name of a platform and the files its emulators open.
fn catalog_platform(platform_id: &str) -> (String, Vec<String>) {
    let profiles = emulator_definitions().iter().flat_map(|d| &d.profiles).filter(|p| p.platform_id == platform_id);
    let mut name = String::new();
    let mut exts = Vec::new();
    for p in profiles {
        if name.is_empty() {
            name = p.platform_name.clone();
        }
        exts.extend(p.extensions.iter().cloned());
    }
    (name, exts)
}

/// Registers the emulators found and imports the games of the usual ROM
/// folders (see the module), at the top of `disks` too ([`disks`]).
/// Blocking: detection asks Flatpak, the walk reads folders; seconds at most.
pub fn auto_setup(paths: &DataPaths, env: &dyn DetectEnv, disks: &[PathBuf]) -> Result<SetupReport> {
    let current = list_user_emulators(paths)?;
    let new: Vec<_> = discover_emulators(env, &current).into_iter().filter(|d| !d.already_added).collect();
    if !new.is_empty() {
        add_discovered(paths, &new)?;
    }
    let emulators = list_user_emulators(paths)?;
    let mut report = SetupReport::default();
    let mut names = BTreeSet::new();
    report.emulators = emulators.iter().filter(|e| names.insert(e.name.clone())).map(|e| e.name.clone()).collect();

    let known: Vec<PathBuf> = list_rom_scanners(paths)?.into_iter().map(|s| PathBuf::from(s.directory)).collect();
    let mut unplayable = BTreeSet::new();
    for folder in find_rom_folders(&rom_roots(Path::new(&env.home()), disks)) {
        if known.contains(&folder.dir) {
            continue;
        }
        let Some((emulator, profile)) = pick(&emulators, &folder) else {
            let (name, exts) = catalog_platform(folder.platform_id);
            if !name.is_empty() && has_game(&folder.dir, &exts) {
                unplayable.insert(name);
            }
            continue;
        };
        let scanner = upsert_rom_scanner(
            paths,
            UpsertRomScannerArgs {
                emulator_id: emulator.id.clone(),
                profile_id: profile.id.clone(),
                directory: folder.dir.to_string_lossy().into_owned(),
                scan_subfolders: Some(true),
                merge_related_files: Some(true),
                auto_scan_on_refresh: Some(true),
                ..Default::default()
            },
        )?;
        report.folders += 1;
        let scan = run_rom_scanners(paths, Some(&scanner.id))?;
        for e in &scan.errors {
            log::warn!("ROM folder {}: {e}", folder.dir.display());
        }
        report.imported += scan.imported;
    }
    report.unplayable = unplayable.into_iter().collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    fn emulator(id: &str, definition: &str, platform: &str, exts: &[&str]) -> UserEmulator {
        UserEmulator {
            id: id.into(),
            definition_id: definition.into(),
            name: definition.into(),
            profiles: vec![UserEmulatorProfile {
                id: format!("{id}-p"),
                platform_id: Some(platform.into()),
                extensions: exts.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// A machine with nothing to detect, at `home` (its system drive too).
    struct Bare(PathBuf);
    impl DetectEnv for Bare {
        fn which(&self, _: &str) -> Option<String> {
            None
        }
        fn flatpak_installed(&self, _: &str) -> bool {
            false
        }
        fn exists(&self, path: &str) -> bool {
            Path::new(path).exists()
        }
        fn read_dir(&self, _: &str) -> Vec<String> {
            vec![]
        }
        fn home(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
        fn var(&self, key: &str) -> Option<String> {
            (key == "SystemDrive").then(|| self.home())
        }
    }

    #[test]
    fn imports_the_systems_an_emulator_plays_and_names_the_others() {
        let h = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(h.path().join("data"));
        let home = h.path().join("home");
        touch(&home.join("Emulation/roms/ps2/Okami.iso"));
        touch(&home.join("Emulation/roms/ps2/systeminfo.txt"));
        touch(&home.join("Emulation/roms/gc/Wind Waker.rvz"));
        touch(&home.join("Emulation/roms/snes/systeminfo.txt"));
        crate::store::upsert_user_emulator(
            &paths,
            crate::types::UpsertUserEmulatorArgs {
                definition_id: "pcsx2".into(),
                name: "PCSX2".into(),
                executable: h.path().join("pcsx2").to_string_lossy().into_owned(),
                launch_kind: "native".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let env = Bare(home.clone());
        let report = auto_setup(&paths, &env, &[]).unwrap();
        assert_eq!(report.emulators, ["PCSX2"]);
        assert_eq!((report.folders, report.imported), (1, 1));
        assert_eq!(report.unplayable, ["Nintendo GameCube"]);
        let scanners = list_rom_scanners(&paths).unwrap();
        assert_eq!(scanners.len(), 1);
        assert!(scanners[0].auto_scan_on_refresh);
        // A folder added once is left alone.
        let again = auto_setup(&paths, &env, &[]).unwrap();
        assert_eq!((again.folders, again.imported), (0, 0));
    }

    #[test]
    fn system_folders_are_named_like_emudeck_and_es_de() {
        assert_eq!(platform_of("PS2"), Some("sony_playstation2"));
        assert_eq!(platform_of("gc"), Some("nintendo_gamecube"));
        assert_eq!(platform_of("megadrive"), Some("sega_genesis"));
        assert_eq!(platform_of("bios"), None);
        assert_eq!(platform_of("media"), None);
    }

    #[test]
    fn finds_the_roots_and_their_system_folders() {
        let h = tempfile::tempdir().unwrap();
        let (home, sd) = (h.path().join("home"), h.path().join("media").join("sd"));
        touch(&home.join("Emulation/roms/ps2/God of War.iso"));
        touch(&home.join("Emulation/roms/bios/scph.bin"));
        touch(&sd.join("Emulation/roms/gba/Advance Wars.gba"));
        let roots = rom_roots(&home, std::slice::from_ref(&sd));
        assert_eq!(roots, [under(&home, "Emulation/roms"), under(&sd, "Emulation/roms")]);
        let folders = find_rom_folders(&roots);
        let found: Vec<_> = folders.iter().map(|f| f.platform_id).collect();
        assert_eq!(found, ["sony_playstation2", "nintendo_gameboyadvance"]);
    }

    #[test]
    fn an_emudeck_folder_with_only_its_notes_has_no_game() {
        let h = tempfile::tempdir().unwrap();
        touch(&h.path().join("ps2/systeminfo.txt"));
        let exts = vec!["iso".to_string(), "chd".to_string()];
        assert!(!has_game(&h.path().join("ps2"), &exts));
        touch(&h.path().join("ps2/Okami/Okami.CHD"));
        assert!(has_game(&h.path().join("ps2"), &exts));
    }

    #[test]
    fn a_dedicated_emulator_comes_before_retroarch_and_needs_its_files() {
        let h = tempfile::tempdir().unwrap();
        touch(&h.path().join("gba/Advance Wars.gba"));
        let folder = RomFolder { dir: h.path().join("gba"), platform_id: "nintendo_gameboyadvance" };
        let core = h.path().join("mgba_libretro.so");
        touch(&core);
        let mut ra = emulator("ra", "retroarch", "nintendo_gameboyadvance", &["gba"]);
        ra.profiles[0].core_path = Some(core.to_string_lossy().into_owned());
        let mgba = emulator("mg", "mgba", "nintendo_gameboyadvance", &["gba"]);
        let both = [ra.clone(), mgba];
        assert_eq!(pick(&both, &folder).map(|(e, _)| e.id.as_str()), Some("mg"));
        assert_eq!(pick(std::slice::from_ref(&ra), &folder).map(|(e, _)| e.id.as_str()), Some("ra"));
        // Without its core, RetroArch cannot play it.
        ra.profiles[0].core_path = None;
        assert!(pick(&[ra], &folder).is_none());
        // Nor an emulator for other files.
        assert!(pick(&[emulator("x", "pcsx2", "nintendo_gameboyadvance", &["iso"])], &folder).is_none());
    }
}
