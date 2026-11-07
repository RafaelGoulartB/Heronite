//! Game files: achievements a game keeps in its own files, as the
//! open-source Steam API emulators (Goldberg's and gbe_fork) save them for
//! DRM-free and offline copies. Off unless Settings turns it on; only read.
//!
//! The game's Steam app id comes from its files (or a choice made by hand),
//! the names and icons from the emulator's own list, else from Steam (the
//! client's files, the Web API with the key, the public page), and what is
//! unlocked from the emulator's saves. The set is its own ("Game files"),
//! never mixed into a Steam account's.

pub mod gbe;
pub mod locate;

use std::path::PathBuf;

use heronite_launch::probe::SystemProbe;
use heronite_launch::settings::{load_game_settings, GlobalSettings};

use self::locate::{Located, Places};
use super::steam::{self, web};
use super::{Ctx, Fetch};
use crate::model::Source;
use crate::target::LocalTarget;

/// The places of this PC for a game: the native saves folders and, for a
/// game that runs under Wine, its prefix's; and the app id its launch
/// settings give the emulator.
pub fn places(ctx: &Ctx, target: &LocalTarget) -> Places {
    let mut roots = locate::native_roots();
    let settings = load_game_settings(ctx.paths, &target.app_name, &GlobalSettings::load(ctx.paths), &SystemProbe);
    if !cfg!(windows) {
        let prefix = target.launcher_prefix.clone().or_else(|| Some(PathBuf::from(&settings.wine_prefix)).filter(|p| p.is_absolute()));
        if let Some(prefix) = prefix {
            roots.extend(locate::prefix_roots(&prefix));
        }
    }
    let env_app_id = settings
        .env_options
        .iter()
        .find(|(k, _)| k == "SteamAppId" || k == "SteamGameId")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
    Places { roots, env_app_id }
}

pub fn fetch(ctx: &Ctx, target: &LocalTarget) -> Fetch {
    if target.pin.as_ref().is_some_and(|p| p.off) {
        return Fetch::None;
    }
    let Some(found) = locate::locate(target, &places(ctx, target)) else { return Fetch::None };
    read(ctx, &found)
}

/// The set of what was located (the network only for the names and the
/// rarity, when the emulator's files do not have them).
pub fn read(ctx: &Ctx, found: &Located) -> Fetch {
    let schema_file = found.schema_file();
    if found.state.is_none() && schema_file.is_none() {
        return Fetch::None;
    }
    let (schema, rarity) = match (&schema_file, &found.settings) {
        (Some(file), Some(settings)) => {
            match std::fs::read(file).map_err(|e| e.to_string()).and_then(|b| gbe::parse_schema(&b, ctx.lang, settings)) {
                Ok(parsed) => parsed,
                Err(e) => return Fetch::Failed(format!("{}: {e}", file.display())),
            }
        }
        _ => match steam::schema_of(ctx, &found.app_id) {
            Ok(Some(schema)) => (schema, Default::default()),
            Ok(None) => return Fetch::None,
            Err(e) => return Fetch::Failed(e),
        },
    };
    if schema.entries.is_empty() {
        return Fetch::None;
    }
    let unlocks = match &found.state {
        Some(file) => match std::fs::read(file).map_err(|e| e.to_string()).and_then(|b| gbe::parse_state(&b)) {
            Ok(state) => Some(gbe::unlocks(&state, &schema)),
            Err(e) => return Fetch::Failed(format!("{}: {e}", file.display())),
        },
        None => None,
    };
    let rarity = if rarity.is_empty() { web::rarity(ctx.http, steam::API_BASE, &found.app_id) } else { rarity };
    Fetch::Set(steam::build(Source::Local, &found.app_id, &schema, unlocks.as_ref(), &rarity, steam::CDN_BASE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Accounts;
    use heronite_data::paths::DataPaths;
    use heronite_metadata::Http;

    /// A game with its emulator's list (so nothing is asked of Steam but
    /// the rarity, which fails offline and is left out).
    #[test]
    fn reads_the_emulators_list_and_the_saves() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("Game/steam_settings");
        std::fs::create_dir_all(settings.join("img")).unwrap();
        std::fs::write(settings.join("img/a.jpg"), b"x").unwrap();
        std::fs::write(
            settings.join("achievements.json"),
            br#"[{"name":"A","displayName":"Alpha","icon":"img/a.jpg","unlock_percentage":12.5},
                 {"name":"B","displayName":"Beta","hidden":1}]"#,
        )
        .unwrap();
        let state = dir.path().join("saves/480/achievements.json");
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(&state, br#"{"A":{"earned":true,"earned_time":1700000000}}"#).unwrap();
        let found = Located { app_id: "480".into(), from: locate::AppIdFrom::File, settings: Some(settings.clone()), state: Some(state) };
        let paths = DataPaths::at(dir.path());
        let http = Http::default();
        let accounts = Accounts::default();
        let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
        let Fetch::Set(set) = read(&ctx, &found) else { panic!("expected a set") };
        assert_eq!((set.source, set.external_id.as_str(), set.total(), set.unlocked()), (Source::Local, "480", 2, 1));
        let (a, b) = (&set.achievements[0], &set.achievements[1]);
        assert_eq!((a.name.as_str(), a.unlocked, a.rarity), ("Alpha", true, Some(12.5)));
        assert_eq!(a.unlocked_at.unwrap().timestamp(), 1_700_000_000);
        assert_eq!(std::path::Path::new(&a.icon), settings.join("img/a.jpg"));
        assert!(b.hidden && !b.unlocked);
    }

    #[test]
    fn a_broken_state_file_fails_without_losing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("achievements.json"), br#"[{"name":"A"}]"#).unwrap();
        let state = dir.path().join("achievements.json");
        std::fs::write(&state, b"{ half written").unwrap();
        let found = Located { app_id: "1".into(), from: locate::AppIdFrom::File, settings: Some(settings), state: Some(state) };
        let paths = DataPaths::at(dir.path());
        let (http, accounts) = (Http::default(), Accounts::default());
        let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
        assert!(matches!(read(&ctx, &found), Fetch::Failed(_)));
    }
}
