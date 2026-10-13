//! Bridge between `heronite-launch` and the UI: builds plans, starts/stops
//! games and forwards launcher events to the UI thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use heronite_data::{CollectionGame, DataPaths};
use heronite_i18n::tr;
use heronite_launch::net::SystemNet;
use heronite_launch::prepare::CloudSync;
use heronite_launch::runtime::PrepServices;
use heronite_launch::steam::{open_steam_client_uri, SteamAction, SteamLocator};
use heronite_launch::{
    build_plan, LaunchEvent, LaunchOptions, LaunchPlan, LaunchRequest, Launcher, PlanContext, SystemProbe, SystemSpawner,
};

/// Cloud saves around a launch go through legendary (the Epic crate depends on
/// the launch crate, so the launcher gets this through a trait).
struct EpicCloudSaves(DataPaths);

impl CloudSync for EpicCloudSaves {
    fn sync(&self, _runner: &str, app_name: &str, saves_path: &str, flag: &str) -> Result<String, String> {
        heronite_epic::saves::sync_flag(&self.0, app_name, saves_path, Some(flag)).map_err(|e| format!("{e:#}"))
    }
}

/// What the post-game work needs to know about a running session. Filled in
/// when the game is launched and read when it ends, so that work does not
/// depend on any window (see `services::session`).
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub app_name: String,
    pub runner: String,
    pub title: String,
    pub steam_app_id: Option<String>,
    /// Names the screenshot matcher also tries (store title, install folder).
    pub aliases: Vec<String>,
    pub is_emulator: bool,
    /// ROM the emulator runs (emulated games).
    pub rom: Option<String>,
    /// The game's selected ROM in the overlay meta (fallback for `rom`).
    pub selected_rom: Option<String>,
}

pub struct GameRuntime {
    launcher: Launcher,
    paths: DataPaths,
    sessions: Mutex<HashMap<String, SessionInfo>>,
}

impl GameRuntime {
    pub fn new(paths: DataPaths) -> (Self, UnboundedReceiver<LaunchEvent>) {
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
        launcher.set_prep_services(PrepServices {
            probe: Arc::new(SystemProbe),
            net: Arc::new(SystemNet),
            cloud: Some(Arc::new(EpicCloudSaves(paths.clone()))),
        });
        launcher.set_epic_setup(Arc::new(crate::services::downloads::EpicLaunchSetup(paths.clone())));
        // Header of every launch log (Heroic's `formatSystemInfo`).
        launcher.set_system_info(Arc::new(crate::pages::settings::logs::system_info_text));
        spawn_tools_update(paths.clone());
        (Self { launcher, paths, sessions: Mutex::new(HashMap::new()) }, frx)
    }

    /// The running (or just started) session of a game.
    pub fn session_info(&self, app_name: &str) -> Option<SessionInfo> {
        self.sessions.lock().ok()?.get(app_name).cloned()
    }

    /// `HERONITE_DEBUG_FAKE_SESSION`: registers a session as if the game had
    /// been launched (nothing is started).
    pub fn debug_register_session(&self, game: &CollectionGame) {
        if let Ok(mut s) = self.sessions.lock() {
            s.insert(game.app_name.clone(), session_info(game, None));
        }
    }

    /// Takes the session out when it ended.
    pub fn take_session(&self, app_name: &str) -> Option<SessionInfo> {
        self.sessions.lock().ok()?.remove(app_name)
    }

    /// Builds the plan (reads GamesConfig etc.) and starts the game.
    pub fn play(&self, game: &CollectionGame) -> Result<(), String> {
        self.play_emulated(game, None).map(|_| ())
    }

    /// Like [`play`](Self::play); for emulated games `rom` names the ROM to run
    /// and a save state chosen in the launch dialog (pending, one-shot) adds its
    /// arguments. Returns the ROM the plan runs.
    pub fn play_emulated(&self, game: &CollectionGame, rom: Option<String>) -> Result<Option<String>, String> {
        self.launch_with(game, rom, LaunchOptions::default())
    }

    /// "Play anyway" for an Epic game with a pending update: legendary is told
    /// not to check the game version (`--skip-version-check`).
    pub fn play_skip_version_check(&self, game: &CollectionGame) -> Result<(), String> {
        let opts = LaunchOptions { skip_version_check: true, ..Default::default() };
        self.launch_with(game, None, opts).map(|_| ())
    }

