//! The platforms of emulated games: the app's platform ids (the emulator
//! catalog's and Playnite's `platformId`, `nintendo_super_nes`), how the
//! Collection names them, and how each database a source reads names them.
//!
//! One row per platform. A source of emulated games adds a column (a
//! [`Database`] variant and its names in each row); [`Platform::names_in`]
//! gives them. [`resolve`] works out the platform of a Collection game whose
//! entry does not say it (games brought in from Playnite or added by hand
//! have no `platformId`, or `other`).

use std::path::Path;

use heronite_data::models::LocalGameMeta;

/// A database a source matches platforms against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    /// The LaunchBox Games Database (`<Platform>` of `Metadata.xml`).
    LaunchBox,
    /// libretro-thumbnails (a folder of `thumbnails.libretro.com`).
    Libretro,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Platform {
    /// Stable id, the overlay's `platformId`.
    pub id: &'static str,
    /// As the Collection names it (IGDB's names, which the metadata uses).
    pub name: &'static str,
    /// What also names it, as [`normalize`] leaves it: folder names of ROM
    /// collections (EmuDeck, RetroDECK, ES-DE), names of emulators made for
    /// it alone, other databases' names.
    aliases: &'static [&'static str],
    /// ROM file extensions only this platform uses (lower case).
    extensions: &'static [&'static str],
    /// LaunchBox's platforms, the main one first.
    launchbox: &'static [&'static str],
    /// libretro-thumbnails' systems (folder names), the main one first.
    /// Empty: no system, or one with too few pictures to be worth asking
    /// (PlayStation 3 and 4, Vita, Xbox 360 have under 70), or one named by
    /// something other than the dump (arcade: MAME's descriptions; DOS and
    /// ScummVM: titles, while the game's file is an executable).
    libretro: &'static [&'static str],
}

impl Platform {
    /// The names `db` gives this platform, the main one first (empty: it
    /// does not have it).
    pub fn names_in(&self, db: Database) -> &'static [&'static str] {
        match db {
            Database::LaunchBox => self.launchbox,
            Database::Libretro => self.libretro,
        }
    }
}

const fn p(
    id: &'static str,
    name: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    launchbox: &'static [&'static str],
    libretro: &'static [&'static str],
) -> Platform {
    Platform { id, name, aliases, extensions, launchbox, libretro }
}

