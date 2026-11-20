use std::collections::BTreeSet;

use super::*;
use crate::platforms::PLATFORMS;

/// The folders of `thumbnails.libretro.com`, as its index listed them on
/// 2026-11-10 (the `stats` folder is not a system).
const SYSTEMS: &str = include_str!("../../tests/fixtures/libretro/systems.txt");

/// The first 32 bytes the server sent for `Super Metroid (Japan, USA) (En,Ja).png`.
const SUPER_METROID_HEAD: &[u8] = include_bytes!("../../tests/fixtures/libretro/super_metroid_head.png");

fn real_systems() -> BTreeSet<&'static str> {
    SYSTEMS.lines().filter(|l| !l.trim().is_empty()).collect()
}

#[test]
fn every_system_the_table_names_exists_on_the_server() {
    let real = real_systems();
    assert!(real.len() > 100 && real.contains("Sony - PlayStation"));
    for p in PLATFORMS {
        for system in p.names_in(Database::Libretro) {
            assert!(real.contains(system), "{}: no folder `{system}` on the server", p.id);
        }
    }
    for (extension, system) in BY_EXTENSION {
        assert!(real.contains(system), ".{extension}: no folder `{system}`");
        // The platform that uses the extension is the one that names the system.
        let platform = platforms::by_extension(extension).unwrap_or_else(|| panic!(".{extension} is no platform's"));
        assert!(platform.names_in(Database::Libretro).contains(system), ".{extension} is {}'s, not `{system}`'s", platform.id);
    }
    // Every system of a platform with several is told by some extension, or by none (discs).
    for p in PLATFORMS.iter().filter(|p| p.names_in(Database::Libretro).len() > 1) {
        assert!(BY_EXTENSION.iter().any(|(_, s)| p.names_in(Database::Libretro).contains(s)), "{}: no extension picks a system", p.id);
    }
    // No folder is the main system of two platforms.
    let mains: Vec<&str> = PLATFORMS.iter().filter_map(|p| p.names_in(Database::Libretro).first().copied()).collect();
    assert_eq!(mains.len(), mains.iter().collect::<BTreeSet<_>>().len());
}

#[test]
fn the_platforms_with_and_without_a_folder() {
    let without: Vec<&str> = PLATFORMS.iter().filter(|p| p.names_in(Database::Libretro).is_empty()).map(|p| p.id).collect();
    assert_eq!(
        without,
        [
            "nintendo_switch",
            "sony_playstation3",
            "sony_playstation4",
            "sony_psvita",
            "microsoft_xbox360",
            "arcade",
            "pc_dos",
            "pc_windows",
            "pc"
        ]
    );
    assert_eq!(PLATFORMS.len() - without.len(), 43);
    let snes = platforms::by_id("nintendo_super_nes").unwrap();
    assert_eq!(snes.names_in(Database::Libretro)[0], "Nintendo - Super Nintendo Entertainment System");
    assert_eq!(platforms::from_database(Database::Libretro, "Sony - PlayStation").map(|p| p.id), Some("sony_playstation"));
    assert_eq!(platforms::from_database(Database::Libretro, "Nintendo - Game Boy Color").map(|p| p.id), Some("nintendo_gameboy"));
    assert_eq!(platforms::from_database(Database::Libretro, "Sony - PlayStation 3"), None);
}

#[test]
fn the_extension_picks_the_system() {
    let gb = platforms::by_id("nintendo_gameboy").unwrap();
    assert_eq!(systems_for(gb, Some("Tetris (World).gb")), ["Nintendo - Game Boy"]);
    assert_eq!(systems_for(gb, Some("Pokemon - Crystal Version (USA, Europe).GBC")), ["Nintendo - Game Boy Color"]);
    assert_eq!(
        systems_for(gb, Some("Tetris (World).zip")),
        ["Nintendo - Game Boy", "Nintendo - Game Boy Color"],
        "an archive says nothing"
    );
    assert_eq!(systems_for(gb, None), ["Nintendo - Game Boy", "Nintendo - Game Boy Color"]);
    let snes = platforms::by_id("nintendo_super_nes").unwrap();
    assert_eq!(systems_for(snes, Some("Super Metroid (Japan, USA) (En,Ja).sfc")), ["Nintendo - Super Nintendo Entertainment System"]);
    assert_eq!(systems_for(snes, Some("BS Zelda.bs")), ["Nintendo - Satellaview"]);
    let pce = platforms::by_id("nec_turbografx_16").unwrap();
    assert_eq!(systems_for(pce, Some("Bonk's Adventure (USA).pce")), ["NEC - PC Engine - TurboGrafx 16"]);
    assert_eq!(systems_for(pce, Some("Gate of Thunder (USA).chd")).len(), 3, "a disc image says nothing: every system is asked");
    assert_eq!(systems_for(pce, Some("SuperGrafx.sgx")), ["NEC - PC Engine SuperGrafx"]);
    let ps = platforms::by_id("sony_playstation").unwrap();
    assert_eq!(systems_for(ps, Some("Castlevania - Symphony of the Night (USA).cue")), ["Sony - PlayStation"]);
    let ps3 = platforms::by_id("sony_playstation3").unwrap();
    assert!(systems_for(ps3, Some("EBOOT.BIN")).is_empty());
}

#[test]
fn a_pngs_size_is_in_its_first_bytes() {
    assert_eq!(SUPER_METROID_HEAD.len(), 32);
    assert_eq!(png_size(SUPER_METROID_HEAD), Some((512, 357)));
    assert_eq!(png_size(&SUPER_METROID_HEAD[..23]), None, "cut before the height");
    assert_eq!(png_size(&SUPER_METROID_HEAD[..24]), Some((512, 357)));
    assert_eq!(png_size(b"<!DOCTYPE HTML PUBLIC \"-//IETF//DTD HTML 2.0//EN\"><html>"), None);
    let mut zero = SUPER_METROID_HEAD.to_vec();
    zero[16..20].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(png_size(&zero), None);
    let mut other = SUPER_METROID_HEAD.to_vec();
    other[12..16].copy_from_slice(b"IDAT");
    assert_eq!(png_size(&other), None);
}

/// Sizes of real box art (the server's, 2026-11-10).
#[test]
fn the_shape_decides_the_family() {
    use CoverFamily::*;
    for (w, h, family, what) in [
        (512, 357, Wide, "SNES, N64: landscape boxes"),
        (512, 512, Square, "PlayStation, Game Boy Advance"),
        (512, 504, Square, "Dreamcast"),
        (355, 512, Tall, "NES"),
        (359, 512, Tall, "NES"),
        (512, 731, Tall, "Mega Drive"),
        (483, 680, Tall, "Mega Drive"),
        (512, 883, Tall, "PSP"),
        (512, 724, Tall, "PlayStation 2"),
        (512, 663, Tall, "Amiga"),
        (512, 416, Square, "ratio 1.23: a little wide, still square"),
        (512, 1024, Tall, "very tall"),
        (1000, 1000, Square, ""),
        (1920, 1080, Wide, ""),
    ] {
        assert_eq!(family_of(w, h), family, "{w}x{h} ({what})");
    }
    // The limits sit between the families' own ratios.
    assert_eq!(family_of(3, 4), Tall);
    assert_eq!(family_of(5, 7), Tall);
    assert_eq!(family_of(100, 118), Tall);
    assert_eq!(family_of(100, 117), Square);
    assert_eq!(family_of(133, 100), Wide);
    assert_eq!(family_of(132, 100), Square);
}
