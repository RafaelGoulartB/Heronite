//! ReShade (<https://reshade.me>) per game: post-processing effects added
//! by a DLL the game loads in place of a graphics one (`dxgi.dll`,
//! `d3d9.dll`, `opengl32.dll`, `ddraw.dll`). It loads the real DLL and runs
//! shaders on the image before it reaches the screen.
//!
//! Heronite does what ReShade's installer does: picks the executable and
//! its API, copies the DLL under the right name, adds shader packages and
//! a preset, writes `ReShade.ini` ([`install`], [`manage`]). Everything is
//! downloaded from the official sources when the user installs
//! ([`fetch`]); nothing ships with the app.
//!
//! Under Wine and Proton the game must also prefer the DLL in its folder
//! over Wine's own: [`apply`] adds `<proxy>=n,b` to `WINEDLLOVERRIDES` at
//! launch, so removing ReShade leaves nothing behind in the settings. On
//! Windows the file alone is enough. A Steam game Steam starts under Proton
//! gets the override in its launch options instead, written by the game
//! settings while ReShade is on and taken out once it is off or removed
//! ([`steam`] finds its folder and which build Steam installed).
//!
//! Not supported: Vulkan games (ReShade is a registered layer there),
//! Direct3D 8 (needs `d3d8to9`), Linux games (a Steam game's Linux build
//! among them), emulated games (ReShade would go next to the emulator, into
//! every game it runs).

mod anticheat;
mod archive;
pub mod catalog;
pub mod detect;
pub mod fetch;
mod ini;
pub mod install;
pub mod manage;
pub mod manifest;
mod overview;
pub mod preset;
pub mod steam;

use heronite_data::DataPaths;
use serde_json::{Map, Value};

pub use anticheat::{Finding, Risk, Source};
pub use overview::{fits, overview, Facts, Overview, SourceRow, State};

use crate::env::Env;

/// Mirrors the manifest's `enabled`: whether the DLL has its name.
pub const KEY_ENABLED: &str = "heroniteReshade";
/// Path of the executable ReShade is for; empty: detected.
pub const KEY_TARGET: &str = "heroniteReshadeTarget";
/// [`API_CHOICES`]: the DLL name ReShade takes.
pub const KEY_API: &str = "heroniteReshadeApi";
/// Anti-cheat names the user accepted the risk of.
pub const KEY_ANTICHEAT_ACK: &str = "heroniteReshadeAntiCheatAck";

/// `auto` and the DLL names that can be picked (`d3d11` is the DirectX 11
/// alternative to `dxgi`).
pub const API_CHOICES: [&str; 6] = ["auto", "dxgi", "d3d11", "d3d9", "opengl32", "ddraw"];

/// The game's ReShade settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReShade {
    pub enabled: bool,
    pub target: String,
    /// One of [`API_CHOICES`].
    pub api: String,
    pub anticheat_ack: Vec<String>,
}

impl ReShade {
    pub fn from_map(m: &Map<String, Value>) -> Self {
        let text = |k: &str| m.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
        let api = text(KEY_API);
        Self {
            enabled: m.get(KEY_ENABLED) == Some(&Value::Bool(true)),
            target: text(KEY_TARGET),
            api: if API_CHOICES.contains(&api.as_str()) { api } else { "auto".into() },
            anticheat_ack: m
                .get(KEY_ANTICHEAT_ACK)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }

    /// The DLL name picked by hand (`None`: the API's own).
    pub fn proxy_choice(&self) -> Option<String> {
        (self.api != "auto" && !self.api.is_empty()).then(|| format!("{}.dll", self.api))
    }
}

/// The DLL names of an override list (`a,b=n,b;c=d`), lowercase, without
/// `.dll`, with the entry each belongs to.
fn override_names(value: &str) -> Vec<(String, usize)> {
    value
        .split(';')
        .enumerate()
        .flat_map(|(i, entry)| {
            let names = entry.split_once('=').map_or(entry, |(n, _)| n);
            names.split(',').map(move |n| (n.trim().to_ascii_lowercase().trim_end_matches(".dll").to_string(), i))
        })
        .filter(|(n, _)| !n.is_empty())
        .collect()
}

fn stem(dll: &str) -> String {
    dll.trim().to_ascii_lowercase().trim_end_matches(".dll").to_string()
}

/// `current` with `<dll>=n,b` added for each of `dlls` it does not name.
/// Gives the names the user already sets: their value is kept.
pub fn merge_overrides(current: &str, dlls: &[&str]) -> (String, Vec<String>) {
    let named: Vec<String> = override_names(current).into_iter().map(|(n, _)| n).collect();
    let mut entries: Vec<String> = current.split(';').map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).collect();
    let mut kept = Vec::new();
    for dll in dlls {
        let s = stem(dll);
        if named.contains(&s) {
            kept.push(s);
        } else if !entries.iter().any(|e| e == &format!("{s}=n,b")) {
            entries.push(format!("{s}=n,b"));
        }
    }
    (entries.join(";"), kept)
}