/// Every platform the app knows. An extension is listed only for the one
/// platform that uses it (`iso`, `bin`, `chd`, `cue` say nothing).
pub static PLATFORMS: &[Platform] = &[
    p(
        "nintendo_nes",
        "Nintendo Entertainment System",
        &["nes", "famicom", "fc", "familycomputer", "nintendoentertainmentsystem", "nintendones", "nestopia", "fceux", "fceumm"],
        &["nes", "unf", "unif", "fds"],
        &["Nintendo Entertainment System", "Nintendo Famicom Disk System"],
        &["Nintendo - Nintendo Entertainment System", "Nintendo - Family Computer Disk System"],
    ),
    p(
        "nintendo_super_nes",
        "Super Nintendo Entertainment System",
        &["snes", "sfc", "supernintendo", "superfamicom", "supernes", "supernintendoentertainmentsystem", "snes9x", "bsnes", "zsnes"],
        &["sfc", "smc", "swc", "fig", "bs"],
        &["Super Nintendo Entertainment System", "Nintendo Satellaview"],
        &["Nintendo - Super Nintendo Entertainment System", "Nintendo - Satellaview"],
    ),
    p(
        "nintendo_gameboy",
        "Game Boy",
        &["gb", "gbc", "gameboy", "gameboycolor", "nintendogameboy", "nintendogameboycolor", "gambatte", "sameboy"],
        &["gb", "gbc", "sgb"],
        &["Nintendo Game Boy", "Nintendo Game Boy Color"],
        &["Nintendo - Game Boy", "Nintendo - Game Boy Color"],
    ),
    p(
        "nintendo_gameboyadvance",
        "Game Boy Advance",
        &["gba", "gameboyadvance", "nintendogameboyadvance"],
        &["gba", "agb"],
        &["Nintendo Game Boy Advance"],
        &["Nintendo - Game Boy Advance"],
    ),
    p(
        "nintendo_64",
        "Nintendo 64",
        &["n64", "nintendo64", "mupen64plus", "m64py", "gopher64", "project64", "simple64"],
        &["n64", "v64", "z64"],
        &["Nintendo 64", "Nintendo 64DD"],
        &["Nintendo - Nintendo 64"],
    ),
    p(
        "nintendo_ds",
        "Nintendo DS",
        &["nds", "ds", "nintendods", "nintendodsi", "dsi", "melonds", "desmume", "drastic"],
        &["nds", "dsi", "ids", "srl"],
        &["Nintendo DS"],
        &["Nintendo - Nintendo DS", "Nintendo - Nintendo DSi"],
    ),
    p(
        "nintendo_3ds",
        "Nintendo 3DS",
        &["3ds", "n3ds", "nintendo3ds", "azahar", "citra", "lime3ds"],
        &["3ds", "3dsx", "cci", "cxi", "cia"],
        &["Nintendo 3DS"],
        &["Nintendo - Nintendo 3DS"],
    ),
    p(
        "nintendo_gamecube",
        "Nintendo GameCube",
        &["gc", "gamecube", "ngc", "nintendogamecube"],
        &["gcm"],
        &["Nintendo GameCube"],
        &["Nintendo - GameCube"],
    ),
    p("nintendo_wii", "Wii", &["wii", "nintendowii"], &["wbfs"], &["Nintendo Wii"], &["Nintendo - Wii"]),
    p("nintendo_wiiu", "Wii U", &["wiiu", "nintendowiiu", "cemu"], &["wud", "wux", "rpx"], &["Nintendo Wii U"], &["Nintendo - Wii U"]),
    p(
        "nintendo_switch",
        "Nintendo Switch",
        &["switch", "nintendoswitch", "ryujinx", "yuzu", "suyu", "sudachi", "citron", "eden"],
        &["nsp", "xci", "nca", "nro", "nso"],
        &["Nintendo Switch"],
        &[],
    ),
    p(
        "nintendo_virtualboy",
        "Virtual Boy",
        &["virtualboy", "vb", "nintendovirtualboy"],
        &["vb", "vboy"],
        &["Nintendo Virtual Boy"],
        &["Nintendo - Virtual Boy"],
    ),
    p(
        "sega_genesis",
        "Sega Mega Drive/Genesis",
        &["genesis", "megadrive", "md", "segagenesis", "segamegadrive", "segamegadrivegenesis", "megadrivegenesis", "blastem"],
        &["md", "gen", "smd"],
        &["Sega Genesis"],
        &["Sega - Mega Drive - Genesis"],
    ),
    p("sega_32x", "Sega 32X", &["32x", "sega32x"], &["32x"], &["Sega 32X"], &["Sega - 32X"]),
    p("sega_cd", "Sega CD", &["segacd", "megacd", "segamegacd"], &[], &["Sega CD"], &["Sega - Mega-CD - Sega CD"]),
    p(
        "sega_master_system",
        "Sega Master System/Mark III",
        &["mastersystem", "sms", "segamastersystem", "segamastersystemmarkiii", "markiii"],
        &["sms"],
        &["Sega Master System"],
        &["Sega - Master System - Mark III"],
    ),
    p("sega_gamegear", "Sega Game Gear", &["gamegear", "gg", "segagamegear"], &["gg"], &["Sega Game Gear"], &["Sega - Game Gear"]),
    p(
        "sega_saturn",
        "Sega Saturn",
        &["saturn", "segasaturn", "yabause", "kronos", "yabasanshiro"],
        &[],
        &["Sega Saturn"],
        &["Sega - Saturn"],
    ),
    p(
        "sega_dreamcast",
        "Dreamcast",
        &["dreamcast", "dc", "segadreamcast", "flycast", "redream", "reicast", "demul"],
        &["gdi", "cdi"],
        &["Sega Dreamcast"],
        &["Sega - Dreamcast"],
    ),
    p("sega_sg1000", "SG-1000", &["sg1000", "segasg1000"], &["sg"], &["Sega SG-1000"], &["Sega - SG-1000"]),
    p(
        "sony_playstation",
        "PlayStation",
        &["psx", "ps1", "playstation", "sonyplaystation", "playstation1", "duckstation", "epsxe", "pcsxrearmed", "swanstation"],
        &[],
        &["Sony Playstation"],
        &["Sony - PlayStation"],
    ),
    p(
        "sony_playstation2",
        "PlayStation 2",
        &["ps2", "playstation2", "sonyplaystation2", "pcsx2", "aethersx2", "nethersx2"],
        &[],
        &["Sony Playstation 2"],
        &["Sony - PlayStation 2"],
    ),
    p("sony_playstation3", "PlayStation 3", &["ps3", "playstation3", "sonyplaystation3", "rpcs3"], &[], &["Sony Playstation 3"], &[]),
    p("sony_playstation4", "PlayStation 4", &["ps4", "playstation4", "sonyplaystation4", "shadps4"], &[], &["Sony Playstation 4"], &[]),
    p(
        "sony_psp",
        "PlayStation Portable",
        &["psp", "playstationportable", "sonypsp", "sonyplaystationportable", "ppsspp"],
        &["cso"],
        &["Sony PSP", "Sony PSP Minis"],
        &["Sony - PlayStation Portable"],
    ),
    p(
        "sony_psvita",
        "PlayStation Vita",
        &["psvita", "vita", "playstationvita", "sonyplaystationvita", "vita3k"],
        &["vpk"],
        &["Sony Playstation Vita"],
        &[],
    ),
    p("microsoft_xbox", "Xbox", &["xbox", "microsoftxbox", "xemu", "xqemu"], &["xiso"], &["Microsoft Xbox"], &["Microsoft - Xbox"]),
    p("microsoft_xbox360", "Xbox 360", &["xbox360", "microsoftxbox360", "xenia"], &["xex"], &["Microsoft Xbox 360"], &[]),
    p(
        "nec_turbografx_16",
        "TurboGrafx-16/PC Engine",
        &["pcengine", "tg16", "turbografx16", "pce", "turbografx16pcengine", "necturbografx16", "pcenginecd", "turbografxcd", "tgcd"],
        &["pce", "sgx"],
        &["NEC TurboGrafx-16", "NEC TurboGrafx-CD", "PC Engine SuperGrafx"],
        &["NEC - PC Engine - TurboGrafx 16", "NEC - PC Engine CD - TurboGrafx-CD", "NEC - PC Engine SuperGrafx"],
    ),
    p("nec_pcfx", "PC-FX", &["pcfx", "necpcfx"], &[], &["NEC PC-FX"], &["NEC - PC-FX"]),
    p(
        "snk_neogeopocket",
        "Neo Geo Pocket",
        &["ngp", "ngpc", "neogeopocket", "neogeopocketcolor", "snkneogeopocket"],
        &["ngp", "ngc", "ngpc"],
        &["SNK Neo Geo Pocket", "SNK Neo Geo Pocket Color"],
        &["SNK - Neo Geo Pocket", "SNK - Neo Geo Pocket Color"],
    ),
    p("snk_neogeo_cd", "Neo Geo CD", &["neogeocd", "snkneogeocd"], &[], &["SNK Neo Geo CD"], &["SNK - Neo Geo CD"]),
    p(
        "bandai_wonderswan",
        "WonderSwan",
        &["wonderswan", "wonderswancolor", "wswan", "wswanc", "ws", "wsc", "bandaiwonderswan"],
        &["ws", "wsc"],
        &["WonderSwan", "WonderSwan Color"],
        &["Bandai - WonderSwan", "Bandai - WonderSwan Color"],
    ),
    p("atari_2600", "Atari 2600", &["atari2600", "a2600", "stella"], &["a26"], &["Atari 2600"], &["Atari - 2600"]),
    p("atari_5200", "Atari 5200", &["atari5200", "a5200"], &["a52"], &["Atari 5200"], &["Atari - 5200"]),
    p("atari_7800", "Atari 7800", &["atari7800", "a7800"], &["a78"], &["Atari 7800"], &["Atari - 7800"]),
    p("atari_lynx", "Atari Lynx", &["lynx", "atarilynx", "handy"], &["lnx", "lyx"], &["Atari Lynx"], &["Atari - Lynx"]),
    p(
        "atari_jaguar",
        "Atari Jaguar",
        &["jaguar", "atarijaguar", "jaguarcd"],
        &["j64", "jag"],
        &["Atari Jaguar", "Atari Jaguar CD"],
        &["Atari - Jaguar"],
    ),
    p("atari_st", "Atari ST/STE", &["atarist", "hatari"], &["st", "stx", "msa"], &["Atari ST"], &["Atari - ST"]),
    p("coleco_vision", "ColecoVision", &["colecovision", "coleco"], &["col"], &["ColecoVision"], &["Coleco - ColecoVision"]),
    p(
        "mattel_intellivision",
        "Intellivision",
        &["intellivision", "mattelintellivision"],
        &["int"],
        &["Mattel Intellivision"],
        &["Mattel - Intellivision"],
    ),
    p("vectrex", "Vectrex", &["vectrex", "gcevectrex"], &["vec"], &["GCE Vectrex"], &["GCE - Vectrex"]),
    p(
        "3do",
        "3DO Interactive Multiplayer",
        &["3do", "panasonic3do", "3dointeractivemultiplayer"],
        &[],
        &["3DO Interactive Multiplayer"],
        &["The 3DO Company - 3DO"],
    ),
    p(
        "arcade",
        "Arcade",
        &["arcade", "mame", "fbneo", "fba", "finalburn", "neogeo", "cps1", "cps2", "cps3"],
        &[],
        &["Arcade", "SNK Neo Geo AES", "SNK Neo Geo MVS"],
        &[],
    ),
    p(
        "commodore_64",
        "Commodore C64/128/MAX",
        &["c64", "commodore64", "vice"],
        &["d64", "t64", "prg", "crt"],
        &["Commodore 64", "Commodore 128"],
        &["Commodore - 64"],
    ),
    p(
        "commodore_amiga",
        "Amiga",
        &["amiga", "commodoreamiga", "amigacd32", "cd32", "fsuae", "winuae"],
        &["adf", "adz", "hdf"],
        &["Commodore Amiga", "Commodore Amiga CD32"],
        &["Commodore - Amiga", "Commodore - CD32"],
    ),
    p("amstrad_cpc", "Amstrad CPC", &["amstradcpc", "cpc"], &[], &["Amstrad CPC"], &["Amstrad - CPC"]),
    p(
        "sinclair_zxspectrum",
        "ZX Spectrum",
        &["zxspectrum", "spectrum", "sinclairzxspectrum", "zx"],
        &["tzx", "z80", "sna"],
        &["Sinclair ZX Spectrum"],
        &["Sinclair - ZX Spectrum"],
    ),
    p(
        "microsoft_msx",
        "MSX",
        &["msx", "msx2", "microsoftmsx"],
        &["mx1", "mx2"],
        &["Microsoft MSX", "Microsoft MSX2", "Microsoft MSX2+"],
        &["Microsoft - MSX", "Microsoft - MSX2"],
    ),
    p("pc_dos", "DOS", &["dos", "msdos", "pcdos", "dosbox"], &[], &["MS-DOS"], &[]),
    p(
        "pc_windows",
        "PC (Microsoft Windows)",
        &["windows", "pcwindows", "pcmicrosoftwindows", "win"],
        &[],
        &["Windows", "Windows 3.X"],
        &[],
    ),
    // The emulator catalog's ScummVM profile.
    p("pc", "PC", &["scummvm"], &["scummvm"], &["ScummVM", "MS-DOS", "Windows"], &[]),
];

