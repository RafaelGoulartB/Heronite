//! The sandbox (`heroniteSandbox`, a Heronite key of the game's settings):
//! everything a game runs, and everything run in its Wine prefix, goes
//! through bubblewrap (`bwrap`). Inside, the program sees the system
//! read-only, its prefix, its game folder (read-only), the graphics card,
//! the display, the sound and the controllers. It has no network and no home
//! folder: `$HOME` is a private folder of the sandbox
//! (`<data>/sandbox/<key>`, see [`private_home`]), so saves, settings and
//! shader caches written there stay between runs.
//!
//! Each [`Exception`] lets one thing through for games that need it.
//!
//! How it runs: [`script`] builds a small `sh` program that creates the
//! folders, looks up what depends on the session (display sockets, the
//! controllers plugged in) and starts `bwrap`. It goes in the command line
//! after the wrappers that stay outside (gamescope, GameMode, MangoHud) and
//! before Proton / the game (umu does not run inside, see [`without_umu`]).
//! Inside, the command runs in its own pid
//! namespace (otherwise `/proc/<pid>/root` of any process of the user shows
//! the real files), and the sandbox lasts until its last process exits.
//!
//! Programs started while the game runs (programs alongside the game, Wine
//! tools) join its sandbox with `nsenter` ([`join`]) so they share its
//! wineserver.

mod host;
mod mounts;
mod script;

use serde_json::{json, Map, Value};

use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::env::Env;
use crate::plan::{Command, PlanContext, StopSpec};
use crate::probe::Probe;
use crate::settings::GameSettings;

pub use host::{detect, find_sandbox, join, Bwrap, OVERLAY_VERSION};
pub use mounts::Mounts;

pub const KEY: &str = "heroniteSandbox";
/// Set inside the sandbox, so scripts and tools can tell.
pub const ENV_INSIDE: &str = "HERONITE_SANDBOX";
/// How long after its command starts a sandbox is looked for (see
/// [`find_sandbox`]); past that it is taken as not there.
pub const FOUND_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

/// What can be let through, one at a time. Each one makes some games work
/// and costs a part of the isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Exception {
    /// Internet and the local network.
    Network,
    /// Writing to the game's folder (read-only otherwise).
    GameFolder,
    /// The folders of "Folders visible to Proton" (`heroniteSharedFolders`).
    SharedFolders,
    /// Every device, not only the graphics card and the controllers.
    Devices,
}

impl Exception {
    pub const ALL: [Exception; 4] = [Exception::Network, Exception::GameFolder, Exception::SharedFolders, Exception::Devices];

    /// Key inside the setting's object.
    pub fn key(self) -> &'static str {
        match self {
            Exception::Network => "network",
            Exception::GameFolder => "gameFolder",
            Exception::SharedFolders => "sharedFolders",
            Exception::Devices => "devices",
        }
    }
}

/// The setting: `{"enabled": true, "network": false, "gameFolder": false, ...}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sandbox {
    pub enabled: bool,
    pub network: bool,
    pub game_folder: bool,
    pub shared_folders: bool,
    pub devices: bool,
}

impl Sandbox {
    pub fn from_map(m: &Map<String, Value>) -> Self {
        Self::from_value(m.get(KEY))
    }

    pub fn from_value(v: Option<&Value>) -> Self {
        let Some(Value::Object(o)) = v else { return Self::default() };
        let b = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        Self {
            enabled: b("enabled"),
            network: b(Exception::Network.key()),
            game_folder: b(Exception::GameFolder.key()),
            shared_folders: b(Exception::SharedFolders.key()),
            devices: b(Exception::Devices.key()),
        }
    }

    pub fn to_value(self) -> Value {
        let mut o = json!({ "enabled": self.enabled });
        for e in Exception::ALL {
            o[e.key()] = Value::Bool(self.allows(e));
        }
        o
    }

    pub fn allows(self, e: Exception) -> bool {
        match e {
            Exception::Network => self.network,
            Exception::GameFolder => self.game_folder,
            Exception::SharedFolders => self.shared_folders,
            Exception::Devices => self.devices,
        }
    }

    pub fn with(mut self, e: Exception, on: bool) -> Self {
        match e {
            Exception::Network => self.network = on,
            Exception::GameFolder => self.game_folder = on,
            Exception::SharedFolders => self.shared_folders = on,
            Exception::Devices => self.devices = on,
        }
        self
    }
}

/// Whose private home a sandbox gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner<'a> {
    /// Programs in a Wine prefix: every game and tool of the prefix share it.
    Prefix(&'a str),
    /// A Linux game (by app name).
    Game(&'a str),
}

/// The folder the sandbox shows as `$HOME` (see `DataPaths::sandbox_home_of_prefix`).
pub fn private_home(paths: &DataPaths, owner: Owner) -> String {
    let dir = match owner {
        Owner::Prefix(prefix) => paths.sandbox_home_of_prefix(prefix),
        Owner::Game(app) => paths.sandbox_home_of_game(app),
    };
    dir.to_string_lossy().into_owned()
}

/// What one sandboxed command needs besides the system.
#[derive(Debug, Clone, Default)]
pub struct Request<'a> {
    /// The game's folder: read-only unless [`Exception::GameFolder`].
    pub folder: Option<&'a str>,
    /// Other folders it reads (the program of "Run EXE", the programs
    /// alongside the game).
    pub read: Vec<String>,
    /// The command's own variables (the frame generation DLL...).
    pub env: Option<&'a Env>,
}

