//! What the Collection's extra sorts and groups read besides the library
//! (`heronite_data::organize`): the day each game was added, the size of the
//! installed ones and the system of the emulated ones. Built with each load,
//! off the UI thread; sizes are only read (store value, Steam manifest, ROM
//! files, folders already measured), never walked here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use heronite_data::install_size::{parse_store_size, rom_bytes, SizeCache};
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

/// The system an emulated game runs on.
fn emulated_system(g: &CollectionGame, systems: &HashMap<String, String>) -> Option<String> {
    if g.launch_kind.as_deref() != Some("emulator") {
        return None;
    }
    g.meta.as_deref().and_then(|m| m.platform_id.as_deref()).and_then(|id| systems.get(id)).cloned()
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