/// `value` without the names in `dlls` (an entry left with no name goes).
pub fn strip_overrides(value: &str, dlls: &[&str]) -> String {
    let drop: Vec<String> = dlls.iter().map(|d| stem(d)).collect();
    value
        .split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (names, val) = match entry.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (entry, None),
            };
            let left: Vec<&str> = names.split(',').map(str::trim).filter(|n| !n.is_empty() && !drop.contains(&stem(n))).collect();
            match (left.is_empty(), val) {
                (true, _) => None,
                (false, Some(v)) => Some(format!("{}={v}", left.join(","))),
                (false, None) => Some(left.join(",")),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The names of `dlls` that the user's `WINEDLLOVERRIDES` sets.
pub fn user_overrides(env_options: &[(String, String)], dlls: &[&str]) -> Vec<String> {
    let Some((_, v)) = env_options.iter().find(|(k, _)| k.eq_ignore_ascii_case("WINEDLLOVERRIDES")) else { return Vec::new() };
    let named: Vec<String> = override_names(v).into_iter().map(|(n, _)| n).collect();
    dlls.iter().map(|d| stem(d)).filter(|d| named.contains(d)).collect()
}

/// At launch: when the game's ReShade is on and its DLL is in place, tells
/// Wine to load it (`wine`: Wine or Proton runs the game; on Windows the
/// file is enough). Gives the line for the launch log. Never fails: a
/// problem of ReShade must not keep the game from starting.
pub fn apply(env: &mut Env, paths: &DataPaths, game_id: &str, s: &ReShade, wine: bool) -> Option<String> {
    let m = manifest::load(paths, game_id)?;
    if !s.enabled || !m.enabled {
        return None;
    }
    // The plan is built before `manage::recover` puts a missing DLL back:
    // it counts as there when it will be.
    if !manage::will_load(paths, &m) {
        log::warn!("ReShade is on but {} is missing or not ReShade; starting without it", m.dir().join(&m.proxy).display());
        return None;
    }
    if wine {
        let mut dlls = vec![m.proxy.as_str()];
        if m.compiler.is_some() {
            dlls.push(install::COMPILER);
        }
        let current = env.get("WINEDLLOVERRIDES").cloned().unwrap_or_default();
        let (merged, kept) = merge_overrides(&current, &dlls);
        for k in kept {
            log::warn!("WINEDLLOVERRIDES already sets {k}; ReShade leaves it as it is");
        }
        env.insert("WINEDLLOVERRIDES".into(), merged);
    }
    let mut line = format!("ReShade {} ({}, {}) at {}", m.version, m.api, m.arch, m.target_dir);
    if !s.anticheat_ack.is_empty() {
        line.push_str(&format!("; the game uses {}, the user accepted the risk", s.anticheat_ack.join(", ")));
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SP: &str = "ddraw,steam_api64,dxgi,d3dcompiler_47=n,b;lsteamclient=d";

    #[test]
    fn settings_read_with_defaults() {
        let s = ReShade::from_map(json!({}).as_object().unwrap());
        assert_eq!(s, ReShade { api: "auto".into(), ..Default::default() });
        let s = ReShade::from_map(
            json!({"heroniteReshade": true, "heroniteReshadeApi": "d3d11", "heroniteReshadeAntiCheatAck": ["BattlEye"]})
                .as_object()
                .unwrap(),
        );
        assert!(s.enabled);
        assert_eq!((s.proxy_choice().as_deref(), s.anticheat_ack.as_slice()), (Some("d3d11.dll"), ["BattlEye".to_string()].as_slice()));
        assert_eq!(ReShade::from_map(json!({"heroniteReshadeApi": "vulkan"}).as_object().unwrap()).api, "auto");
    }

    #[test]
    fn overrides_merge_without_duplicates_and_keep_the_users() {
        assert_eq!(merge_overrides("", &["dxgi.dll"]), ("dxgi=n,b".to_string(), vec![]));
        assert_eq!(
            merge_overrides("winemenubuilder.exe=d", &["dxgi.dll", "d3dcompiler_47.dll"]).0,
            "winemenubuilder.exe=d;dxgi=n,b;d3dcompiler_47=n,b"
        );
        let (sp, kept) = merge_overrides(SP, &["dxgi.dll", "d3dcompiler_47.dll"]);
        assert_eq!((sp.as_str(), kept), (SP, vec!["dxgi".to_string(), "d3dcompiler_47".to_string()]));
        assert_eq!(merge_overrides("dxgi=n,b", &["dxgi.dll"]).0, "dxgi=n,b");
        assert_eq!(merge_overrides("a=n;", &["d3d9.dll"]).0, "a=n;d3d9=n,b");
    }

    #[test]
    fn the_users_overrides_can_be_cleaned_up() {
        assert_eq!(strip_overrides(SP, &["dxgi.dll", "d3dcompiler_47.dll"]), "ddraw,steam_api64=n,b;lsteamclient=d");
        assert_eq!(strip_overrides("dxgi=n,b", &["dxgi.dll"]), "");
        let env = vec![("WINEDLLOVERRIDES".to_string(), SP.to_string())];
        assert_eq!(user_overrides(&env, &["dxgi.dll", "d3d9.dll"]), ["dxgi"]);
        assert!(user_overrides(&[], &["dxgi.dll"]).is_empty());
    }
}
