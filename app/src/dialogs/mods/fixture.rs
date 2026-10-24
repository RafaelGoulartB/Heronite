//! Debug only: a manager with content when the service is a stub or has no
//! data folder to read.
//!
//! `HERONITE_DEBUG_MODS_FIXTURE=1` (or a `,`-separated list of `nodir`,
//! `steam`, `account`, `premium`, `fuse`, `jobs`, `running`, `native`,
//! `anticheat`, `deleted`, `misplaced`, `loader`, `loader-on`, `loader-steam`) shows the
//! `mods.json` of the game under the data folder, with invented conflicts, an
//! overwrite folder and jobs, for as long as the service has not loaded the game.

use std::collections::HashSet;
use std::sync::Arc;

use heronite_data::CollectionGame;
use heronite_mods::conflicts::{Conflicts, ModConflicts};
use heronite_mods::games::{self, Profile};
use heronite_mods::nexus::{Account, StoredAccount};
use heronite_mods::overwrite::Written;
use heronite_mods::{store, ModsRoot, Progress};

use super::DialogCtx;
use crate::services::mods::{GameModInfo, GameMods, Job, JobStage, LoaderCard, LoaderStart};

pub(super) struct Fixture {
    pub game: GameMods,
    pub jobs: Vec<Job>,
    pub account: Option<StoredAccount>,
    pub support_problem: Option<String>,
    pub misplaced: HashSet<String>,
}

pub(super) fn load(ctx: &DialogCtx, game: &CollectionGame) -> Option<Fixture> {
    let spec = crate::env::value("HERONITE_DEBUG_MODS_FIXTURE").filter(|v| v != "0")?;
    let has = |flag: &str| spec.split(',').any(|f| f == flag);
    let root = ModsRoot::default_for(&ctx.data_paths);
    let mut list = store::load(&root.game(&game.key));
    list.game_key = game.key.clone();
    list.title = game.title.clone();
    let known = games::find(game.steam_app_id.as_deref(), &game.title);
    let profile = Profile::of(&list.links, known);
    let info = GameModInfo {
        title: game.title.clone(),
        game_dir: (!has("nodir")).then(|| std::path::PathBuf::from("/games/fixture")),
        prefix: None,
        steam_app_id: game.steam_app_id.clone(),
        through_steam: has("steam"),
        native: has("native"),
        known,
        profile,
        nexus_domain: list.links.nexus_domain.clone().or_else(|| known.and_then(|k| k.nexus_domain.map(str::to_string))),
        thunderstore: list.links.thunderstore.clone().or_else(|| known.and_then(|k| k.thunderstore.map(str::to_string))),
        anticheat: if has("anticheat") { vec!["EAC".to_string()] } else { Vec::new() },
    };
    let enabled: Vec<String> = list.enabled().map(|m| m.id.clone()).collect();
    let mut per_mod = std::collections::HashMap::new();
    for id in &enabled {
        per_mod.insert(id.clone(), ModConflicts::default());
    }
    // Every third mod wins files from the one before it.
    for k in (1..enabled.len()).step_by(3) {
        let (loser, winner) = (&enabled[k - 1], &enabled[k]);
        let n = 4 + k * 3;
        per_mod.get_mut(winner).unwrap().overrides.push((loser.clone(), n));
        let l = per_mod.get_mut(loser).unwrap();
        l.overridden_by.push((winner.clone(), n));
        l.fully_overridden = k == 4;
    }
    let jobs = if has("jobs") { jobs(&game.key) } else { Vec::new() };
    let account = if has("premium") || has("account") {
        let account = Account { name: "Rafa".into(), user_id: 1, is_premium: has("premium"), ..Default::default() };
        Some(StoredAccount::new("fixture", account))
    } else {
        None
    };
    Some(Fixture {
        game: GameMods {
            info,
            list,
            conflicts: Some(Arc::new(Conflicts { per_mod })),
            overwrite: if has("deleted") {
                Written { files: 0, bytes: 0, deleted: 2 }
            } else {
                Written { files: 23, bytes: 1_258_291, deleted: 2 }
            },
            loaders: loaders(&has),
            steam_overrides_missing: has("steam").then(|| "winhttp=n,b".to_string()),
        },
        jobs,
        account,
        support_problem: has("fuse").then(|| "Heronite needs FUSE (fusermount3) to apply mods. Install the fuse3 package.".to_string()),
        misplaced: if has("misplaced") { enabled.iter().take(1).cloned().collect() } else { HashSet::new() },
    })
}

fn jobs(key: &str) -> Vec<Job> {
    let job = |id, name: &str, stage, done, total, error: Option<&str>| Job {
        id,
        game_key: key.to_string(),
        name: name.to_string(),
        stage,
        progress: Progress { done, total },
        error: error.map(str::to_string),
        dependencies: 0,
    };
    let mut done = job(3, "Better Combat", JobStage::Done, 0, 0, None);
    done.dependencies = 2;
    vec![
        job(1, "Unofficial Patch", JobStage::Downloading, 18_400_000, 46_200_000, None),
        job(2, "Face Overhaul", JobStage::Failed, 0, 0, Some("The download was interrupted. Try again.")),
        done,
    ]
}

/// A script extender in the first mod, in the state the flags ask for.
fn loaders(has: &dyn Fn(&str) -> bool) -> Vec<LoaderCard> {
    if !(has("loader") || has("loader-on") || has("loader-steam")) {
        return Vec::new();
    }
    let start = match (has("loader-steam"), has("loader-on")) {
        (true, _) => {
            LoaderStart::Steam { line: Some("bash -c 'exec \"${@/SkyrimSELauncher.exe/skse64_loader.exe}\"' -- %command%".to_string()) }
        }
        (_, true) => LoaderStart::On,
        _ => LoaderStart::Off,
    };
    vec![LoaderCard {
        name: "SKSE",
        file: "skse64_loader.exe".to_string(),
        rel: "skse64_loader.exe".to_string(),
        mod_name: "SKSE64".to_string(),
        needs_arguments: false,
        start,
    }]
}
