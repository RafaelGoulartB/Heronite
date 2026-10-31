//! The Proton / Wine builds of the Wine Manager, and the runtimes and tools
//! games need. A build is in use when it is the default, an installed game
//! is set to it, a store client is set to it (its own settings file, shown
//! with the client's installed games), or Steam uses it (Steam's `config.vdf`
//! names the tool: a build registered with Steam is `heronite-<folder>`, one
//! in `compatibilitytools.d` goes by the name its `compatibilitytool.vdf`
//! gives).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::clients::Client;
use heronite_data::{game_config, Library};
use heronite_i18n::tr;
use heronite_launch::steam::SteamLocator;
use serde_json::Value;

use crate::{Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx, lib: &Library, steam: &SteamLocator) -> Vec<Item> {
    let default = heronite_wine::default::current(&ctx.data_paths).map(|w| PathBuf::from(w.bin));
    let users = game_builds(ctx, lib);
    let by_client = client_builds(ctx);
    let steam_config = fs::read_to_string(steam.steam_path.join("config/config.vdf")).unwrap_or_default();
    let mut out: Vec<Item> = heronite_wine::store::read_releases(&ctx.data_paths)
        .into_iter()
        .filter(|r| r.installed())
        .filter_map(|r| {
            let dir = PathBuf::from(r.install_dir.as_deref()?);
            dir.is_dir().then_some((r, dir))
        })
        .map(|(r, dir)| {
            let is_default = default.as_deref().is_some_and(|bin| bin.starts_with(&dir));
            let mut games: Vec<String> = users.iter().filter(|(bin, _)| bin.starts_with(&dir)).flat_map(|(_, keys)| keys.clone()).collect();
            // A client's games run with its build.
            let clients: Vec<Client> = by_client.iter().filter(|(bin, _)| bin.starts_with(&dir)).map(|(_, c)| *c).collect();
            games.extend(
                lib.games.iter().filter(|g| g.installed && g.client().is_some_and(|c| clients.contains(&c))).map(|g| g.key.clone()),
            );
            let by_steam = steam_names(&dir).iter().any(|n| steam_config.contains(&format!("\"{n}\"")));
            let state = if is_default || by_steam || !games.is_empty() || !clients.is_empty() { State::InUse } else { State::Unused };
            let mut item = Item::new(Group::Builds, Kind::Build { version: r.version.clone(), default: is_default }, state, r.version, dir);
            item.games = games;
            item.clients = clients.iter().map(|c| c.name().to_string()).collect();
            item.source = Some(r.kind).filter(|k| !k.is_empty());
            item
        })
        .collect();
    out.extend(runtimes(ctx));
    out
}

/// The Wine / Proton binary each installed game is set to, with the games.
fn game_builds(ctx: &Ctx, lib: &Library) -> Vec<(PathBuf, Vec<String>)> {
    let mut by_bin: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for g in lib.games.iter().filter(|g| g.installed) {
        if let Some(bin) = build_of(ctx, &g.app_name) {
            by_bin.entry(bin).or_default().push(g.key.clone());
        }
    }
    by_bin.into_iter().collect()
}

/// The Wine / Proton binary each store client is set to (its own settings
/// file, set up or not: the build is in use all the same).
fn client_builds(ctx: &Ctx) -> Vec<(PathBuf, Client)> {
    Client::ALL.into_iter().filter_map(|c| Some((build_of(ctx, c.config_app())?, c))).collect()
}

/// The `wineVersion` binary in the settings of `app_name` itself.
fn build_of(ctx: &Ctx, app_name: &str) -> Option<PathBuf> {
    let own = game_config::game_overrides(&ctx.data_paths, app_name);
    own.get("wineVersion").and_then(|v| v.get("bin")).and_then(Value::as_str).filter(|b| !b.is_empty()).map(PathBuf::from)
}

/// The names Steam may know the build by.
fn steam_names(dir: &Path) -> Vec<String> {
    let Some(folder) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { return Vec::new() };
    let mut out = vec![format!("heronite-{folder}"), folder];
    let manifest = fs::read_to_string(dir.join("compatibilitytool.vdf")).unwrap_or_default();
    out.extend(internal_name(&manifest));
    out
}

/// The first key inside `compat_tools`: `"compat_tools" { "dwproton-11" { ...`.
fn internal_name(manifest: &str) -> Option<String> {
    let rest = &manifest[manifest.find("\"compat_tools\"")? + 14..];
    let rest = &rest[rest.find('{')? + 1..];
    let start = rest.find('"')? + 1;
    let len = rest[start..].find('"')?;
    Some(rest[start..start + len].to_string()).filter(|n| !n.is_empty())
}