/// LaunchBox's platforms of a game that is not emulated (a PC game).
pub const LAUNCHBOX_PC: &[&str] = &["Windows", "MS-DOS", "Windows 3.X"];

/// Lower-case letters and digits only: `PS2_Games` → `ps2games`.
pub fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn by_id(id: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.id == id)
}

/// The platform `text` names: its id, its name or an alias (any case,
/// spaces and punctuation ignored).
pub fn by_name(text: &str) -> Option<&'static Platform> {
    let key = normalize(text);
    if key.is_empty() {
        return None;
    }
    PLATFORMS.iter().find(|p| normalize(p.id) == key || normalize(p.name) == key || p.aliases.contains(&key.as_str()))
}

/// The platform whose name in `db` is `name` (as the database writes it).
pub fn from_database(db: Database, name: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.names_in(db).first() == Some(&name)).or_else(|| PLATFORMS.iter().find(|p| p.names_in(db).contains(&name)))
}

/// The one platform that uses files with extension `ext`.
pub fn by_extension(ext: &str) -> Option<&'static Platform> {
    let ext = ext.to_ascii_lowercase();
    let mut found = PLATFORMS.iter().filter(|p| p.extensions.contains(&ext.as_str()));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// A folder of a ROM collection that names a platform: `PS2`, `snes`,
