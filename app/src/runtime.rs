//! Bridge between `heronite-launch` and the UI: builds plans, starts/stops
//! games and forwards launcher events to the UI thread.

use std::sync::Arc;

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use heronite_data::{CollectionGame, HeroicPaths};
use heronite_launch::steam::{open_steam_client_uri, SteamAction, SteamLocator};
use heronite_launch::{
    build_plan, LaunchEvent, LaunchOptions, LaunchPlan, LaunchRequest, Launcher, PlanContext, SystemProbe,
    SystemSpawner,
};

pub struct GameRuntime {
    launcher: Launcher,
    paths: HeroicPaths,
}

impl GameRuntime {
    pub fn new(paths: HeroicPaths) -> (Self, UnboundedReceiver<LaunchEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ftx, frx) = unbounded();
        std::thread::Builder::new()
            .name("launch-events".into())
            .spawn(move || {
                for ev in rx {
                    if ftx.unbounded_send(ev).is_err() {
                        break;
                    }
                }
            })
            .expect("spawning launch event thread");
        let ctx = PlanContext::load(&paths);
        let steam = SteamLocator::detect(&ctx.global, &SystemProbe);
        let launcher = Launcher::system(paths.clone(), steam, tx);
        (Self { launcher, paths }, frx)
    }

    pub fn is_running(&self, app_name: &str) -> bool {
        self.launcher.is_running(app_name)
    }

    /// Builds the plan (reads GamesConfig etc.) and starts the game.
    pub fn play(&self, game: &CollectionGame) -> Result<(), String> {
        self.play_emulated(game, None).map(|_| ())
    }

    /// Like [`play`](Self::play); for emulated games `rom` names the ROM to run
    /// and a save state chosen in the launch dialog (pending, one-shot) adds its
    /// arguments. Returns the ROM the plan runs.
    pub fn play_emulated(&self, game: &CollectionGame, rom: Option<String>) -> Result<Option<String>, String> {
        let ctx = PlanContext::load(&self.paths);
        let mut opts = LaunchOptions::default();
        if game.launch_kind.as_deref() == Some("emulator") {
            let wanted = rom.clone().or_else(|| game.meta.as_ref().and_then(|m| m.selected_rom_path.clone()));
            if let Some(state) =
                heronite_emulation::PendingLaunchStates::global().take(&game.app_name, wanted.as_deref())
            {
                opts.state_args = state.args;
                opts.state_replaces_image = state.replaces_image;
            }
            opts.extra_args = rom.into_iter().collect();
        }
        let plan = build_plan(&ctx, &SystemProbe, &game.game, game.meta.as_deref(), &opts);
        if let LaunchPlan::Unavailable(reason) = &plan {
            return Err(reason.clone());
        }
        let played_rom = match &plan {
            LaunchPlan::Emulator { rom, .. } => rom.clone(),
            _ => None,
        };
        log::info!("launching {} ({}): {plan:?}", game.title, game.app_name);
        self.launcher
            .launch(LaunchRequest {
                app_name: game.app_name.clone(),
                runner: game.runner.clone(),
                title: game.title.clone(),
                plan,
            })
            .map_err(|e| e.to_string())?;
        Ok(played_rom)
    }

    pub fn stop(&self, app_name: &str) -> bool {
        self.launcher.stop(app_name)
    }

    pub fn force_clear(&self, app_name: &str) -> bool {
        self.launcher.force_clear(app_name)
    }

    /// Install action for a game that is not installed.
    pub fn install(&self, game: &CollectionGame) -> Result<(), String> {
        if let Some(id) = game.steam_app_id.as_deref() {
            return open_steam_client_uri(&SystemSpawner::default(), &SystemProbe, SteamAction::Install, id);
        }
        Err(match game.runner.as_str() {
            "legendary" => "Installing Epic games is not available yet; install it in Heronite/Heroic.".into(),
            _ => "This game has no install action.".into(),
        })
    }
}

pub type SharedRuntime = Arc<GameRuntime>;
