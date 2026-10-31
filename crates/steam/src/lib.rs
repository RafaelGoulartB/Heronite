//! Settings of Steam games. Steam keeps launching them (`steam://rungameid`)
//! with its own Proton and prefix; this crate only reads and changes what
//! Steam stores per game:
//!
//! * the launch options ([`options`] turns the line into game settings and
//!   back; [`wrap`] is the script behind the before/after scripts),
//! * the compatibility tool (Proton build) forced for the game.
//!
//! While Steam runs, its CEF debugging interface ([`client`]) is the only
//! way: Steam keeps the settings in memory and rewrites its files. With
//! Steam closed, the files are edited in place ([`files`], [`vdf_edit`]).
//!
//! [`library`] reads the account's library from the client and joins it to
//! the Collection.

pub mod client;
pub mod files;
pub mod library;
pub mod options;
pub mod vdf_edit;
pub mod wrap;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use heronite_i18n::tr;
use heronite_launch::steam::{desktop_protocol_env, flatpak_steam_root, SteamLocator};
use heronite_shortcuts::steam::{enable_client_api_on_next_start, is_steam_running};
use heronite_shortcuts::steam_client::{webhelper, SteamClient};

/// A compatibility tool as Steam names it: `name` is what it stores
/// (`proton_9`, `GE-Proton9-20`), `display` what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatTool {
    pub name: String,
    pub display: String,
    /// The build's folder, when it is known from disk.
    pub path: Option<PathBuf>,
}

/// The kinds of tool, as the menu groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolGroup {
    /// GE-Proton, the Wine Manager's and other builds in `compatibilitytools.d`.
    Custom,
    /// Valve's Proton.
    Valve,
    /// Steam Linux Runtime and the legacy runtime (Linux games).
    Runtime,
}

impl CompatTool {
    pub fn group(&self) -> ToolGroup {
        let n = self.name.to_ascii_lowercase();
        if n.starts_with("steamlinuxruntime") || n == "legacysteamruntime" {
            ToolGroup::Runtime
        } else if n.starts_with("proton_") {
            ToolGroup::Valve
        } else {
            ToolGroup::Custom
        }
    }
}

/// The display name of the tool Steam calls `name` (its logs and mappings
/// write some with dashes: `proton-experimental`).
fn display_of(tools: &[CompatTool], name: &str) -> Option<String> {
    let norm = |s: &str| s.to_ascii_lowercase().replace('-', "_");
    tools.iter().find(|t| norm(&t.name) == norm(name)).map(|t| t.display.clone())
}

/// Custom builds (GE-Proton, the Wine Manager's...) first, by name; then
/// Valve's Proton, Experimental and Hotfix before the newest versions; the
/// runtimes last. The client lists them in no useful order.
fn order_tools(tools: &mut [CompatTool]) {
    let version = |t: &CompatTool| -> (u32, u32) {
        let v = t.display.strip_prefix("Proton ").and_then(|r| r.split(['-', ' ']).next()).unwrap_or("");
        let mut parts = v.split('.').map(|p| p.parse().unwrap_or(0));
        (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
    };
    let rank = |t: &CompatTool| match t.name.as_str() {
        "proton_experimental" => 1,
        "proton_hotfix" => 2,
        n if n.starts_with("proton_") => 3,
        n if n.starts_with("steamlinuxruntime") => 4,
        "legacysteamruntime" => 5,
        _ => 0,
    };
    tools.sort_by_cached_key(|t| (rank(t), std::cmp::Reverse(version(t)), t.display.to_lowercase()));
}

/// How the settings can be changed right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Through the running client; changes apply at once.
    Client,
    /// Steam is closed; its files are edited.
    Files,
    /// Steam runs without its debugging interface. It was turned on for the
    /// next start; until Steam restarts or closes nothing can change.
    ReadOnly,
}

