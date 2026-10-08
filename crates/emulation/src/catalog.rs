//! Full emulator catalog (port of `emulation/catalog.ts`).
//!
//! `heronite-launch` already owns the launch-relevant part (argument
//! templates, working dir, stop pattern, flatpak id); those are read from it
//! rather than duplicated. This table adds what detection, scanning and the
//! emulator manager need: names, binaries, extensions, platforms, cores.

use std::sync::OnceLock;

use heronite_launch::emulators as launch_catalog;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorDefinitionProfile {
    pub id: String,
    pub name: String,
    pub platform_id: String,
    pub platform_name: String,
    pub extensions: Vec<String>,
    pub arguments: String,
    /// RetroArch core file stem (`snes9x_libretro`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_indicator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_extensions: Option<Vec<String>>,
    /// 'emulator' | 'rom'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorDefinition {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    pub binaries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flatpak_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_dir_hints: Option<Vec<String>>,
    pub profiles: Vec<EmulatorDefinitionProfile>,
}

struct P {
    id: &'static str,
    name: &'static str,
    platform_id: &'static str,
    platform_name: &'static str,
    ext: &'static [&'static str],
    /// Append `zip`, `7z`.
    cart: bool,
    core: Option<&'static str>,
    folder_indicator: Option<&'static str>,
    playlists: Option<&'static [&'static str]>,
}

const fn p(
    id: &'static str,
    name: &'static str,
    platform_id: &'static str,
    platform_name: &'static str,
    ext: &'static [&'static str],
) -> P {
    P {
        id,
        name,
        platform_id,
        platform_name,
        ext,
        cart: false,
        core: None,
        folder_indicator: None,
        playlists: None,
    }
}

impl P {
    const fn cart(mut self) -> Self {
        self.cart = true;
        self
    }
    const fn playlists(mut self, v: &'static [&'static str]) -> Self {
        self.playlists = Some(v);
        self
    }
    const fn indicator(mut self, v: &'static str) -> Self {
        self.folder_indicator = Some(v);
        self
    }
}

const fn ra(
    id: &'static str,
    name: &'static str,
    platform_id: &'static str,
    platform_name: &'static str,
    core: &'static str,
    ext: &'static [&'static str],
) -> P {
    P {
        core: Some(core),
        ..p(id, name, platform_id, platform_name, ext)
    }
}

struct D {
    id: &'static str,
    name: &'static str,
    website: Option<&'static str>,
    binaries: &'static [&'static str],
    core_dir_hints: Option<&'static [&'static str]>,
    profiles: &'static [P],
}

