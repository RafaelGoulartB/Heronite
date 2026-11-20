use heronite_data::models::{LocalGameMeta, LocalGameRom};

use super::*;

fn entry(platform: Option<&str>, emulator: Option<&str>, rom: &str, dir: Option<&str>) -> LocalGameMeta {
    LocalGameMeta {
        source: "emulator".into(),
        launch_kind: "emulator".into(),
        platform_id: platform.map(Into::into),
        emulator_name: emulator.map(Into::into),
        roms: Some(vec![LocalGameRom { name: None, path: rom.into(), extra: Default::default() }]),
        windows_install_directory: dir.map(Into::into),
        ..Default::default()
    }
}

fn id(meta: &LocalGameMeta, record: &[&str]) -> Option<&'static str> {
    let record: Vec<String> = record.iter().map(|s| s.to_string()).collect();
    resolve(meta, &record).map(|p| p.id)
}

#[test]
fn every_row_is_unique_and_named_in_launchbox() {
    for (i, p) in PLATFORMS.iter().enumerate() {
        assert!(PLATFORMS[..i].iter().all(|q| q.id != p.id), "{} twice", p.id);
        assert!(!p.names_in(Database::LaunchBox).is_empty(), "{} has no LaunchBox name", p.id);
        for alias in p.aliases {
            assert_eq!(normalize(alias), *alias, "{}: aliases are stored normalized", p.id);
            let others: Vec<&str> = PLATFORMS.iter().filter(|q| q.id != p.id && q.aliases.contains(alias)).map(|q| q.id).collect();
            assert!(others.is_empty(), "{alias} names {} and {others:?}", p.id);
        }
    }
    // The emulator catalog's ids are all here.
    for id in ["nintendo_super_nes", "sony_playstation2", "sega_genesis", "arcade", "pc", "nintendo_wiiu", "bandai_wonderswan"] {
        assert!(by_id(id).is_some(), "{id}");
    }
}

#[test]
fn launchbox_column() {
    let snes = by_id("nintendo_super_nes").unwrap();
    assert_eq!(snes.names_in(Database::LaunchBox)[0], "Super Nintendo Entertainment System");
    assert_eq!(by_id("sony_playstation").unwrap().names_in(Database::LaunchBox), ["Sony Playstation"]);
    assert_eq!(by_id("nintendo_gameboy").unwrap().names_in(Database::LaunchBox), ["Nintendo Game Boy", "Nintendo Game Boy Color"]);
    assert_eq!(from_database(Database::LaunchBox, "Sega Genesis").map(|p| p.id), Some("sega_genesis"));
    assert_eq!(from_database(Database::LaunchBox, "Nintendo Game Boy Color").map(|p| p.id), Some("nintendo_gameboy"));
    // MS-DOS is DOS's own before it is ScummVM's.
    assert_eq!(from_database(Database::LaunchBox, "MS-DOS").map(|p| p.id), Some("pc_dos"));
    assert_eq!(from_database(Database::LaunchBox, "Tiger Game.com"), None);
}

#[test]
fn names_and_extensions() {
    assert_eq!(by_name("Super Nintendo Entertainment System").map(|p| p.id), Some("nintendo_super_nes"));
    assert_eq!(by_name("PlayStation 2").map(|p| p.id), Some("sony_playstation2"));
    assert_eq!(by_name("Sega Mega Drive/Genesis").map(|p| p.id), Some("sega_genesis"));
    assert_eq!(by_name("PC (Microsoft Windows)").map(|p| p.id), Some("pc_windows"));
    assert_eq!(by_name("DuckStation").map(|p| p.id), Some("sony_playstation"));
    assert_eq!(by_name(""), None);
    assert_eq!(by_extension("SFC").map(|p| p.id), Some("nintendo_super_nes"));
    assert_eq!(by_extension("rpx").map(|p| p.id), Some("nintendo_wiiu"));
    for shared in ["iso", "bin", "cue", "chd", "zip", "m3u"] {
        assert_eq!(by_extension(shared), None, "{shared}");
    }
}