/// What Steam has for one game.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub launch_options: String,
    /// `""`: Steam's default.
    pub compat_tool: String,
    /// The tools to pick from (always includes `compat_tool`).
    pub tools: Vec<CompatTool>,
    /// What "Steam's default" runs the game with (display name), when known.
    pub default_tool: Option<String>,
    /// `name` of the tool whose build last ran the game (from the prefix).
    pub last_ran: Option<String>,
    /// Tools in `compatibilitytools.d` the running Steam has not read: it
    /// lists them only after it restarts.
    pub unseen: Vec<CompatTool>,
    pub access: Access,
    /// `None` until Steam runs the game through Proton once.
    pub prefix: Option<files::Prefix>,
    /// Steam's Flatpak: programs outside its sandbox (our script, a
    /// gamescope or MangoHud of the system) are not visible to it.
    pub flatpak: bool,
}

/// A change to one game; `None` leaves that setting alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    pub launch_options: Option<String>,
    /// `Some("")`: back to Steam's default.
    pub compat_tool: Option<String>,
}

/// The Steam install whose settings are read and changed.
#[derive(Debug, Clone)]
pub struct Steam {
    pub locator: SteamLocator,
    pub client: SteamClient,
}

/// Whether the library can be read from the client now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryAccess {
    Ready,
    /// Steam is not running.
    Closed,
    /// Steam runs and its client will answer: it is still starting,
    /// updating or waiting for the user to sign in.
    Starting,
    /// Steam runs without its debugging interface: it started before the
    /// flag file was there (it is now, for its next start).
    NeedsRestart,
    /// No Steam install was found.
    NotInstalled,
}

/// Whether [`Steam::restart`] can be offered: not in a gamescope session
/// (Steam Deck's game mode), where Steam is the session itself.
pub fn can_restart() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").as_deref() != Ok("gamescope")
}

/// Shown when Steam runs without its debugging interface.
pub fn read_only_message() -> &'static str {
    tr!("Steam is running without the access Heronite needs to change its settings. It is turned on for Steam's next start: restart Steam once, or close it, to change these settings.")
}

impl Steam {
    pub fn new(locator: SteamLocator, client: SteamClient) -> Self {
        Self { locator, client }
    }

    fn root(&self) -> &Path {
        &self.locator.steam_path
    }

    /// Steam's pid file sits next to `~/.steam/steam`, wherever the
    /// configured folder points. Windows has none: the registry and the
    /// process list tell instead.
    fn running(&self) -> bool {
        if cfg!(windows) {
            return self.locator.client_running(&heronite_launch::procs::SystemProcs);
        }
        self.pid_dirs().iter().any(|dir| is_steam_running(dir))
    }

    fn pid_dirs(&self) -> [PathBuf; 4] {
        let home = &self.locator.home;
        [
            self.root().to_path_buf(),
            home.join(".steam/steam"),
            home.join(".var/app/com.valvesoftware.Steam/.steam/steam"),
            home.join("snap/steam/common/.steam/steam"),
        ]
    }

    /// Turns the debugging interface on for the next start of every Steam
    /// install found, and of the one running: Steam reads the flag file
    /// only when it starts, so having it there before Steam opens is what
    /// spares the user a restart. Cheap (a few file checks).
    pub fn prepare_access(&self) {
        let running = webhelper().and_then(|w| w.steam_root);
        for root in self.locator.installs().into_iter().chain(running) {
            enable_client_api_on_next_start(&root);
        }
    }

    fn access(&self) -> Access {
        if !self.running() {
            return Access::Files;
        }
        if self.client.is_available() {
            return Access::Client;
        }
        self.prepare_access();
        Access::ReadOnly
    }

    fn user(&self) -> Result<String, String> {
        files::current_user(self.root(), &self.locator.home).ok_or_else(|| tr!("Could not tell which Steam account to use.").to_string())
    }

