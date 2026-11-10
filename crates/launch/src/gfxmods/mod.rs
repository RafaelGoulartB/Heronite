//! Graphics mods per game: DLLs and files a Windows game loads by itself
//! from its folder. ReShade ([`crate::reshade`]) is the first one.
//!
//! Heronite only places files on disk and sets launch variables; it never
//! injects into, hooks or patches a game. Each mod implements
//! [`GraphicsMod`], and [`MODS`] lists them in the order they apply at
//! launch. What they share lives here: [`journal`] undoes a failed
//! operation, [`overrides`] edits `WINEDLLOVERRIDES`, [`anticheat`] tells
//! the risk, [`download`] and [`hash`] fetch and check files, [`catalog`]
//! finds the mods made for a game. [`renodx`] adds RenoDX's HDR mods to a
//! game's ReShade.

pub mod anticheat;
pub mod catalog;
pub(crate) mod download;
pub mod hash;
pub(crate) mod journal;
pub mod overrides;
pub mod renodx;

use heronite_data::DataPaths;

use crate::env::Env;
use crate::settings::GameSettings;

/// What a mod needs from a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// DLL names the game must load from its folder (Wine and Proton need
    /// `<dll>=n,b` for each).
    pub dlls: Vec<String>,
    /// The line for the launch log.
    pub note: String,
}

pub trait GraphicsMod: Sync {
    /// Stable key, for logs and the mod's data.
    fn id(&self) -> &'static str;
    /// The name users know it by.
    fn name(&self) -> &'static str;
    /// What the game loads at this launch: `None` when the mod is off, not
    /// installed, or its files are missing (the game starts without it).
    fn launch(&self, paths: &DataPaths, game_id: &str, s: &GameSettings) -> Option<Launch>;
}

pub static MODS: &[&dyn GraphicsMod] = &[&crate::reshade::ReShadeMod];

/// At launch: the mods that are on tell Wine to load their DLLs (`wine`:
/// Wine or Proton runs the game; on Windows the files are enough). Gives the
/// lines for the launch log. Never fails: a problem of a mod must not keep
/// the game from starting.
pub fn apply(env: &mut Env, paths: &DataPaths, game_id: &str, s: &GameSettings, wine: bool) -> Vec<String> {
    let mut notes = Vec::new();
    for m in MODS {
        let Some(launch) = m.launch(paths, game_id, s) else { continue };
        if wine {
            add_overrides(env, &launch.dlls, m.name());
        }
        notes.push(launch.note);
    }
    notes
}

/// Adds `<dll>=n,b` for `dlls` to the launch's `WINEDLLOVERRIDES`; a name
/// the user already sets keeps its value (`name`: the mod, for the log).
pub(crate) fn add_overrides(env: &mut Env, dlls: &[String], name: &str) {
    let dlls: Vec<&str> = dlls.iter().map(String::as_str).collect();
    let current = env.get("WINEDLLOVERRIDES").cloned().unwrap_or_default();
    let (merged, kept) = overrides::merge_overrides(&current, &dlls);
    for k in kept {
        log::warn!("WINEDLLOVERRIDES already sets {k}; {name} leaves it as it is");
    }
    env.insert("WINEDLLOVERRIDES".into(), merged);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = MODS.iter().map(|m| m.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODS.len());
    }

    #[test]
    fn the_launch_is_reshades_own() {
        use crate::reshade::install::install;
        use crate::reshade::install::tests::{game, net, ops, request};
        use crate::reshade::ReShade;
        let g = game();
        let net = net();
        install(&ops(&g.paths, &net), &request(&g, None), None).unwrap();
        let reshade = ReShade { enabled: true, api: "auto".into(), anticheat_ack: vec!["BattlEye".into()], ..Default::default() };
        let mut s = GameSettings { reshade, ..Default::default() };
        let same = |s: &GameSettings| {
            for wine in [true, false] {
                let mut env = Env::new();
                env.insert("WINEDLLOVERRIDES".into(), "dxgi=b;winemenubuilder.exe=d".into());
                let mut own = env.clone();
                let notes = apply(&mut env, &g.paths, "sp", s, wine);
                let note = crate::reshade::apply(&mut own, &g.paths, "sp", &s.reshade, wine);
                assert_eq!((notes, env), (note.into_iter().collect(), own));
            }
        };
        same(&s);
        // With Heronite's d3dcompiler_47 beside it, that DLL is overridden too.
        let mut m = crate::reshade::manifest::load(&g.paths, "sp").unwrap();
        m.compiler = Some("fxc2".into());
        crate::reshade::manifest::save(&g.paths, "sp", &m).unwrap();
        same(&s);
        let mut env = Env::new();
        apply(&mut env, &g.paths, "sp", &s, true);
        assert_eq!(env["WINEDLLOVERRIDES"], "dxgi=n,b;d3dcompiler_47=n,b");
        s.reshade.enabled = false;
        let mut env = Env::new();
        assert!(apply(&mut env, &g.paths, "sp", &s, true).is_empty() && env.is_empty());
    }
}