/// The runtimes and tools: never offered for removal.
fn runtimes(ctx: &Ctx) -> Vec<Item> {
    let tools = ctx.data_paths.tools_dir();
    let mut parts: Vec<PathBuf> = fs::read_dir(&tools)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !is_builds_dir(p)).collect())
        .unwrap_or_default();
    parts.sort();
    let mut out = Vec::new();
    if !parts.is_empty() {
        let mut item = Item::new(Group::Builds, Kind::Runtime, State::Needed, tr!("DXVK, VKD3D and other tools"), tools.clone());
        item.id = format!("{}#tools", tools.display());
        item.paths = parts;
        out.push(item);
    }
    let umu = ctx.home.join(".local/share/umu");
    if umu.is_dir() {
        out.push(Item::new(Group::Builds, Kind::Runtime, State::Needed, tr!("Steam Linux Runtime (umu)"), umu));
    }
    out
}

/// `tools/proton` and `tools/wine` hold the builds, listed one by one.
fn is_builds_dir(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n == "proton" || n == "wine" || n == "game-porting-toolkit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::{json_store, AppPaths, DataPaths};
    use heronite_wine::WineVersionInfo;
    use serde_json::json;

    #[test]
    fn reads_the_tool_name() {
        let vdf = "\"compatibilitytools\"\n{\n  \"compat_tools\"\n  {\n    \"dwproton-11.0-14\" // Internal name\n    {\n";
        assert_eq!(internal_name(vdf).as_deref(), Some("dwproton-11.0-14"));
        assert_eq!(internal_name("nothing"), None);
    }

    /// Battle.net's Wine build is set in its own settings file, no game's.
    #[test]
    fn a_build_set_on_a_store_client_is_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let ctx = Ctx {
            data_paths: DataPaths::at(dir.path().join("data")),
            app: AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") },
            home: home.clone(),
            updating: false,
        };
        let paths = &ctx.data_paths;
        let build = |version: &str| {
            let install = paths.tools_dir().join("proton").join(version);
            std::fs::create_dir_all(&install).unwrap();
            std::fs::write(install.join("proton"), "x").unwrap();
            let info = WineVersionInfo {
                version: version.into(),
                kind: "GE-Proton".into(),
                install_dir: Some(install.to_string_lossy().into_owned()),
                is_installed: Some(true),
                ..Default::default()
            };
            heronite_wine::store::mark_installed(paths, &info).unwrap();
            install
        };
        let (client_build, spare) = (build("GE-Proton9-1"), build("GE-Proton9-2"));
        let steam = SteamLocator { home: home.clone(), steam_path: home.join(".steam/steam"), system_roots: Vec::new() };
        let state_of = |version: &str| {
            let lib = Library::load(paths).unwrap();
            let found = items(&ctx, &lib, &steam);
            let item = found.iter().find(|i| i.title == version).unwrap();
            assert_eq!(item.clients.is_empty(), item.state == State::Unused, "{version}: only a client is named here");
            (item.state, item.games.clone())
        };
        assert_eq!(state_of("GE-Proton9-1").0, State::Unused);

        let wine_version = json!({"bin": client_build.join("proton"), "name": "GE-Proton9-1", "type": "proton"});
        game_config::set_game_setting(paths, Client::BattleNet.config_app(), &ctx.home_str(), "wineVersion", wine_version).unwrap();
        assert_eq!(state_of("GE-Proton9-1"), (State::InUse, Vec::new()), "in use before it has a game");
        assert_eq!(state_of("GE-Proton9-2").0, State::Unused);
        assert!(spare.exists());

        json_store::write(
            &paths.sideload_library(),
            &json!({"games": [
                {"runner": "sideload", "app_name": "battlenet_wow", "title": "WoW", "is_installed": true, "install": {}},
                {"runner": "sideload", "app_name": "battlenet_d4", "title": "Diablo IV", "install": {}},
                {"runner": "sideload", "app_name": "ea_apex", "title": "Apex", "is_installed": true, "install": {}}]}),
        )
        .unwrap();
        let meta = |app: &str, source: &str| json!({"appName": app, "runner": "sideload", "source": source, "launchKind": "client"});
        json_store::write(
            &paths.local_meta(),
            &json!({"games": {"battlenet_wow": meta("battlenet_wow", "battlenet"), "battlenet_d4": meta("battlenet_d4", "battlenet"),
                              "ea_apex": meta("ea_apex", "ea")}}),
        )
        .unwrap();
        assert_eq!(state_of("GE-Proton9-1"), (State::InUse, vec!["sideload_battlenet_wow".to_string()]), "its installed games only");
    }
}
