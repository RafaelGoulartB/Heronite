//! RetroAchievements' console ids (`rc_consoles.h`) for a game of the
//! Collection: from its platform id, else from the platform names of its
//! metadata. When neither says, the ROM's extension does (`hash::candidates`).

use crate::target::RetroTarget;

/// Collection platform id → RA consoles, the most likely first. A platform
/// can span several (Game Boy and Game Boy Color are one emulator).
const BY_PLATFORM_ID: &[(&str, &[u32])] = &[
    ("nintendo_nes", &[7, 81]),
    ("nintendo_super_nes", &[3]),
    ("nintendo_gameboy", &[4, 6]),
    ("nintendo_gameboyadvance", &[5]),
    ("nintendo_64", &[2]),
    ("nintendo_ds", &[18, 78]),
    ("nintendo_3ds", &[62]),
    ("nintendo_gamecube", &[16]),
    ("nintendo_wii", &[19]),
    ("nintendo_wiiu", &[20]),
    ("sega_genesis", &[1, 10, 9]),
    ("sega_master_system", &[11]),
    ("sega_gamegear", &[15]),
    ("sega_saturn", &[39]),
    ("sega_dreamcast", &[40]),
    ("sony_playstation", &[12]),
    ("sony_playstation2", &[21]),
    ("sony_playstation3", &[82]),
    ("sony_psp", &[41]),
    ("microsoft_xbox", &[22]),
    ("nec_turbografx_16", &[8, 76]),
    ("snk_neogeopocket", &[14]),
    ("bandai_wonderswan", &[53]),
    ("atari_lynx", &[13]),
    ("atari_2600", &[25]),
    ("arcade", &[27]),
];

/// Metadata platform names (normalized by [`normalize`]) → RA consoles.
const BY_NAME: &[(&str, &[u32])] = &[
    ("nes", &[7, 81]),
    ("nintendoentertainmentsystem", &[7, 81]),
    ("famicom", &[7, 81]),
    ("familycomputer", &[7, 81]),
    ("famicomdisksystem", &[81]),
    ("snes", &[3]),
    ("supernintendo", &[3]),
    ("supernintendoentertainmentsystem", &[3]),
    ("supernes", &[3]),
    ("superfamicom", &[3]),
    ("gameboy", &[4, 6]),
    ("gb", &[4, 6]),
    ("gameboycolor", &[6, 4]),
    ("gbc", &[6, 4]),
    ("gameboyadvance", &[5]),
    ("gba", &[5]),
    ("nintendo64", &[2]),
    ("n64", &[2]),
    ("nintendods", &[18, 78]),
    ("nds", &[18, 78]),
    ("nintendodsi", &[78, 18]),
    ("nintendo3ds", &[62]),
    ("3ds", &[62]),
    ("nintendogamecube", &[16]),
    ("gamecube", &[16]),
    ("ngc", &[16]),
    ("wii", &[19]),
    ("nintendowii", &[19]),
    ("wiiu", &[20]),
    ("nintendowiiu", &[20]),
    ("segamegadrive", &[1, 10, 9]),
    ("segamegadrivegenesis", &[1, 10, 9]),
    ("megadrive", &[1, 10, 9]),
    ("megadrivegenesis", &[1, 10, 9]),
    ("segagenesis", &[1, 10, 9]),
    ("genesis", &[1, 10, 9]),
    ("sega32x", &[10]),
    ("32x", &[10]),
    ("segacd", &[9]),
    ("megacd", &[9]),
    ("segamastersystem", &[11]),
    ("mastersystem", &[11]),
    ("segamastersystemmarkiii", &[11]),
    ("segagamegear", &[15]),
    ("gamegear", &[15]),
    ("segasaturn", &[39]),
    ("saturn", &[39]),
    ("segadreamcast", &[40]),
    ("dreamcast", &[40]),
    ("playstation", &[12]),
    ("sonyplaystation", &[12]),
    ("ps1", &[12]),
    ("psx", &[12]),
    ("playstation2", &[21]),
    ("sonyplaystation2", &[21]),
    ("ps2", &[21]),
    ("playstation3", &[82]),
    ("ps3", &[82]),
    ("playstationportable", &[41]),
    ("psp", &[41]),
    ("xbox", &[22]),
    ("microsoftxbox", &[22]),
    ("turbografx16", &[8, 76]),
    ("turbografx16pcengine", &[8, 76]),
    ("pcengine", &[8, 76]),
    ("pcenginecd", &[76]),
    ("turbografxcd", &[76]),
    ("turbografx16cd", &[76]),
    ("neogeopocket", &[14]),
    ("neogeopocketcolor", &[14]),
    ("wonderswan", &[53]),
    ("wonderswancolor", &[53]),
    ("atarilynx", &[13]),
    ("lynx", &[13]),
    ("atari2600", &[25]),
    ("atari7800", &[51]),
    ("atarijaguar", &[17]),
    ("jaguar", &[17]),
    ("virtualboy", &[28]),
    ("arcade", &[27]),
    ("neogeo", &[27]),
    ("neogeocd", &[56]),
    ("3do", &[43]),
    ("3doninteractivemultiplayer", &[43]),
    ("colecovision", &[44]),
    ("intellivision", &[45]),
    ("vectrex", &[46]),
    ("msx", &[29]),
    ("commodore64", &[30]),
    ("c64", &[30]),
    ("amiga", &[35]),
    ("pcfx", &[49]),
    ("pokemonmini", &[24]),
    ("dos", &[26]),
    ("msdos", &[26]),
];