static TABLE: &[D] = &[
    D {
        id: "dolphin",
        name: "Dolphin",
        website: Some("https://dolphin-emu.org/"),
        binaries: &["dolphin-emu", "dolphin"],
        core_dir_hints: None,
        profiles: &[
            p(
                "gamecube",
                "Nintendo GameCube",
                "nintendo_gamecube",
                "Nintendo GameCube",
                &[
                    "elf", "dol", "gcm", "tgc", "ciso", "gcz", "iso", "wad", "rvz", "wia", "m3u",
                ],
            )
            .playlists(&["m3u"]),
            p(
                "wii",
                "Nintendo Wii",
                "nintendo_wii",
                "Nintendo Wii",
                &[
                    "elf", "dol", "tgc", "wbfs", "ciso", "gcz", "iso", "wad", "rvz", "wia", "m3u",
                ],
            )
            .playlists(&["m3u"]),
        ],
    },
    D {
        id: "pcsx2",
        name: "PCSX2",
        website: Some("https://pcsx2.net/"),
        binaries: &["pcsx2-qt", "pcsx2"],
        core_dir_hints: None,
        profiles: &[p(
            "ps2",
            "PlayStation 2",
            "sony_playstation2",
            "PlayStation 2",
            &["iso", "bin", "mdf", "img", "chd", "cso", "gz", "m3u"],
        )
        .playlists(&["m3u", "cue"])],
    },
    D {
        id: "duckstation",
        name: "DuckStation",
        website: Some("https://www.duckstation.org/"),
        binaries: &["duckstation-qt", "duckstation"],
        core_dir_hints: None,
        profiles: &[p(
            "ps1",
            "PlayStation",
            "sony_playstation",
            "PlayStation",
            &["iso", "bin", "img", "chd", "pbp", "ecm", "m3u", "cue"],
        )
        .playlists(&["cue", "m3u"])],
    },
    D {
        id: "ppsspp",
        name: "PPSSPP",
        website: Some("https://www.ppsspp.org/"),
        binaries: &["PPSSPPSDL", "ppsspp", "ppsspp-sdl"],
        core_dir_hints: None,
        profiles: &[p(
            "psp",
            "PlayStation Portable",
            "sony_psp",
            "PlayStation Portable",
            &["iso", "cso", "pbp", "prx", "elf", "chd"],
        )],
    },
    D {
        id: "rpcs3",
        name: "RPCS3",
        website: Some("https://rpcs3.net/"),
        binaries: &["rpcs3"],
        core_dir_hints: None,
        profiles: &[p(
            "ps3",
            "PlayStation 3",
            "sony_playstation3",
            "PlayStation 3",
            &["iso", "pkg"],
        )
        .indicator("EBOOT.BIN")],
    },
    D {
        id: "cemu",
        name: "Cemu",
        website: Some("https://cemu.info/"),
        binaries: &["cemu", "Cemu"],
        core_dir_hints: None,
        profiles: &[p(
            "wiiu",
            "Nintendo Wii U",
            "nintendo_wiiu",
            "Nintendo Wii U",
            &["wud", "wux", "iso", "wad", "rpx", "elf"],
        )],
    },
    D {
        id: "ryujinx",
        name: "Ryujinx",
        website: Some("https://ryujinx.org/"),
        binaries: &["ryujinx", "Ryujinx"],
        core_dir_hints: None,
        profiles: &[p(
            "switch",
            "Nintendo Switch",
            "nintendo_switch",
            "Nintendo Switch",
            &["nsp", "xci", "nca", "nro", "nso"],
        )],
    },
    D {
        id: "xemu",
        name: "Xemu",
        website: Some("https://xemu.app/"),
        binaries: &["xemu"],
        core_dir_hints: None,
        profiles: &[p("xbox", "Xbox", "microsoft_xbox", "Xbox", &["iso", "xiso"])],
    },
    D {
        id: "melonds",
        name: "melonDS",
        website: Some("https://melonds.kuribo64.net/"),
        binaries: &["melonDS", "melonds"],
        core_dir_hints: None,
        profiles: &[p("nds", "Nintendo DS", "nintendo_ds", "Nintendo DS", &["nds", "dsi", "ids"]).cart()],
    },
    D {
        id: "mgba",
        name: "mGBA",
        website: Some("https://mgba.io/"),
        binaries: &["mgba-qt", "mgba"],
        core_dir_hints: None,
        profiles: &[
            p(
                "gba",
                "Game Boy Advance",
                "nintendo_gameboyadvance",
                "Game Boy Advance",
                &["gba", "agb", "mb", "bin"],
            )
            .cart(),
            p(
                "gb",
                "Game Boy / Color",
                "nintendo_gameboy",
                "Game Boy",
                &["gb", "gbc", "sgb", "bin"],
            )
            .cart(),
        ],
    },
    D {
        id: "flycast",
        name: "Flycast",
        website: Some("https://github.com/flyinghead/flycast"),
        binaries: &["flycast"],
        core_dir_hints: None,
        profiles: &[p(
            "dreamcast",
            "Dreamcast",
            "sega_dreamcast",
            "Dreamcast",
            &["gdi", "cdi", "chd", "cue", "iso", "m3u"],
        )
        .playlists(&["gdi", "cue", "m3u"])],
    },
    D {
        id: "mame",
        name: "MAME",
        website: Some("https://www.mamedev.org/"),
        binaries: &["mame", "mame-mame"],
        core_dir_hints: None,
        profiles: &[p("arcade", "Arcade", "arcade", "Arcade", &["zip", "7z"])],
    },
    D {
        id: "scummvm",
        name: "ScummVM",
        website: Some("https://www.scummvm.org/"),
        binaries: &["scummvm"],
        core_dir_hints: None,
        profiles: &[p("scumm", "ScummVM", "pc", "PC", &["scummvm"])],
    },
    D {
        id: "vita3k",
        name: "Vita3K",
        website: Some("https://vita3k.org/"),
        binaries: &["Vita3K", "vita3k"],
        core_dir_hints: None,
        profiles: &[p("vita", "PlayStation Vita", "sony_psvita", "PlayStation Vita", &["vpk"])
            .indicator("eboot.bin")],
    },
    D {
        id: "shadps4",
        name: "shadPS4",
        website: Some("https://shadps4.net/"),
        binaries: &["shadps4", "ShadPS4"],
        core_dir_hints: None,
        profiles: &[p("ps4", "PlayStation 4", "sony_playstation4", "PlayStation 4", &["pkg"])
            .indicator("eboot.bin")],
    },
    D {
        id: "azahar",
        name: "Azahar",
        website: Some("https://azahar-emu.org/"),
        binaries: &["azahar", "citra", "citra-qt"],
        core_dir_hints: None,
        profiles: &[p(
            "3ds",
            "Nintendo 3DS",
            "nintendo_3ds",
            "Nintendo 3DS",
            &["3ds", "3dsx", "cci", "cxi", "app", "cia"],
        )
        .cart()],
    },
    D {
        id: "retroarch",
        name: "RetroArch",
        website: Some("https://www.retroarch.com/"),
        binaries: &["retroarch"],
        core_dir_hints: Some(&[
            "/usr/lib/libretro",
            "/usr/lib64/libretro",
            "/usr/lib/x86_64-linux-gnu/libretro",
            "/usr/share/libretro/cores",
        ]),
        profiles: &[
            ra("nes", "NES", "nintendo_nes", "Nintendo Entertainment System", "nestopia_libretro", &["nes", "fds", "unf", "unif"]).cart(),
            ra("snes", "SNES", "nintendo_super_nes", "Super Nintendo", "snes9x_libretro", &["sfc", "smc", "swc", "fig", "bs"]).cart(),
            ra("gb", "Game Boy", "nintendo_gameboy", "Game Boy", "gambatte_libretro", &["gb", "gbc", "sgb"]).cart(),
            ra("gba", "Game Boy Advance", "nintendo_gameboyadvance", "Game Boy Advance", "mgba_libretro", &["gba", "agb", "mb"]).cart(),
            ra("n64", "Nintendo 64", "nintendo_64", "Nintendo 64", "mupen64plus_next_libretro", &["n64", "v64", "z64"]).cart(),
            ra("nds", "Nintendo DS", "nintendo_ds", "Nintendo DS", "melonds_libretro", &["nds", "dsi", "ids"]).cart(),
            ra("md", "Mega Drive / Genesis", "sega_genesis", "Mega Drive", "genesis_plus_gx_libretro", &["md", "gen", "smd", "bin"]).cart(),
            ra("ms", "Master System", "sega_master_system", "Master System", "genesis_plus_gx_libretro", &["sms", "bin"]).cart(),
            ra("gg", "Game Gear", "sega_gamegear", "Game Gear", "genesis_plus_gx_libretro", &["gg", "bin"]).cart(),
            ra("saturn", "Saturn", "sega_saturn", "Sega Saturn", "mednafen_saturn_libretro", &["cue", "chd", "iso", "m3u"]),
            ra("dc", "Dreamcast", "sega_dreamcast", "Dreamcast", "flycast_libretro", &["gdi", "cdi", "chd", "cue", "m3u"]),
            ra("ps1", "PlayStation", "sony_playstation", "PlayStation", "pcsx_rearmed_libretro", &["cue", "bin", "iso", "chd", "pbp", "m3u"]),
            ra("psp", "PSP", "sony_psp", "PlayStation Portable", "ppsspp_libretro", &["iso", "cso", "pbp", "chd"]),
            ra("pce", "PC Engine", "nec_turbografx_16", "PC Engine", "mednafen_pce_fast_libretro", &["pce", "cue", "chd", "sgx"]).cart(),
            ra("ngp", "Neo Geo Pocket", "snk_neogeopocket", "Neo Geo Pocket", "mednafen_ngp_libretro", &["ngp", "ngc", "ngpc"]).cart(),
            ra("wswan", "WonderSwan", "bandai_wonderswan", "WonderSwan", "mednafen_wswan_libretro", &["ws", "wsc"]).cart(),
            ra("lynx", "Atari Lynx", "atari_lynx", "Atari Lynx", "mednafen_lynx_libretro", &["lnx", "lyx"]).cart(),
            ra("a2600", "Atari 2600", "atari_2600", "Atari 2600", "stella_libretro", &["a26", "bin"]).cart(),
            ra("arcade", "Arcade", "arcade", "Arcade", "fbneo_libretro", &["zip", "7z"]),
            ra("mame", "MAME", "arcade", "Arcade", "mame_libretro", &["zip", "7z"]),
        ],
    },
    D {
        id: "custom",
        name: "Custom",
        website: None,
        binaries: &[],
        core_dir_hints: None,
        profiles: &[p("custom", "Custom", "other", "Other", &[])],
    },
];

