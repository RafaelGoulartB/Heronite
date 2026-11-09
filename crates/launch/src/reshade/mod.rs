//! ReShade (<https://reshade.me>) per game: post-processing effects added
//! by a DLL the game loads in place of a graphics one (`dxgi.dll`,
//! `d3d9.dll`, `opengl32.dll`, `ddraw.dll`). It loads the real DLL and runs
//! shaders on the image before it reaches the screen.
//!
//! Heronite does what ReShade's installer does: picks the executable and
//! its API, copies the DLL under the right name, adds shader packages and
//! a preset, writes `ReShade.ini` ([`install`], [`manage`]). Everything is
//! downloaded from the official sources when the user installs
//! ([`fetch`]); nothing ships with the app. The standard build is
//! installed; the build with add-on support (which RenoDX's HDR mods need,
//! [`crate::gfxmods::renodx`]) is a switch per game, refused for games with
//! anti-cheat ([`manage::set_addon`]).
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

use crate::gfxmods::anticheat;
pub use crate::gfxmods::anticheat::{Finding, Risk, Source};
pub use crate::gfxmods::overrides::{merge_overrides, strip_overrides, user_overrides};
use crate::gfxmods::{GraphicsMod, Launch};
pub use overview::{fits, overview, Facts, Overview, SourceRow, State};

use crate::env::Env;
use crate::settings::GameSettings;

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

/// ReShade as a [`GraphicsMod`].
pub struct ReShadeMod;

impl GraphicsMod for ReShadeMod {
    fn id(&self) -> &'static str {
        "reshade"
    }

    fn name(&self) -> &'static str {
        "ReShade"
    }

    /// When the game's ReShade is on and its DLL is in place: the DLL, and
    /// `d3dcompiler_47` when Heronite put it there.
    fn launch(&self, paths: &DataPaths, game_id: &str, s: &GameSettings) -> Option<Launch> {
        launch(paths, game_id, &s.reshade)
    }
}

/// At launch: when the game's ReShade is on and its DLL is in place, tells
/// Wine to load it (`wine`: Wine or Proton runs the game; on Windows the
/// file is enough). Gives the line for the launch log. Never fails: a
/// problem of ReShade must not keep the game from starting.
/// [`crate::gfxmods::apply`] does this for every mod.
pub fn apply(env: &mut Env, paths: &DataPaths, game_id: &str, s: &ReShade, wine: bool) -> Option<String> {
    let launch = launch(paths, game_id, s)?;
    if wine {
        crate::gfxmods::add_overrides(env, &launch.dlls, ReShadeMod.name());
    }
    Some(launch.note)
}

fn launch(paths: &DataPaths, game_id: &str, s: &ReShade) -> Option<Launch> {
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
    let mut dlls = vec![m.proxy.clone()];
    if m.compiler.is_some() {
        dlls.push(install::COMPILER.to_string());
    }
    let mut note = format!("ReShade {} ({}, {}) at {}", m.version, m.api, m.arch, m.target_dir);
    if !s.anticheat_ack.is_empty() {
        note.push_str(&format!("; the game uses {}, the user accepted the risk", s.anticheat_ack.join(", ")));
    }
    Some(Launch { dlls, note })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
