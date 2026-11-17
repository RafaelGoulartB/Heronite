//! How Ludusavi names an emulated game.
//!
//! The emulator manifests ([`ludusavi-emulator-manifests`]) key a game by its
//! cleaned No-Intro/Redump title plus the platform, and their generator strips
//! the *trailing* `(...)`/`[...]` groups of that title (`strip_tags`). A
//! Collection title can keep those groups — a ROM added by hand or by
//! drag/drop does not go through [`crate::names::title_from_rom_path`] — so the
//! lookup tries the title as it is and with its trailing tags removed.
//!
//! This is a port of the generator's rule, not of `emulation/names.ts`:
//! [`crate::names::sanitize_rom_name`] removes groups anywhere, which would
//! also eat punctuation that is part of a title.
//!
//! [`ludusavi-emulator-manifests`]: https://github.com/sharkusmanch/ludusavi-emulator-manifests

use regex::Regex;
use std::sync::OnceLock;

/// `strip_tags` of the generator: `re.sub(r"(\s*[(\[][^)\]]*[)\]])+\s*$", "", name)`.
///
/// `None` only if the literal regex ever failed to compile; callers then keep
/// the name as it is instead of panicking.
fn tags_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\s*[(\[][^)\]]*[)\]])+\s*$").ok()).as_ref()
}

/// Trailing `(...)`/`[...]` groups removed, repeatedly, then trimmed. When
/// nothing is left, the input comes back untouched, as the generator does
/// (`re.sub(...).strip() or name`).
fn strip_trailing_tags(name: &str) -> String {
    let Some(re) = tags_re() else { return name.to_string() };
    let stripped = re.replace(name, "");
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        name.to_string()
    } else {
        trimmed.to_string()
    }
}

/// The title a ROM file name gives the manifests: its stem — the last segment
/// of either separator, so a Windows path works on any host — with the
/// trailing tags the generator strips.
fn rom_title(rom_path: &str) -> Option<String> {
    let file = rom_path.rsplit(['/', '\\']).next()?;
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let title = strip_trailing_tags(stem);
    (!title.trim().is_empty()).then_some(title)
}