/// `PS3_Games`, `Mega Drive ROMs`.
fn by_folder(name: &str) -> Option<&'static Platform> {
    let key = normalize(name);
    let trimmed = ["games", "roms", "isos", "rom", "iso", "collection"].iter().find_map(|s| key.strip_suffix(s)).unwrap_or(&key);
    by_name(&key).or_else(|| (!trimmed.is_empty()).then(|| by_name(trimmed)).flatten())
}

/// The folders and file of a path written on any system (`/` or `\`).
fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
}

/// A ROM path of the entry with `{InstallDir}` replaced by its folder.
fn expand(meta: &LocalGameMeta, path: &str) -> String {
    match meta.windows_install_directory.as_deref() {
        Some(dir) if path.contains("{InstallDir}") => path.replace("{InstallDir}", dir.trim_end_matches(['/', '\\'])),
        _ => path.to_owned(),
    }
}

/// The ROM a game plays: the one picked, else the first listed.
fn rom_path(meta: &LocalGameMeta) -> Option<String> {
    let picked = meta.selected_rom_path.as_deref().filter(|p| !p.trim().is_empty());
    let first = || meta.roms.as_ref()?.iter().map(|r| r.path.as_str()).find(|p| !p.trim().is_empty());
    picked.or_else(first).map(|p| expand(meta, p))
}