/// RA's console names, for the "pick the game" dialog.
const NAMES: &[(u32, &str)] = &[
    (1, "Mega Drive"),
    (2, "Nintendo 64"),
    (3, "SNES"),
    (4, "Game Boy"),
    (5, "Game Boy Advance"),
    (6, "Game Boy Color"),
    (7, "NES"),
    (8, "PC Engine"),
    (9, "Sega CD"),
    (10, "32X"),
    (11, "Master System"),
    (12, "PlayStation"),
    (13, "Atari Lynx"),
    (14, "Neo Geo Pocket"),
    (15, "Game Gear"),
    (16, "GameCube"),
    (17, "Atari Jaguar"),
    (18, "Nintendo DS"),
    (19, "Wii"),
    (20, "Wii U"),
    (21, "PlayStation 2"),
    (22, "Xbox"),
    (24, "Pokemon Mini"),
    (25, "Atari 2600"),
    (26, "MS-DOS"),
    (27, "Arcade"),
    (28, "Virtual Boy"),
    (29, "MSX"),
    (30, "Commodore 64"),
    (35, "Amiga"),
    (39, "Saturn"),
    (40, "Dreamcast"),
    (41, "PlayStation Portable"),
    (43, "3DO"),
    (44, "ColecoVision"),
    (45, "Intellivision"),
    (46, "Vectrex"),
    (49, "PC-FX"),
    (51, "Atari 7800"),
    (53, "WonderSwan"),
    (56, "Neo Geo CD"),
    (62, "Nintendo 3DS"),
    (76, "PC Engine CD"),
    (78, "Nintendo DSi"),
    (81, "Famicom Disk System"),
    (82, "PlayStation 3"),
];

/// Lowercase letters and digits only: `PlayStation 2` → `playstation2`.
pub fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn for_platform_id(id: &str) -> &'static [u32] {
    BY_PLATFORM_ID.iter().find(|(k, _)| *k == id).map_or(&[], |(_, v)| v)
}

pub fn for_platform_name(name: &str) -> &'static [u32] {
    let name = normalize(name);
    BY_NAME.iter().find(|(k, _)| *k == name).map_or(&[], |(_, v)| v)
}

/// The consoles a ROM can be for, from what the Collection knows: the
/// platform id, else the metadata names. Empty when it says nothing.
pub fn for_target(target: &RetroTarget) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let known = target.platform_id.as_deref().map(for_platform_id).filter(|c| !c.is_empty());
    let named = || target.platforms.iter().flat_map(|n| for_platform_name(n).iter().copied());
    for c in known.map_or_else(|| named().collect::<Vec<_>>(), |c| c.to_vec()) {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

pub fn name(console: u32) -> String {
    NAMES.iter().find(|(id, _)| *id == console).map_or_else(|| format!("Console {console}"), |(_, n)| (*n).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(platform_id: Option<&str>, platforms: &[&str]) -> RetroTarget {
        RetroTarget {
            platform_id: platform_id.map(str::to_owned),
            platforms: platforms.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn platform_ids_map_to_ra_consoles() {
        assert_eq!(for_platform_id("nintendo_super_nes"), [3]);
        assert_eq!(for_platform_id("nintendo_gameboy"), [4, 6]);
        assert_eq!(for_platform_id("sony_playstation2"), [21]);
        assert_eq!(for_platform_id("nintendo_switch"), [] as [u32; 0]);
    }

    #[test]
    fn names_are_normalized() {
        assert_eq!(normalize("PlayStation 2"), "playstation2");
        assert_eq!(for_platform_name("PlayStation 2"), [21]);
        assert_eq!(for_platform_name("Super Nintendo Entertainment System"), [3]);
        assert_eq!(for_platform_name("Sega Mega Drive/Genesis"), [1, 10, 9]);
        assert_eq!(for_platform_name("PC (Microsoft Windows)"), [] as [u32; 0]);
    }

    #[test]
    fn the_platform_id_wins_over_the_names() {
        assert_eq!(for_target(&target(Some("nintendo_nes"), &["PlayStation 2"])), [7, 81]);
        assert_eq!(for_target(&target(None, &["PlayStation 2", "PlayStation 2", "PlayStation"])), [21, 12]);
        // "other" never reaches here (the target drops it) and an unknown id falls back to the names.
        assert_eq!(for_target(&target(Some("nintendo_switch"), &["Wii"])), [19]);
        assert!(for_target(&target(None, &[])).is_empty());
    }

    #[test]
    fn console_names() {
        assert_eq!(name(21), "PlayStation 2");
        assert_eq!(name(999), "Console 999");
    }
}
