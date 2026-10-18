//! The Proton / Wine builds of the Wine Manager, and the runtimes and tools
//! games need. A build is in use when it is the default, an installed game
//! is set to it, or Steam uses it (Steam's `config.vdf` names the tool: a
//! build registered with Steam is `heronite-<folder>`, one in
//! `compatibilitytools.d` goes by the name its `compatibilitytool.vdf` gives).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::{game_config, Library};
use heronite_i18n::tr;
use heronite_launch::steam::SteamLocator;
use serde_json::Value;

use crate::{Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx, lib: &Library, steam: &SteamLocator) -> Vec<Item> {
    let default = heronite_wine::default::current(&ctx.data_paths).map(|w| PathBuf::from(w.bin));
    let users = game_builds(ctx, lib);
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
            let games: Vec<String> = users.iter().filter(|(bin, _)| bin.starts_with(&dir)).flat_map(|(_, keys)| keys.clone()).collect();
            let by_steam = steam_names(&dir).iter().any(|n| steam_config.contains(&format!("\"{n}\"")));
            let state = if is_default || by_steam || !games.is_empty() { State::InUse } else { State::Unused };
            let mut item = Item::new(Group::Builds, Kind::Build { version: r.version.clone(), default: is_default }, state, r.version, dir);
            item.games = games;
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
        let own = game_config::game_overrides(&ctx.data_paths, &g.app_name);
        let Some(bin) = own.get("wineVersion").and_then(|v| v.get("bin")).and_then(Value::as_str).filter(|b| !b.is_empty()) else {
            continue;
        };
        by_bin.entry(PathBuf::from(bin)).or_default().push(g.key.clone());
    }
    by_bin.into_iter().collect()
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
    use super::internal_name;

    #[test]
    fn reads_the_tool_name() {
        let vdf = "\"compatibilitytools\"\n{\n  \"compat_tools\"\n  {\n    \"dwproton-11.0-14\" // Internal name\n    {\n";
        assert_eq!(internal_name(vdf).as_deref(), Some("dwproton-11.0-14"));
        assert_eq!(internal_name("nothing"), None);
    }
}