    /// Reads the game's settings. Blocking (the client may take seconds).
    pub fn read(&self, app_id: &str) -> Result<Snapshot, String> {
        if !self.root().join("config").is_dir() {
            return Err(tr!("Steam was not found in {path}.", path = self.root().display()));
        }
        let mut access = self.access();
        let mut from_client = None;
        if access == Access::Client {
            match client::read(&self.client, app_id) {
                Ok(app) => from_client = Some(app),
                Err(e) => {
                    log::warn!("Reading Steam app {app_id} through the client failed: {e:#}");
                    access = Access::ReadOnly;
                }
            }
        }
        let on_disk = files::compat_tools(&self.locator);
        let (launch_options, compat_tool, automatic, mut tools) = match from_client {
            Some(app) => (app.launch_options, app.compat_tool, app.automatic, app.tools.unwrap_or_else(|| on_disk.clone())),
            None => (
                files::read_launch_options(self.root(), &self.user()?, app_id)?,
                files::read_compat_tool(self.root(), app_id),
                None,
                on_disk.clone(),
            ),
        };
        for t in tools.iter_mut().filter(|t| t.path.is_none()) {
            t.path = on_disk.iter().find(|d| d.name == t.name).and_then(|d| d.path.clone());
        }
        if !compat_tool.is_empty() && !tools.iter().any(|t| t.name == compat_tool) {
            tools.push(CompatTool { name: compat_tool.clone(), display: compat_tool.clone(), path: None });
        }
        order_tools(&mut tools);
        let default_tool = automatic.or_else(|| {
            let name = files::automatic_tool(self.root(), app_id)?;
            Some(display_of(&tools, &name).unwrap_or(name))
        });
        let prefix = files::prefix(&self.locator, app_id);
        let last_ran = prefix.as_ref().and_then(|p| p.proton.as_deref()).and_then(|dir| {
            let dir = std::fs::canonicalize(dir).ok()?;
            let same = |p: &Path| std::fs::canonicalize(p).is_ok_and(|p| p == dir);
            tools.iter().find(|t| t.path.as_deref().is_some_and(same)).map(|t| t.name.clone())
        });
        let unseen = match access {
            Access::Files => Vec::new(),
            _ => files::unseen_tools(&self.locator, self.started_at()),
        };
        Ok(Snapshot { launch_options, compat_tool, tools, default_tool, last_ran, unseen, access, prefix, flatpak: self.flatpak() })
    }

    fn flatpak(&self) -> bool {
        self.root().starts_with(flatpak_steam_root(&self.locator.home))
    }

