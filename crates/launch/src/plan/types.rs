//! The data a plan is made of: commands, stop specs, prep specs and [`LaunchPlan`].

use crate::env::Env;
use crate::settings::GameSettings;

/// A process to spawn. `env` holds only *additions* to the inherited
/// environment; `program` is run as given (resolved through `$PATH` if bare).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub env: Env,
    pub cwd: Option<String>,
}

impl Command {
    /// Shell-quoted one-liner for logs.
    pub fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        shlex::try_join(parts.iter().map(String::as_str)).unwrap_or_else(|_| parts.join(" "))
    }
}

/// What "stop" does besides killing the tracked process tree: it
/// terminates the programs its patterns name (see [`crate::procs::program_matches`])
/// and shuts the Wine prefix down.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StopSpec {
    pub kill_patterns: Vec<String>,
    pub commands: Vec<Command>,
}

/// How a Windows program is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WineRuntime {
    /// `umu-run` with `PROTONPATH` (Proton inside the Steam Runtime).
    Umu,
    /// `proton waitforexitandrun <exe>`.
    Proton,
    /// `wine <exe>`.
    Wine,
}

/// Anything that is spawned as a child process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessPlan {
    pub command: Command,
    pub stop: StopSpec,
    /// Directories to create first (the Wine prefix).
    pub create_dirs: Vec<String>,
    /// Run to completion, in order, before `command` (prefix creation).
    pub pre_commands: Vec<Command>,
    /// Run after `command` exits (`wineserver --wait`).
    pub wait_after_exit: Option<Command>,
    /// The user's script run to completion right before `command`.
    pub before_script: Option<Command>,
    /// The user's script run to completion after the game is gone.
    pub after_script: Option<Command>,
    /// Program to `chmod +x` before launching (Heroic does this too).
    pub make_executable: Option<String>,
    /// Human-readable decisions worth logging (e.g. fell back to default Wine).
    pub notes: Vec<String>,
    /// What Heroic's `prepareLaunch` / `prepareWineLaunch` and the cloud-save
    /// sync around a launch need before and after the process runs.
    pub prep: PrepSpec,
    /// Programs started while the game runs and closed with it.
    pub companions: Vec<CompanionPlan>,
    /// Holds the "do not suspend" lock while the game runs (see
    /// [`crate::keep_awake`]).
    pub keep_awake: Option<Command>,
    /// legendary starts the game through umu (a Wine plan says it with
    /// [`WineRuntime::Umu`]): its `GAMEID` picks the protonfixes.
    pub through_umu: bool,
}

/// A program run alongside the game (see [`crate::companions`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionPlan {
    /// The file the user picked (the command may run it through Wine).
    pub path: String,
    pub command: Command,
    /// Time after the game started.
    pub delay: std::time::Duration,
}

/// Work around a launch that needs the network, the tools folder or the
/// Wine prefix (run by [`crate::prepare`] on the launch thread).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrepSpec {
    /// The umu runtime is wanted but not installed: download it first.
    pub umu: bool,
    /// Effective settings of a game run through Wine/Proton: drives the
    /// DXVK / DXVK-NVAPI / VKD3D install into the prefix and the EAC /
    /// BattlEye runtime downloads.
    pub wine: Option<GameSettings>,
    /// Set when `autoSyncSaves` is on: cloud saves are downloaded before the
    /// game starts and uploaded after it exits.
    pub cloud_saves: Option<CloudSaves>,
    /// Epic game run through Wine: what Heroic does the first time the game
    /// starts in its prefix (`legendarySetup`, `installFixes`).
    pub first_launch: Option<FirstLaunch>,
}

/// What a game needs the first time it runs in a Wine prefix (the prefix
/// remembers it in its `installed_games` file), ported from `legendarySetup`
/// and `installFixes`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FirstLaunch {
    /// `legendary`, `gog` or `nile`: which store's setup runs.
    pub runner: String,
    pub app_name: String,
    pub install_path: String,
    /// `install.platform` of the game.
    pub platform: String,
    pub ea_managed: bool,
    pub ubisoft_managed: bool,
    /// `known_fixes.json`: winetricks verbs.
    pub winetricks: Vec<String>,
    /// `known_fixes.json`: programs (relative to the install folder) to run
    /// in the prefix.
    pub run_in_prefix: Vec<String>,
    /// Installers the store names for the game (Amazon's `PostInstall`), run
    /// before the store setup asks for more ([`crate::first_launch::StoreSetup`]).
    pub post_install: Vec<SetupStep>,
}

/// What a cloud save sync around a launch works on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CloudSaves {
    /// `legendary` or `gog`: the store that keeps the saves.
    pub runner: String,
    /// Epic: the `savesPath` folder. GOG: empty (the store's own locations
    /// are used).
    pub path: String,
}

impl CloudSaves {
    pub fn new(runner: &str, path: &str) -> Self {
        Self { runner: runner.to_string(), path: path.to_string() }
    }
}

/// A Windows program run in the prefix during the first-launch setup.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetupStep {
    /// Program (a Linux path or a Wine built-in such as `msiexec`) and its arguments.
    pub parts: Vec<String>,
    /// Folder it starts in.
    pub cwd: Option<String>,
}

impl PrepSpec {
    pub fn is_empty(&self) -> bool {
        *self == PrepSpec::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPlan {
    /// Open `steam://rungameid/<id>`; the game process is found in Steam's logs.
    SteamUri { app_id: String, uri: String },
    /// A Linux program started directly (with Heroic's wrappers and env).
    Native(ProcessPlan),
    /// A Windows program through umu/Proton/Wine.
    Wine { process: ProcessPlan, runtime: WineRuntime, prefix: String },
    /// An emulator with its ROM.
    Emulator { process: ProcessPlan, emulator: String, rom: Option<String> },
    /// `legendary launch <app>` (Epic).
    Legendary { process: ProcessPlan, app_name: String },
    /// Cannot be launched; the string says why.
    Unavailable(String),
}

impl LaunchPlan {
    pub fn process(&self) -> Option<&ProcessPlan> {
        match self {
            LaunchPlan::Native(p)
            | LaunchPlan::Wine { process: p, .. }
            | LaunchPlan::Emulator { process: p, .. }
            | LaunchPlan::Legendary { process: p, .. } => Some(p),
            LaunchPlan::SteamUri { .. } | LaunchPlan::Unavailable(_) => None,
        }
    }

    pub fn process_mut(&mut self) -> Option<&mut ProcessPlan> {
        match self {
            LaunchPlan::Native(p)
            | LaunchPlan::Wine { process: p, .. }
            | LaunchPlan::Emulator { process: p, .. }
            | LaunchPlan::Legendary { process: p, .. } => Some(p),
            LaunchPlan::SteamUri { .. } | LaunchPlan::Unavailable(_) => None,
        }
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        match self {
            LaunchPlan::Unavailable(r) => Some(r),
            _ => None,
        }
    }

    /// Short stable label for logs and UI ("steam", "native", ...).
    pub fn kind(&self) -> &'static str {
        match self {
            LaunchPlan::SteamUri { .. } => "steam",
            LaunchPlan::Native(_) => "native",
            LaunchPlan::Wine { .. } => "wine",
            LaunchPlan::Emulator { .. } => "emulator",
            LaunchPlan::Legendary { .. } => "legendary",
            LaunchPlan::Unavailable(_) => "unavailable",
        }
    }
}