fn build() -> Vec<EmulatorDefinition> {
    TABLE
        .iter()
        .map(|d| {
            let launch = launch_catalog::emulator_definition(d.id);
            EmulatorDefinition {
                id: d.id.into(),
                name: d.name.into(),
                website: d.website.map(Into::into),
                binaries: d.binaries.iter().map(|s| s.to_string()).collect(),
                flatpak_id: launch.and_then(|l| l.flatpak_id).map(Into::into),
                stop_pattern: launch.and_then(|l| l.stop_pattern).map(Into::into),
                core_dir_hints: d
                    .core_dir_hints
                    .map(|h| h.iter().map(|s| s.to_string()).collect()),
                profiles: d
                    .profiles
                    .iter()
                    .map(|pr| {
                        let lp = launch_catalog::definition_profile(d.id, pr.id);
                        let mut extensions: Vec<String> =
                            pr.ext.iter().map(|s| s.to_string()).collect();
                        if pr.cart {
                            extensions.push("zip".into());
                            extensions.push("7z".into());
                        }
                        EmulatorDefinitionProfile {
                            id: pr.id.into(),
                            name: pr.name.into(),
                            platform_id: pr.platform_id.into(),
                            platform_name: pr.platform_name.into(),
                            extensions,
                            arguments: lp
                                .map(|l| l.arguments.to_string())
                                .unwrap_or_else(|| "{ImagePath}".into()),
                            core: pr.core.map(Into::into),
                            folder_indicator: pr.folder_indicator.map(Into::into),
                            playlist_extensions: pr
                                .playlists
                                .map(|v| v.iter().map(|s| s.to_string()).collect()),
                            working_dir: lp.and_then(|l| l.working_dir).map(Into::into),
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

/// `EMULATOR_DEFINITIONS` (including the `custom` pseudo emulator).
pub fn emulator_definitions() -> &'static [EmulatorDefinition] {
    static DEFS: OnceLock<Vec<EmulatorDefinition>> = OnceLock::new();
    DEFS.get_or_init(build)
}

/// `getEmulatorDefinition`.
pub fn emulator_definition(id: &str) -> Option<&'static EmulatorDefinition> {
    emulator_definitions().iter().find(|d| d.id == id)
}

/// `getDefinitionProfile`.
pub fn definition_profile(
    definition_id: &str,
    profile_id: &str,
) -> Option<&'static EmulatorDefinitionProfile> {
    emulator_definition(definition_id)?
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
}

/// `guessDefinitionId`: which catalog emulator a free-form name refers to.
pub fn guess_definition_id(name: Option<&str>) -> String {
    let normalized: String = name
        .unwrap_or("")
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if normalized.is_empty() {
        return "custom".into();
    }
    const ALIASES: &[(&str, &str)] = &[
        ("retroarch", "retroarch"),
        ("dolphin", "dolphin"),
        ("pcsx2", "pcsx2"),
        ("duckstation", "duckstation"),
        ("ppsspp", "ppsspp"),
        ("rpcs3", "rpcs3"),
        ("cemu", "cemu"),
        ("ryujinx", "ryujinx"),
        ("xemu", "xemu"),
        ("melonds", "melonds"),
        ("mgba", "mgba"),
        ("flycast", "flycast"),
        ("mame", "mame"),
        ("scummvm", "scummvm"),
        ("vita3k", "vita3k"),
        ("shadps4", "shadps4"),
        ("azahar", "azahar"),
        ("citra", "azahar"),
    ];
    for (needle, id) in ALIASES {
        if normalized.contains(needle) {
            return (*id).into();
        }
    }
    "custom".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_launch_catalog() {
        let ra = definition_profile("retroarch", "snes").unwrap();
        assert_eq!(ra.arguments, "-L {CorePath} {ImagePath}");
        assert_eq!(ra.core.as_deref(), Some("snes9x_libretro"));
        assert_eq!(ra.extensions, ["sfc", "smc", "swc", "fig", "bs", "zip", "7z"]);
        assert_eq!(
            definition_profile("mame", "arcade").unwrap().working_dir.as_deref(),
            Some("rom")
        );
        let d = emulator_definition("pcsx2").unwrap();
        assert_eq!(d.flatpak_id.as_deref(), Some("net.pcsx2.PCSX2"));
        assert_eq!(d.stop_pattern.as_deref(), Some("pcsx2"));
        assert_eq!(
            definition_profile("rpcs3", "ps3").unwrap().folder_indicator.as_deref(),
            Some("EBOOT.BIN")
        );
        assert!(emulator_definition("custom").unwrap().flatpak_id.is_none());
    }

    #[test]
    fn every_profile_exists_in_the_launch_catalog() {
        for d in emulator_definitions() {
            let l = launch_catalog::emulator_definition(&d.id)
                .unwrap_or_else(|| panic!("{} missing in launch catalog", d.id));
            for pr in &d.profiles {
                assert!(
                    l.profiles.iter().any(|lp| lp.id == pr.id),
                    "{}:{} missing in launch catalog",
                    d.id,
                    pr.id
                );
            }
        }
        assert_eq!(emulator_definitions().len(), 18);
    }

    #[test]
    fn guesses_ids() {
        assert_eq!(guess_definition_id(Some("PCSX2 (AppImage)")), "pcsx2");
        assert_eq!(guess_definition_id(Some("Citra Nightly")), "azahar");
        assert_eq!(guess_definition_id(Some("org.libretro.RetroArch")), "retroarch");
        assert_eq!(guess_definition_id(Some("whatever")), "custom");
        assert_eq!(guess_definition_id(None), "custom");
    }
}