/// The names Ludusavi may know an emulated game by, from the catalog id of its
/// emulator ([`super::backup::emulator_engine`]). The manifests suffix the
/// platform, because one title can exist on several systems
/// ("Turbo Goose Racer (RetroArch)", "… (PS2)"), while the Collection keeps
/// the bare title. The bases are the title as it is, the title with the
/// trailing tags the generator strips, and the tag-stripped ROM file name:
/// that last one also covers a title renamed, punctuation-stripped or cut at
/// 40 characters when the game was added by hand.
///
/// Ryujinx is left out on purpose: its manifest has one entry for every
/// Switch save, so restoring it from one game would roll back all of them.
///
/// Stripping widens which titles land on one entry — `Game (NES)` and `Game`
/// both look up `Game (RetroArch)`, as the manifests already group one title
/// across regions — so restoring that entry restores every file it holds.
pub(super) fn emulator_names(engine: Option<&str>, title: &str, rom_path: Option<&str>) -> Vec<String> {
    let title = title.trim();
    if title.is_empty() {
        return Vec::new();
    }
    let suffixes: &[&str] = match engine.unwrap_or_default() {
        "retroarch" => &["RetroArch"],
        "duckstation" => &["PS1"],
        "pcsx2" => &["PS2"],
        "rpcs3" => &["PS3"],
        "shadps4" => &["PS4"],
        "ppsspp" => &["PSP"],
        "vita3k" => &["Vita"],
        "dolphin" => &["GameCube", "Wii"],
        "cemu" => &["Wii U"],
        "azahar" => &["3DS"],
        "scummvm" => &["ScummVM"],
        _ => return Vec::new(),
    };
    let stripped = strip_trailing_tags(title);
    let mut bases: Vec<&str> = vec![title];
    if stripped != title {
        bases.push(stripped.as_str());
    }
    let rom = rom_path.and_then(rom_title);
    if let Some(rom) = rom.as_deref() {
        if !bases.contains(&rom) {
            bases.push(rom);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for base in bases {
        for suffix in suffixes {
            // Never "Game (RetroArch) (RetroArch)".
            if base.ends_with(&format!("({suffix})")) {
                continue;
            }
            let name = format!("{base} ({suffix})");
            // The bare title is tried before these.
            if name == title || out.contains(&name) {
                continue;
            }
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_trailing_tags() {
        // Tag shapes from the real naming schemes — No-Intro (regions,
        // languages, revisions, dev states, special releases), Redump (discs,
        // alt, rerelease), GoodTools dump/hack/translation flags and TOSEC's
        // no-space groups — on invented titles, each checked against the
        // generator's own regex.
        let cases = [
            // No-Intro
            ("Turbo Goose Racer (World)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA, Europe)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (Japan, USA, Europe)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (Latin America)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (Japan) (En,Ja)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Rev A)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Rev 1) (Beta)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Rev 1) (Kiosk)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (Japan) (Virtual Console)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (eShop) (Rev 1)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Unl)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Proto) (1994-01-01)", "Turbo Goose Racer"),
            // Redump
            ("Turbo Goose Racer (USA) (Disc 1) (Rev 1)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (Europe) (Disc 3)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (Japan) (Alt)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Rerelease)", "Turbo Goose Racer"),
            // GoodTools dump, hack and translation flags
            ("Turbo Goose Racer (USA) [!]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [b1]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [o1]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [a1]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [f1]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [p1]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [h1C]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) [T+Eng1.0]", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (PRG 0)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (USA) (Hack)", "Turbo Goose Racer"),
            ("Robot Gardener (Unl) (1993)", "Robot Gardener"),
            ("Robot Gardener [b] (USA) (Rev A)", "Robot Gardener"),
            ("Robot Gardener (USA) [b1] [!]", "Robot Gardener"),
            // TOSEC: groups without spaces between them
            ("Turbo Goose Racer (1993)(Nintendo)(USA)(En)", "Turbo Goose Racer"),
            ("Turbo Goose Racer (1993)(Nintendo)(Europe)(En,Fr,De)", "Turbo Goose Racer"),
            ("Robot Gardener (USA)(Rev 1)", "Robot Gardener"),
            // Trailing groups of any kind strip, repeatedly.
            ("Robot Gardener (USA) (Disc 1) (Side A)", "Robot Gardener"),
            ("Robot Gardener (v1.1)", "Robot Gardener"),
            ("Robot Gardener ()", "Robot Gardener"),
            ("Robot Gardener []", "Robot Gardener"),
            ("Robot Gardener  (USA)  ", "Robot Gardener"),
            ("\tRobot Gardener (USA)\t", "Robot Gardener"),
            ("Robot Gardener - Turbo Edition (USA)", "Robot Gardener - Turbo Edition"),
            ("Turbo Goose Racer: Turbo Edition (USA)", "Turbo Goose Racer: Turbo Edition"),
            ("Robot Gardener, Inc. (Turbo Microgame$!)", "Robot Gardener, Inc."),
            // Real names with text after the tags: nothing to strip, so the
            // lookup needs the ROM name or a generator alias.
            ("Turbo Goose Racer (USA) v1.2", "Turbo Goose Racer (USA) v1.2"),
            ("Turbo Goose Racer (USA) v1.2 (Hack)", "Turbo Goose Racer (USA) v1.2"),
            ("Robot Gardener (USA) Extra", "Robot Gardener (USA) Extra"),
            ("Robot Gardener (USA) 2", "Robot Gardener (USA) 2"),
            // Only tags, or malformed groups: the input comes back untouched.
            ("(USA)", "(USA)"),
            ("   ", "   "),
            ("Robot Gardener ((USA))", "Robot Gardener ((USA))"),
            ("Robot Gardener (a)b)", "Robot Gardener (a)b)"),
            ("Robot Gardener (USA", "Robot Gardener (USA"),
            // A title cut at 40 characters has nothing to strip; the ROM name
            // is what recovers it (see the second half of the fix).
            ("A Very Long Game Name Cut At Fo", "A Very Long Game Name Cut At Fo"),
        ];
        for (name, expected) in cases {
            assert_eq!(strip_trailing_tags(name), expected, "{name:?}");
        }
    }

    #[test]
    fn names_try_the_title_and_its_tag_stripped_form() {
        assert_eq!(emulator_names(Some("retroarch"), "Turbo Goose Racer", None), ["Turbo Goose Racer (RetroArch)"]);
        assert_eq!(
            emulator_names(Some("retroarch"), "Turbo Goose Racer (NA)", None),
            ["Turbo Goose Racer (NA) (RetroArch)", "Turbo Goose Racer (RetroArch)"]
        );
        assert_eq!(emulator_names(Some("dolphin"), "Moon Cheese Caper", None), ["Moon Cheese Caper (GameCube)", "Moon Cheese Caper (Wii)"]);
        assert_eq!(
            emulator_names(Some("dolphin"), "Moon Cheese Caper (NA)", None),
            [
                "Moon Cheese Caper (NA) (GameCube)",
                "Moon Cheese Caper (NA) (Wii)",
                "Moon Cheese Caper (GameCube)",
                "Moon Cheese Caper (Wii)"
            ]
        );
        assert_eq!(emulator_names(Some("pcsx2"), "Robot Gardener 2", None), ["Robot Gardener 2 (PS2)"]);
        assert!(emulator_names(Some("ryujinx"), "Anything", None).is_empty());
        assert!(emulator_names(None, "Control", None).is_empty());
        // A PC game has no engine, so even a stray ROM in its meta adds nothing.
        assert!(emulator_names(None, "Control", Some("/roms/stray.sfc")).is_empty());
        assert!(emulator_names(Some("retroarch"), "   ", None).is_empty());
        // Already suffixed: nothing to add.
        assert!(emulator_names(Some("retroarch"), "Already (RetroArch)", None).is_empty());
        // A trailing tag is tried as-is first, then stripped.
        assert_eq!(emulator_names(Some("pcsx2"), "Robot Gardener (2004)", None), ["Robot Gardener (2004) (PS2)", "Robot Gardener (PS2)"]);
    }

    #[test]
    fn names_try_the_rom_name_when_the_title_differs() {
        // Renamed title: the ROM still carries the database title.
        assert_eq!(
            emulator_names(Some("retroarch"), "MK3", Some("/roms/Great Adventure (NA).sfc")),
            ["MK3 (RetroArch)", "Great Adventure (RetroArch)"]
        );
        // Punctuation `clean_title` deletes from a manual title.
        assert_eq!(
            emulator_names(Some("retroarch"), "Game The Sequel", Some("/roms/Game: The Sequel (USA).sfc")),
            ["Game The Sequel (RetroArch)", "Game: The Sequel (RetroArch)"]
        );
        // An underscore ROM name is looked up correctly; the manifest's file
        // globs may still not match it, which is the manifests' limitation.
        assert_eq!(
            emulator_names(Some("retroarch"), "Game Name", Some("/roms/Game_Name.sfc")),
            ["Game Name (RetroArch)", "Game_Name (RetroArch)"]
        );
        // Multi-disc: the disc tag goes with the rest.
        assert_eq!(
            emulator_names(Some("retroarch"), "Great Adventure", Some("/roms/Great Adventure (USA) (Disc 2).bin")),
            ["Great Adventure (RetroArch)"]
        );
        // A Windows path on any host: the stem is the last segment either way.
        assert_eq!(
            emulator_names(Some("retroarch"), "MK3", Some(r"C:\roms\Great Adventure (NA).sfc")),
            ["MK3 (RetroArch)", "Great Adventure (RetroArch)"]
        );
        // A real ROM-hack name: the version sits after the region, so nothing
        // strips; the ROM's clean name still reaches the manifest entry.
        assert_eq!(
            emulator_names(Some("retroarch"), "Turbo Goose Racer (USA) v1.2", Some("/roms/Turbo Goose Racer (USA).sfc")),
            ["Turbo Goose Racer (USA) v1.2 (RetroArch)", "Turbo Goose Racer (RetroArch)"]
        );
        // The ROM base is not added twice when it equals the stripped title.
        assert_eq!(
            emulator_names(Some("retroarch"), "Great Adventure (NA)", Some("/roms/Great Adventure (NA).sfc")),
            ["Great Adventure (NA) (RetroArch)", "Great Adventure (RetroArch)"]
        );
        // No ROM, no third base.
        assert_eq!(emulator_names(Some("retroarch"), "MK3", None), ["MK3 (RetroArch)"]);
    }

    #[test]
    fn candidates_are_unique() {
        let names = emulator_names(Some("dolphin"), "Moon Cheese Caper (NA)", None);
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "{names:?}");
    }
}