/// Entries as the Collection keeps them (Playnite imports without a
/// platform id, scans with `other`, the catalog's own ids).
#[test]
fn the_platform_of_real_entries() {
    let catalog = entry(Some("nintendo_super_nes"), Some("RetroArch"), "/roms/Super Metroid (Japan, USA) (En,Ja).sfc", None);
    assert_eq!(id(&catalog, &[]), Some("nintendo_super_nes"));
    // Playnite: no platform id, an emulator named by the user.
    let sotn = entry(
        None,
        Some("Duckstation"),
        "{InstallDir}\\Castlevania - Symphony of the Night (USA).cue",
        Some("D:\\Games\\_PSS\\PS_Games\\Castlevania - Symphony of the Night (USA)"),
    );
    assert_eq!(id(&sotn, &["PlayStation 3", "Sega Saturn", "PlayStation", "PlayStation Portable"]), Some("sony_playstation"));
    let mgs2 = entry(None, Some("PS3"), "{InstallDir}\\EBOOT.BIN", Some("C:\\Games\\PS3_Games\\PS3_GAME\\USRDIR\\"));
    assert_eq!(id(&mgs2, &[]), Some("sony_playstation3"));
    // The user named Cemu "Wii": the Wii U's own file says better.
    let botw =
        entry(None, Some("Wii"), "{InstallDir}\\The Legend of Zelda Breath of the Wild\\code\\U-King.rpx", Some("D:\\Games\\Nintendo\\"));
    assert_eq!(id(&botw, &["Wii U", "Nintendo Switch"]), Some("nintendo_wiiu"));
    // A scan with a custom emulator: `other`, an emulator and a folder that say it.
    let atv = entry(Some("other"), Some("PCSX2"), "/data/Games/Emulation/PS2/games/ATV Offroad Fury 4 (USA).iso", None);
    assert_eq!(id(&atv, &["Other"]), Some("sony_playstation2"));
    let by_folder_only = entry(Some("other"), Some("My emulator"), "/data/Games/Emulation/PS2/games/ATV Offroad Fury 4 (USA).iso", None);
    assert_eq!(id(&by_folder_only, &[]), Some("sony_playstation2"));
    let gow = entry(None, Some("My emulator"), "{InstallDir}\\God of War II (USA).iso", Some("D:\\Games\\_PSS\\PS2_Games\\"));
    assert_eq!(id(&gow, &[]), Some("sony_playstation2"));
    // Only the record says it, once.
    let record_only = entry(None, None, "/games/Burnout 3.iso", Some("/games"));
    assert_eq!(id(&record_only, &["PlayStation 2"]), Some("sony_playstation2"));
    assert_eq!(id(&record_only, &["Xbox", "PlayStation 2"]), None, "two platforms: not sure");
    assert_eq!(id(&record_only, &["PC (Microsoft Windows)", "PlayStation 2"]), Some("sony_playstation2"), "PC is not an emulated platform");
    assert_eq!(id(&record_only, &[]), None);
}

#[test]
fn the_file_a_dump_is_matched_by() {
    let file = |rom: &str, dir: Option<&str>| rom_file(&entry(None, None, rom, dir));
    assert_eq!(file("/roms/snes/Super Metroid (Japan, USA) (En,Ja).sfc", None).as_deref(), Some("Super Metroid (Japan, USA) (En,Ja).sfc"));
    assert_eq!(file("{InstallDir}\\God of War II (USA).iso", Some("D:\\PS2\\")).as_deref(), Some("God of War II (USA).iso"));
    // Boot files: the folder that names the game.
    assert_eq!(
        file("{InstallDir}\\EBOOT.BIN", Some("C:\\Games\\PS3_Games\\Burnout Paradise\\PS3_GAME\\USRDIR\\")).as_deref(),
        Some("Burnout Paradise")
    );
    assert_eq!(file("{InstallDir}\\EBOOT.BIN", Some("C:\\Games\\PS3_Games\\PS3_GAME\\USRDIR\\")), None, "nothing names it");
    assert_eq!(
        file("{InstallDir}\\The Legend of Zelda Breath of the Wild\\code\\U-King.rpx", Some("D:\\Games\\Nintendo\\")).as_deref(),
        Some("The Legend of Zelda Breath of the Wild")
    );
    let mut picked = entry(None, None, "/roms/a.sfc", None);
    picked.selected_rom_path = Some("/roms/Chrono Trigger (USA).sfc".into());
    assert_eq!(rom_file(&picked).as_deref(), Some("Chrono Trigger (USA).sfc"));
    assert_eq!(rom_file(&LocalGameMeta::default()), None);
}
