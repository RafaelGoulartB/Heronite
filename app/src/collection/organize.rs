//! What the Collection's extra sorts and groups read besides the library
//! (`heronite_data::organize`): the day each game was added, the size of the
//! installed ones and the system of the emulated ones. Built with each load,
//! off the UI thread; sizes are only read (store value, Steam manifest, ROM
//! files, folders already measured), never walked here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use heronite_data::install_size::{parse_store_size, rom_bytes, SizeCache};
use heronite_data::models::LocalGameMeta;
use heronite_data::organize::GameExtras;
use heronite_data::{CollectionGame, DataPaths, Library};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

pub(super) fn game_extras(lib: &Library, paths: &DataPaths, cache: &Path) -> GameExtras {
    let added = heronite_data::added::sync(paths, &lib.games);
    let systems = system_names();
    let locator = std::cell::OnceCell::new();
    let walked = SizeCache::load(cache);
    let mut extras = GameExtras { added, ..Default::default() };
    for g in &lib.games {
        if let Some(name) = emulated_system(g, &systems) {
            extras.systems.insert(g.key.clone(), name);
        }
        if !g.installed {
            continue;
        }
        let steam = || Some(locator.get_or_init(|| SteamLocator::detect(&GlobalSettings::load(paths), &SystemProbe)));
        if let Some(bytes) = size_of(g, steam, &walked).filter(|b| *b > 0) {
            extras.sizes.insert(g.key.clone(), bytes);
        }
    }
    extras
}

/// Only the systems of the emulated games (all `Organize::platform` reads),
/// for what has no use for sizes and dates: the Settings pickers.
pub(crate) fn system_extras(lib: &Library) -> GameExtras {
    let systems = system_names();
    let systems = lib.games.iter().filter_map(|g| Some((g.key.clone(), emulated_system(g, &systems)?))).collect();
    GameExtras { systems, ..Default::default() }
}

/// The system an emulated game runs on. A game imported through a profile
/// set up by hand has the `other` platform id; its ROM, folders or record
/// (the profile's platform name, seeded at import) can still name it.
fn emulated_system(g: &CollectionGame, systems: &HashMap<String, String>) -> Option<String> {
    if g.launch_kind.as_deref() != Some("emulator") {
        return None;
    }
    let record = g.metadata.as_deref().map_or(&[][..], |m| m.platforms.as_slice());
    system_of(g.meta.as_deref()?, record, systems)
}

fn system_of(meta: &LocalGameMeta, record: &[String], systems: &HashMap<String, String>) -> Option<String> {
    let id = meta.platform_id.as_deref()?;
    if id != "other" {
        return systems.get(id).cloned();
    }
    match heronite_metadata::platforms::resolve(meta, record) {
        Some(p) => Some(systems.get(p.id).cloned().unwrap_or_else(|| p.name.to_owned())),
        None => systems.get(id).cloned(),
    }
}

/// Platform id → name, from the emulator catalog.
fn system_names() -> HashMap<String, String> {
    heronite_emulation::catalog::emulator_definitions()
        .iter()
        .flat_map(|d| d.profiles.iter())
        .map(|p| (p.platform_id.clone(), p.platform_name.clone()))
        .collect()
}

fn size_of<'a>(g: &CollectionGame, steam: impl FnOnce() -> Option<&'a SteamLocator>, walked: &SizeCache) -> Option<u64> {
    if let Some(bytes) = g.details.install_size.as_deref().and_then(parse_store_size) {
        return Some(bytes);
    }
    let meta = g.meta.as_deref();
    if g.launch_kind.as_deref() == Some("emulator") {
        let roms: Vec<PathBuf> = meta
            .and_then(|m| m.selected_rom_path.as_deref())
            .into_iter()
            .chain(meta.and_then(|m| m.roms.as_ref()).into_iter().flatten().map(|r| r.path.as_str()))
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();
        return Some(rom_bytes(&roms));
    }
    if g.launch_kind.as_deref() == Some("steam-uri") {
        return g.steam_app_id.as_deref().and_then(|id| steam()?.size_on_disk(id));
    }
    let dir = [g.details.install_path.as_deref(), g.game.install.install_path.as_deref()]
        .into_iter()
        .flatten()
        .map(|p| Path::new(p.trim()))
        .find(|p| p.is_absolute())?;
    walked.get(dir)
}

#[cfg(test)]
mod tests {
    use super::{system_names, system_of};
    use heronite_data::models::{LocalGameMeta, LocalGameRom};

    fn rom(platform_id: &str, path: &str) -> LocalGameMeta {
        LocalGameMeta {
            platform_id: Some(platform_id.into()),
            emulator_name: Some("Ymir".into()),
            roms: Some(vec![LocalGameRom { name: None, path: path.into(), extra: Default::default() }]),
            ..Default::default()
        }
    }

    #[test]
    fn hand_made_profile_takes_the_platform_its_record_names() {
        let systems = system_names();
        let meta = rom("other", "/games/roms/Panzer Dragoon.chd");
        assert_eq!(system_of(&meta, &["Sega Saturn".into()], &systems).as_deref(), Some("Sega Saturn"));
    }

    #[test]
    fn unnamed_platform_stays_other_and_known_ids_are_kept() {
        let systems = system_names();
        assert_eq!(system_of(&rom("other", "/games/roms/x.chd"), &[], &systems).as_deref(), Some("Other"));
        assert_eq!(system_of(&rom("sega_saturn", "/games/roms/x.chd"), &[], &systems).as_deref(), Some("Sega Saturn"));
    }
}