/// Why the sandbox cannot run here, or `None` when it can (also used by the
/// settings to show the state).
pub fn unavailable(probe: &dyn Probe) -> Option<String> {
    match probe.bwrap() {
        None => Some(tr!("The sandbox needs bubblewrap (bwrap), which is not installed. Install the bubblewrap package, or turn the sandbox off for this game.").into()),
        Some(b) if !b.supported() => Some(tr!(
            "bubblewrap {version} is too old for the sandbox (0.5 or newer is needed). Update it, or turn the sandbox off for this game.",
            version = b.version_text()
        )),
        Some(b) => b.error.clone().map(|e| {
            tr!("bubblewrap cannot create a sandbox on this system ({error}). Turn the sandbox off for this game, or allow unprivileged user namespaces.", error = e)
        }),
    }
}

/// The `sh -c` program that starts what follows it in the sandbox of
/// `settings` (see [`wrap`]). `Ok(None)`: the sandbox is off. `Err`: it is
/// on but cannot run, and nothing may run outside it.
pub fn prelude(
    ctx: &PlanContext,
    probe: &dyn Probe,
    settings: &GameSettings,
    owner: Owner,
    req: &Request,
) -> Result<Option<Vec<String>>, String> {
    if !settings.sandbox.enabled {
        return Ok(None);
    }
    if let Some(why) = unavailable(probe) {
        return Err(why);
    }
    let bwrap = probe.bwrap().expect("checked by unavailable");
    let mounts = Mounts::build(ctx, probe, settings, owner, req, &bwrap)?;
    Ok(Some(script::prelude(&bwrap, &settings.sandbox, &mounts)))
}

/// `cmd` started by `prelude` (from [`prelude`]): the program and its
/// arguments become the prelude's own.
pub fn wrap(cmd: Command, prelude: &[String]) -> Command {
    let Some((program, rest)) = prelude.split_first() else { return cmd };
    let mut args = rest.to_vec();
    args.push(cmd.program);
    args.extend(cmd.args);
    Command { program: program.clone(), args, env: cmd.env, cwd: cmd.cwd }
}

/// [`wrap`] for a Wine command of the prefix of `settings` (the Wine
/// tools, the first-launch setup, prefix creation); unchanged when the
/// sandbox is off.
pub fn wrap_in_prefix(
    ctx: &PlanContext,
    probe: &dyn Probe,
    settings: &GameSettings,
    cmd: Command,
    req: Request,
) -> Result<Command, String> {
    let env = cmd.env.clone();
    let req = Request { env: Some(&env), ..req };
    match prelude(ctx, probe, settings, Owner::Prefix(&settings.wine_prefix), &req)? {
        Some(p) => Ok(wrap(cmd, &p)),
        None => Ok(cmd),
    }
}

/// The parts of a plan around its command, for a game in the sandbox
/// (`prelude`; unchanged when `None`): prefix creation runs in it too; no
/// `wineserver --wait` (the sandbox lasts until its last process ends) and no
/// stop commands (they would start a sandbox of their own, with another
/// wineserver: stopping the game's processes ends its sandbox).
pub fn confine(
    prelude: Option<&[String]>,
    pre_commands: Vec<Command>,
    wait_after_exit: Option<Command>,
    stop: StopSpec,
) -> (Vec<Command>, Option<Command>, StopSpec) {
    match prelude {
        None => (pre_commands, wait_after_exit, stop),
        Some(p) => (pre_commands.into_iter().map(|c| wrap(c, p)).collect(), None, StopSpec { commands: Vec::new(), ..stop }),
    }
}

/// Why umu does not run in the sandbox, so its games run Proton directly
/// (and Linux games without the Steam runtime). Its container builds a copy
/// of its Steam runtime on every start (pressure-vessel, hundreds of MiB);
/// shown read-only that cannot work, shown writable a program in the
/// sandbox could change the runtime every game outside uses, and in a
/// throwaway layer the copy is held in memory while the game runs.
pub fn without_umu() -> String {
    tr!("In the sandbox, Proton runs directly, without umu's Steam runtime container").into()
}

/// The command [`wrap`] put in the sandbox.
pub fn inner(cmd: &Command) -> Option<Command> {
    if !is_wrapped(cmd) {
        return None;
    }
    let (program, args) = cmd.args[script::ARGS..].split_first()?;
    Some(Command { program: program.clone(), args: args.to_vec(), env: cmd.env.clone(), cwd: cmd.cwd.clone() })
}

/// A command of the prefix's tools (from [`wrap`]) run in the sandbox that
/// already runs in `prefix`, when one does: a second sandbox would start a
/// second wineserver in the same prefix. `Err` when `file` (the program
/// the user picked) is outside that sandbox. Reads `/proc`; not run by tests.
pub fn into_running(cmd: Command, prefix: &str, file: Option<&str>) -> Result<Command, String> {
    // Linux programs start a sandbox of their own: no wineserver to share.
    if !cmd.env.contains_key("WINEPREFIX") {
        return Ok(cmd);
    }
    let Some(inner) = inner(&cmd) else { return Ok(cmd) };
    let Some(pid) = host::running_in_prefix(prefix) else { return Ok(cmd) };
    if let Some(path) = file.filter(|p| !host::visible_in(pid, p)) {
        return Err(tr!("The game is running in its sandbox, which cannot see {path}. Close the game to run it.", path));
    }
    Ok(join(&inner, pid))
}

/// Whether `cmd` was started by [`wrap`].
pub fn is_wrapped(cmd: &Command) -> bool {
    script::starts_with_prelude(&cmd.program, &cmd.args)
}

#[cfg(test)]
mod tests;
