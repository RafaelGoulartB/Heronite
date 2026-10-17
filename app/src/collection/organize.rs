//! What the Collection's extra sorts and groups read besides the library
//! (`heronite_data::organize`): the day each game was added, the size of the
//! installed ones and the system of the emulated ones. Built with each load,
//! off the UI thread; sizes are only read (store value, Steam manifest, ROM
//! files, folders already measured), never walked here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use heronite_data::install_size::{rom_bytes, SizeCache};
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
        let meta = g.meta.as_deref();
        if g.launch_kind.as_deref() == Some("emulator") {
            if let Some(name) = meta.and_then(|m| m.platform_id.as_deref()).and_then(|id| systems.get(id)) {
                extras.systems.insert(g.key.clone(), name.clone());
            }
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

/// Platform id → name, from the emulator catalog.
fn system_names() -> HashMap<String, String> {
    heronite_emulation::catalog::emulator_definitions()
        .iter()
        .flat_map(|d| d.profiles.iter())
        .map(|p| (p.platform_id.clone(), p.platform_name.clone()))
        .collect()
}

fn size_of<'a>(g: &CollectionGame, steam: impl FnOnce() -> Option<&'a SteamLocator>, walked: &SizeCache) -> Option<u64> {
    if let Some(bytes) = g.details.install_size.as_deref().and_then(parse_size) {
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

/// A size as stores write it: bytes ("4518092416"), or a number and a unit
/// ("41.92 GB", "1.5 GiB", "700 MB"), 1024-based.
fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(text.len());
    let number: f64 = text[..split].replace(',', "").parse().ok()?;
    let unit = text[split..].trim().to_ascii_lowercase();
    let power = match unit.trim_end_matches("ib").trim_end_matches('b') {
        "" => 0,
        "k" => 1,
        "m" => 2,
        "g" => 3,
        "t" => 4,
        _ => return None,
    };
    let bytes = number * 1024f64.powi(power);
    (bytes.is_finite() && bytes >= 1.).then_some(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn sizes_in_store_shapes() {
        assert_eq!(parse_size("4518092416"), Some(4_518_092_416));
        assert_eq!(parse_size("1.5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_size("41.92 GB"), Some(45_011_257_262));
        assert_eq!(parse_size("700 MB"), Some(734_003_200));
        assert_eq!(parse_size("0"), None);
        assert_eq!(parse_size("big"), None);
    }
}