/// Files that start a game without naming it: a PlayStation 3 or Vita
/// game's `EBOOT.BIN`, a Wii U game's `.rpx`, an Xbox game's `default.xbe`.
fn is_boot_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(lower.as_str(), "eboot.bin" | "default.xbe" | "default.xex" | "param.sfo") || matches!(ext, "rpx" | "elf" | "dol")
}

/// Folders inside a game's dump that do not name it.
fn is_generic_folder(name: &str) -> bool {
    let key = normalize(name);
    matches!(
        key.as_str(),
        "usrdir"
            | "ps3game"
            | "code"
            | "content"
            | "meta"
            | "game"
            | "games"
            | "rom"
            | "roms"
            | "iso"
            | "isos"
            | "bin"
            | "data"
            | "installdir"
    ) || key.len() <= 2
        || by_folder(name).is_some()
}

/// The file name a source matches a dump by: the ROM's own name (with its
/// extension), or, for a game started from a boot file (`EBOOT.BIN`), the
/// nearest folder that is not part of the dump's layout. `None` when
/// nothing names the game.
pub fn rom_file(meta: &LocalGameMeta) -> Option<String> {
    let path = rom_path(meta)?;
    let parts = parts(&path);
    let (file, folders) = parts.split_last()?;
    if !is_boot_file(file) {
        return Some((*file).to_owned());
    }
    folders.iter().rev().find(|f| !is_generic_folder(f) && !f.ends_with(':')).map(|f| (*f).to_owned())
}

/// The platform of an emulated game, from the first that says it: the
/// entry's `platformId`, the ROM's extension when only one platform uses
/// it, the emulator's name, a folder of the ROM's path or of the game's
/// folder (`…/PS2/…`), else the record's platforms when they name exactly
/// one emulated platform. Reads nothing: everything is in `meta` and
/// `record_platforms`.
pub fn resolve(meta: &LocalGameMeta, record_platforms: &[String]) -> Option<&'static Platform> {
    if let Some(p) = meta.platform_id.as_deref().and_then(by_id) {
        return Some(p);
    }
    let rom = rom_path(meta);
    let ext = rom
        .as_deref()
        .and_then(|r| Path::new(parts(r).last().copied().unwrap_or("")).extension().map(|e| e.to_string_lossy().into_owned()));
    if let Some(p) = ext.as_deref().and_then(by_extension) {
        return Some(p);
    }
    if let Some(p) = meta.emulator_name.as_deref().and_then(by_name) {
        return Some(p);
    }
    let folders = rom.iter().chain(meta.windows_install_directory.iter()).flat_map(|path| {
        let parts = parts(path);
        // The ROM's own folders, nearest first; not the file.
        let n = if rom.as_deref() == Some(path.as_str()) { parts.len().saturating_sub(1) } else { parts.len() };
        parts.into_iter().take(n).rev().collect::<Vec<_>>()
    });
    for folder in folders {
        if let Some(p) = by_folder(folder) {
            return Some(p);
        }
    }
    let mut named = record_platforms.iter().filter_map(|n| by_name(n)).filter(|p| !p.id.starts_with("pc"));
    let first = named.next()?;
    named.all(|p| p == first).then_some(first)
}

#[cfg(test)]
mod tests;
