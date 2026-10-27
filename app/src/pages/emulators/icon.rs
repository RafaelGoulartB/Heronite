//! The icon of each emulator of the catalog: Dolphin's and RetroArch's
//! logos, and for the others one drawn after their console or their name.

use heronite_emulation::catalog::guess_definition_id;
use heronite_launch::emulators::UserEmulator;

use crate::icons::Icon;

/// The icon of the catalog emulator `definition_id`; a gamepad for a custom
/// one.
pub fn of_definition(definition_id: &str) -> Icon {
    match definition_id {
        "dolphin" => Icon::BrandDolphin,
        "retroarch" => Icon::BrandRetroArch,
        "pcsx2" => Icon::EmuPcsx2,
        "duckstation" => Icon::EmuDuckStation,
        "ppsspp" => Icon::EmuPpsspp,
        "rpcs3" => Icon::EmuRpcs3,
        "cemu" => Icon::EmuCemu,
        "ryujinx" => Icon::EmuRyujinx,
        "xemu" => Icon::EmuXemu,
        "melonds" => Icon::EmuMelonDs,
        "mgba" => Icon::EmuMgba,
        "flycast" => Icon::EmuFlycast,
        "mame" => Icon::EmuMame,
        "scummvm" => Icon::EmuScummVm,
        "vita3k" => Icon::EmuVita3k,
        "shadps4" => Icon::EmuShadPs4,
        "azahar" => Icon::EmuAzahar,
        _ => Icon::SportsEsports,
    }
}

/// The icon of `e`: its catalog emulator's, or, for one added by hand, the
/// one its name points at ("PCSX2 Nightly" is PCSX2).
pub fn of(e: &UserEmulator) -> Icon {
    let id = if e.definition_id.is_empty() || e.definition_id == "custom" {
        guess_definition_id(Some(&e.name))
    } else {
        e.definition_id.clone()
    };
    of_definition(&id)
}

#[cfg(test)]
mod tests {
    use heronite_emulation::catalog::emulator_definitions;
    use heronite_launch::emulators::UserEmulator;

    use super::{of, of_definition};
    use crate::icons::Icon;

    #[test]
    fn every_emulator_of_the_catalog_has_an_icon_of_its_own() {
        let mut seen = Vec::new();
        for d in emulator_definitions().iter().filter(|d| d.id != "custom") {
            let icon = of_definition(&d.id);
            assert_ne!(icon, Icon::SportsEsports, "{} has no icon", d.id);
            assert!(!seen.contains(&icon), "{} shares its icon", d.id);
            seen.push(icon);
        }
        assert_eq!(of_definition("custom"), Icon::SportsEsports);
    }

    #[test]
    fn one_added_by_hand_takes_the_icon_its_name_points_at() {
        let e =
            |definition_id: &str, name: &str| UserEmulator { definition_id: definition_id.into(), name: name.into(), ..Default::default() };
        assert_eq!(of(&e("custom", "PCSX2 Nightly")), Icon::EmuPcsx2);
        assert_eq!(of(&e("", "My emulator")), Icon::SportsEsports);
        assert_eq!(of(&e("dolphin", "GameCube")), Icon::BrandDolphin);
    }
}
