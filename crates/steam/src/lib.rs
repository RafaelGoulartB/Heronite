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

use std::path::Path;

use heronite_i18n::tr;
use heronite_launch::steam::{flatpak_steam_root, SteamLocator};
use heronite_shortcuts::steam::{enable_client_api_on_next_start, is_steam_running};
use heronite_shortcuts::steam_client::SteamClient;

/// A compatibility tool as Steam names it: `name` is what it stores
/// (`proton_9`, `GE-Proton9-20`), `display` what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatTool {
    pub name: String,
    pub display: String,
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
    /// Steam runs without its debugging interface (turned on for its next start).
    NeedsRestart,
    /// No Steam install was found.
    NotInstalled,
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
    /// configured folder points.
    fn running(&self) -> bool {
        let home = &self.locator.home;
        [self.root().to_path_buf(), home.join(".steam/steam"), home.join(".var/app/com.valvesoftware.Steam/.steam/steam")]
            .iter()
            .any(|dir| is_steam_running(dir))
    }

    fn access(&self) -> Access {
        if !self.running() {
            return Access::Files;
        }
        if self.client.is_available() {
            return Access::Client;
        }
        enable_client_api_on_next_start(self.root());
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
        let (launch_options, compat_tool, mut tools) = match from_client {
            Some(app) => (app.launch_options, app.compat_tool, app.tools.unwrap_or_else(|| files::compat_tools(&self.locator))),
            None => (
                files::read_launch_options(self.root(), &self.user()?, app_id)?,
                files::read_compat_tool(self.root(), app_id),
                files::compat_tools(&self.locator),
            ),
        };
        if !compat_tool.is_empty() && !tools.iter().any(|t| t.name == compat_tool) {
            tools.push(CompatTool { name: compat_tool.clone(), display: compat_tool.clone() });
        }
        order_tools(&mut tools);
        Ok(Snapshot {
            launch_options,
            compat_tool,
            tools,
            access,
            prefix: files::prefix(&self.locator, app_id),
            flatpak: self.root().starts_with(flatpak_steam_root(&self.locator.home)),
        })
    }

    /// Whether [`read_library`](Self::read_library) can work now. Blocking
    /// (asks the client, a few milliseconds).
    pub fn library_access(&self) -> LibraryAccess {
        if !self.root().join("config").is_dir() && !self.running() {
            return LibraryAccess::NotInstalled;
        }
        match self.access() {
            Access::Client => LibraryAccess::Ready,
            Access::Files => LibraryAccess::Closed,
            Access::ReadOnly => LibraryAccess::NeedsRestart,
        }
    }

    /// The account's games, from the running client.
    pub fn read_library(&self) -> Result<Vec<heronite_data::steam_owned::SteamOwnedGame>, String> {
        match self.library_access() {
            LibraryAccess::Ready => library::read_owned(&self.client).map_err(|e| format!("{e:#}")),
            LibraryAccess::Closed => Err(tr!("Steam is not running. Open Steam to read your library.").to_string()),
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
        let t = |name: &str, display: &str| CompatTool { name: name.into(), display: display.into() };
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