    fn launch_with(&self, game: &CollectionGame, rom: Option<String>, mut opts: LaunchOptions) -> Result<Option<String>, String> {
        let ctx = PlanContext::load(&self.paths);
        if game.launch_kind.as_deref() == Some("emulator") {
            let wanted = rom.clone().or_else(|| game.meta.as_ref().and_then(|m| m.selected_rom_path.clone()));
            if let Some(state) = heronite_emulation::PendingLaunchStates::global().take(&game.app_name, wanted.as_deref()) {
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
        // Registered before the launch so the events find it; an "already
        // running" refusal must not disturb the session that is there.
        let previous = self.sessions.lock().ok().and_then(|mut s| s.insert(game.app_name.clone(), session_info(game, played_rom.clone())));
        let launched = self.launcher.launch(LaunchRequest {
            app_name: game.app_name.clone(),
            runner: game.runner.clone(),
            title: game.title.clone(),
            plan,
        });
        if let Err(e) = launched {
            if let Ok(mut s) = self.sessions.lock() {
                match previous {
                    Some(p) => s.insert(game.app_name.clone(), p),
                    None => s.remove(&game.app_name),
                };
            }
            return Err(e.to_string());
        }
        Ok(played_rom)
    }

    pub fn stop(&self, app_name: &str) -> bool {
        self.launcher.stop(app_name)
    }

    pub fn force_clear(&self, app_name: &str) -> bool {
        self.launcher.force_clear(app_name)
    }

    /// Saves the playtime of every running game now (the app is quitting).
    /// Blocking file I/O: off the UI thread. Returns how many were running.
    pub fn checkpoint_all(&self) -> usize {
        self.launcher.checkpoint_all()
    }

    /// Install action for a game that is not installed.
    pub fn install(&self, game: &CollectionGame) -> Result<(), String> {
        if let Some(id) = game.steam_app_id.as_deref() {
            return open_steam_client_uri(&SystemSpawner::default(), &SystemProbe, SteamAction::Install, id);
        }
        Err(match game.runner.as_str() {
            "legendary" => tr!("Epic games are installed from the install dialog.").into(),
            _ => tr!("This game has no install action.").into(),
        })
    }
}

fn session_info(game: &CollectionGame, rom: Option<String>) -> SessionInfo {
    SessionInfo {
        app_name: game.app_name.clone(),
        runner: game.runner.clone(),
        title: game.title.clone(),
        steam_app_id: game.steam_app_id.clone(),
        aliases: heronite_media::GameQuery::from_game(game).aliases,
        is_emulator: game.launch_kind.as_deref() == Some("emulator"),
        rom,
        selected_rom: game.meta.as_ref().and_then(|m| m.selected_rom_path.clone()),
    }
}

/// Game settings switch "Auto Install/Update DXVK / DXVK-NVAPI / VKD3D on
/// Prefix": installs or removes `tool` in the prefix of `settings` right away
/// (Heroic's `toggleDXVK`, `toggleDXVKNVAPI`, `toggleVKD3D`). Blocking and
/// slow (downloads, wine commands): call it from a thread. `progress` gets the
/// steps ("Downloading DXVK..."). Not run by tests.
pub fn toggle_tool_blocking(
    paths: &DataPaths,
    settings: &heronite_launch::settings::GameSettings,
    tool: heronite_launch::tools::Tool,
    action: heronite_launch::dxvk::Action,
    progress: &dyn Fn(String),
) -> Result<heronite_launch::dxvk::Outcome, String> {
    let ctx = PlanContext::load(paths);
    let (net, spawner, never) = (SystemNet, SystemSpawner::default(), || false);
    let notify = |n: heronite_launch::prepare::Notice| progress(n.message);
    let prep = heronite_launch::prepare::Prep {
        paths,
        probe: &SystemProbe,
        net: &net,
        spawner: &spawner,
        cloud: None,
        cancel: &never,
        notify: &notify,
    };
    prep.toggle_tool(&ctx, settings, tool, action)
}

/// Heroic refreshes DXVK, VKD3D, DXVK-NVAPI and winetricks a few seconds after
/// start (`DXVK.getLatest()`, `Winetricks.download()` in `main.ts`), so the
/// first launch that needs them finds them. Off with
/// `HERONITE_DISABLE_IDLE_JOBS` (tests against real data) or
/// `HERONITE_DISABLE_TOOLS_UPDATE` (only this).
fn spawn_tools_update(paths: DataPaths) {
    if crate::env::flag("HERONITE_DISABLE_IDLE_JOBS") || crate::env::flag("HERONITE_DISABLE_TOOLS_UPDATE") {
        return;
    }
    let spawned = std::thread::Builder::new().name("tools-update".into()).spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        // A new setup has no legendary yet: get it before the user logs in to Epic.
        if let Err(e) = heronite_launch::legendary_install::ensure(&paths, &heronite_launch::SystemProbe, &SystemNet, &|| false) {
            log::warn!("{e}");
        }
        heronite_launch::tools::update_all_if_due(&paths, &SystemNet, &|| false, &|m| log::info!("{m}"));
        // known_fixes.json (winetricks verbs, env variables) as Heroic keeps it.
        if let Err(e) = heronite_launch::known_fixes::update_if_needed(&paths, &SystemNet, &|| false) {
            log::warn!("{e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("could not start the tools update: {e}");
    }
}