    /// When the running Steam started: the time of its pid file.
    fn started_at(&self) -> Option<SystemTime> {
        self.pid_dirs().iter().filter(|dir| is_steam_running(dir)).find_map(|dir| {
            let files = [dir.parent().map(|p| p.join("steam.pid")), Some(dir.join("steam.pid"))];
            files.into_iter().flatten().filter_map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).max()
        })
    }

    /// Whether Steam runs, and whether its client answers (Heronite can
    /// read and change settings through it). Blocking, a few milliseconds.
    pub fn status(&self) -> (bool, bool) {
        let running = self.running();
        (running, running && self.client.is_available())
    }

    /// The `steam` command with `args`, for this install (Flatpak or not).
    fn command(&self, steam_bin: &str, args: &[&str]) -> std::process::Command {
        use std::process::{Command, Stdio};
        let mut c = if self.flatpak() {
            let mut c = Command::new("flatpak");
            c.args(["run", "com.valvesoftware.Steam"]);
            c
        } else {
            Command::new(steam_bin)
        };
        // Not this app's environment: when Steam started Heronite it holds
        // Steam's overlay preload, its runtime and the AppImage's paths.
        c.args(args)
            .env_clear()
            .envs(desktop_protocol_env(std::env::vars()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        c
    }

    /// Starts Steam with the access Heronite needs already turned on.
    /// `steam_bin` is the `steam` command. Returns at once.
    pub fn open(&self, steam_bin: &str) -> Result<(), String> {
        self.prepare_access();
        let mut start = self.command(steam_bin, &[]);
        heronite_platform::process::new_group(&mut start);
        let mut child = start.spawn().map_err(|e| tr!("Could not run Steam: {error}", error = e.to_string()))?;
        // Reaped when it exits; Steam keeps running after Heronite closes.
        std::thread::spawn(move || child.wait());
        Ok(())
    }

    /// Closes Steam and starts it again (so it lists new tools and opens
    /// the access Heronite needs). `steam_bin` is the `steam` command.
    /// Blocking: waits up to a minute for Steam to close.
    pub fn restart(&self, steam_bin: &str) -> Result<(), String> {
        if !can_restart() {
            return Err(tr!("Steam cannot be restarted from here in game mode.").to_string());
        }
        if self.running() {
            let fail = |e: std::io::Error| tr!("Could not run Steam: {error}", error = e.to_string());
            self.command(steam_bin, &["-shutdown"]).status().map_err(fail)?;
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            while self.running() {
                if std::time::Instant::now() > deadline {
                    return Err(tr!("Steam did not close within a minute.").to_string());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        self.open(steam_bin)
    }

    /// Whether [`read_library`](Self::read_library) can work now. Blocking
    /// (asks the client, a few milliseconds).
    pub fn library_access(&self) -> LibraryAccess {
        let running = self.running();
        if !self.root().join("config").is_dir() && !running {
            return LibraryAccess::NotInstalled;
        }
        self.prepare_access();
        if !running {
            return LibraryAccess::Closed;
        }
        if self.client.is_available() {
            return LibraryAccess::Ready;
        }
        // Until its interface process starts, Steam is updating or starting;
        // once it has, its command line says whether the client will answer.
        match webhelper() {
            Some(w) if !w.debugging => LibraryAccess::NeedsRestart,
            _ => LibraryAccess::Starting,
        }
    }

    /// The account's games, from the running client.
    pub fn read_library(&self) -> Result<Vec<heronite_data::steam_owned::SteamOwnedGame>, String> {
        match self.library_access() {
            LibraryAccess::Ready => library::read_owned(&self.client).map_err(|e| format!("{e:#}")),
            LibraryAccess::Closed => Err(tr!("Steam is not running. Open Steam to read your library.").to_string()),
            LibraryAccess::Starting => Err(tr!("Steam is still starting. Your library is read as soon as it is ready.").to_string()),
            LibraryAccess::NeedsRestart => Err(tr!("Steam is running without the access Heronite needs to read your library. It is turned on for Steam's next start: restart Steam once.").to_string()),
            LibraryAccess::NotInstalled => Err(tr!("Steam was not found on this PC. Install Steam and sign in, then open it to read your library.").to_string()),
        }
    }

    /// Applies `change`; the access used. Blocking.
    pub fn apply(&self, app_id: &str, change: &Change) -> Result<Access, String> {
        let access = self.access();
        match access {
            Access::Client => {
                let fail = |e: anyhow::Error| tr!("Steam did not take the change: {error}", error = format!("{e:#}"));
                if let Some(line) = &change.launch_options {
                    client::set_launch_options(&self.client, app_id, line).map_err(fail)?;
                }
                if let Some(tool) = &change.compat_tool {
                    client::set_compat_tool(&self.client, app_id, tool).map_err(fail)?;
                }
            }
            Access::Files => {
                if let Some(line) = &change.launch_options {
                    files::write_launch_options(self.root(), &self.user()?, app_id, line)?;
                }
                if let Some(tool) = &change.compat_tool {
                    files::write_compat_tool(self.root(), app_id, tool)?;
                }
            }
            Access::ReadOnly => return Err(read_only_message().to_string()),
        }
        Ok(access)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_builds_first_then_valve_newest_first() {
        let t = |name: &str, display: &str| CompatTool { name: name.into(), display: display.into(), path: None };
        let mut tools = vec![
            t("steamlinuxruntime", "Steam Linux Runtime 1.0"),
            t("proton_63", "Proton 6.3-8"),
            t("proton_experimental", "Proton Experimental"),
            t("proton_9", "Proton 9.0-4"),
            t("legacysteamruntime", "Legacy Steam Runtime"),
            t("proton_11", "Proton 11.0-2"),
            t("proton_hotfix", "Proton Hotfix"),
            t("proton_7", "Proton 7.0-6"),
            t("dwproton", "dwproton-11.0-14"),
            t("GE-Proton11-6", "GE-Proton11-6"),
        ];
        order_tools(&mut tools);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "dwproton",
                "GE-Proton11-6",
                "proton_experimental",
                "proton_hotfix",
                "proton_11",
                "proton_9",
                "proton_7",
                "proton_63",
                "steamlinuxruntime",
                "legacysteamruntime"
            ]
        );
    }
}
