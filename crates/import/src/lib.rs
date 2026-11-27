//! Games of the other launchers brought into the Collection: Linux's
//! (Heroic, Lutris, Bottles, Faugus) and, on Windows, Heroic and Playnite.
//!
//! * [`lutris`] reads Lutris' library (`pga.db`), [`bottles`] the programs
//!   added to each bottle of Bottles (`bottle.yml`) and [`faugus`] Faugus
//!   Launcher's (`games.json`). Their installed games become Collection
//!   entries that ask that launcher to start them ([`collection`]), with its
//!   play time and covers. Steam games they list are left to the Steam
//!   import. When the user picks that they start here, the ones that can
//!   ([`Own`]) come over as this app's own instead, with their program,
//!   arguments, Wine prefix, Wine and environment variables
//!   ([`collection`]); the others keep asking the launcher.
//! * [`heroic`] copies the games added by hand in Heroic (they start here)
//!   and its records of installed Epic, GOG and Amazon games, so those show
//!   as installed once logged in here; each comes with its game settings
//!   (its Wine prefix) while it has none here.
//! * [`playnite`] reads Playnite's library (a LiteDB file, [`litedb`]) on
//!   Windows. Its installed games are brought over as this app's own
//!   ([`transfer`]): they start here, with their settings, emulator, details,
//!   status, favourite, categories and play time. A game only Playnite can
//!   start (one of its add-ons', or a script) keeps starting through it, like
//!   Lutris'.
//!
//! [`detect`] finds this system's launchers ([`Launcher::here`]; on Linux
//! native or Flatpak, Heroic in `%APPDATA%` on Windows) and reads their libraries: small files, read only. [`sync`] writes the
//! Collection; a game gone from its launcher stays, as not installed.
//! [`duplicates`] finds the games then in the Collection twice (added here by
//! hand, and brought in from Lutris, Bottles, Faugus or Playnite).

pub mod bottles;
pub mod collection;
pub mod duplicates;
pub mod faugus;
pub mod heroic;
pub mod litedb;
pub mod lutris;
pub mod playnite;
pub mod transfer;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use heronite_data::launchers::Launcher;
use heronite_data::DataPaths;

/// A launcher found on this machine and what its library holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub launcher: Launcher,
    /// Its data folder.
    pub location: PathBuf,
    /// Installed through Flatpak.
    pub flatpak: bool,
    /// Games another launcher starts (Lutris, Bottles, Faugus, Playnite).
    pub games: Vec<Game>,
    /// Heroic only: what [`heroic`] copies.
    pub heroic: Option<heroic::Library>,
    /// The launcher holds its library open (Playnite while it runs): its
    /// games cannot be read now, and none are listed.
    pub in_use: bool,
}

impl Found {
    /// The games it has (for Heroic, the ones added by hand and the
    /// installed store games).
    pub fn count(&self) -> usize {
        self.games.len() + self.heroic.as_ref().map_or(0, |h| h.count())
    }
}

/// An installed game of Lutris, Bottles, Faugus or Playnite.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Game {
    /// The launcher's id for it (Lutris' numeric id, the program's id in
    /// Bottles, Faugus' `gameid`, Playnite's game id).
    pub id: String,
    pub title: String,
    /// Its folder, when known.
    pub folder: Option<String>,
    pub windows: bool,
    pub minutes: u64,
    /// RFC 3339.
    pub last_played: Option<String>,
    /// Portrait cover and wide banner, as files.
    pub cover: Option<String>,
    pub banner: Option<String>,
    /// The command line that asks the launcher to start it.
    pub command: String,
    /// Its Wine prefix (Windows games).
    pub prefix: Option<String>,
    /// Where the program that runs it is when that is not its folder (the
    /// emulator of a Playnite game): the session follows it there.
    pub runs_from: Option<String>,
    /// A Playnite game that starts here without Playnite: how, and what it
    /// carries. `command` then starts it only until it is brought over.
    pub transfer: Option<Box<transfer::Transfer>>,
    /// A game of Lutris, Bottles or Faugus: how it starts here without its
    /// launcher, when it can. Used only when the user picks that its games
    /// start here.
    pub own: Option<Own>,
}

/// How a game of Lutris, Bottles or Faugus starts here, as its launcher
/// starts it: [`Game::windows`] runs `program` under Wine in
/// [`Game::prefix`] (always set then).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Own {
    /// A full path.
    pub program: String,
    /// Its arguments, as one line.
    pub args: String,
    /// The folder it starts in, when that is not the program's.
    pub working_dir: Option<String>,
    /// The Wine or Proton build the launcher runs it with, when it is on
    /// this PC; else this app's default one.
    pub wine: Option<Wine>,
    /// Its environment variables (the launcher's own for all games first).
    pub env: Vec<(String, String)>,
}

/// A Wine or Proton build (`wineVersion` of the game settings).
#[derive(Debug, Clone, PartialEq)]
pub struct Wine {
    /// `wine`, or Proton's `proton` script.
    pub bin: String,
    pub name: String,
    pub proton: bool,
}

/// What a [`sync`] changed in the Collection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    /// Entries whose details, install state or play time changed.
    pub updated: usize,
}

/// The launchers on this machine (`home` is the user's home folder), in
/// [`Launcher::here`] order. One that cannot be read is logged and left out.
pub fn detect(home: &Path, data: &DataPaths) -> Vec<Found> {
    let mut out = Vec::new();
    for &launcher in Launcher::here() {
        let found = match launcher {
            Launcher::Heroic => heroic::detect(home, data),
            Launcher::Lutris => lutris::detect(home),
            Launcher::Bottles => bottles::detect(home),
            Launcher::Faugus => faugus::detect(home),
            Launcher::Playnite => playnite::detect(),
        };
        match found {
            Ok(Some(f)) => out.push(f),
            Ok(None) => {}
            Err(e) => log::warn!("Reading {}'s library: {e:#}", launcher.name()),
        }
    }
    out
}

/// Brings `found`'s games into the Collection, but the ones the user took
/// out (`removed`: their app names). `home` is the user's home folder.
/// `start_here`: the games that can start here without their launcher come
/// over as this app's own ([`Launcher::can_start_here`]). Blocking (small
/// files). A library in use is an error: none of its games would be marked
/// as not installed.
pub fn sync(paths: &DataPaths, home: &Path, found: &Found, removed: &BTreeSet<String>, start_here: bool) -> Result<Report> {
    anyhow::ensure!(!found.in_use, "{} holds its library open", found.launcher.name());
    match &found.heroic {
        Some(library) => heroic::sync(paths, library, removed),
        None => collection::sync(paths, home, found.launcher, &found.games, removed, start_here),
    }
}

/// A YAML file as JSON values (Lutris and Bottles keep their settings so).
pub(crate) fn load_yaml(file: &Path) -> Result<serde_json::Value> {
    use anyhow::Context;
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let options = serde_saphyr::options! { duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins };
    serde_saphyr::from_str_with_options::<serde_json::Value>(&text, options).with_context(|| format!("{} is not YAML", file.display()))
}

/// A YAML scalar as text (`1`, `true` and words alike).
pub(crate) fn scalar(v: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A Proton build named `name` in Steam's folders of builds added by hand
/// (`compatibilitytools.d`, native or Flatpak Steam).
pub(crate) fn proton_named(home: &Path, name: &str) -> Option<Wine> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let data = xdg(home, "XDG_DATA_HOME", ".local/share");
    [
        data.join("Steam/compatibilitytools.d"),
        home.join(".steam/root/compatibilitytools.d"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d"),
    ]
    .into_iter()
    .map(|d| d.join(name).join("proton"))
    .find(|p| p.is_file())
    .map(|p| Wine { bin: p.to_string_lossy().into_owned(), name: name.to_string(), proton: true })
}

/// `KEY=value` words of a command line's start: its environment variables
/// (the rest, a wrapper or `%command%`, is left out).
pub(crate) fn env_words(line: &str) -> Vec<(String, String)> {
    line.split_whitespace()
        .filter_map(|w| {
            let (k, v) = w.split_once('=')?;
            let name =
                !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            name.then(|| (k.to_string(), v.trim_matches(['\'', '"']).to_string()))
        })
        .collect()
}

/// An XDG base folder of `home`: the variable when `home` is this user's
/// home, else `home/default`. In Heronite's Flatpak the variables point into
/// its sandbox; the host's are `HOST_<var>` there.
pub(crate) fn xdg(home: &Path, var: &str, default: &str) -> PathBuf {
    let own = std::env::var_os("HOME").is_some_and(|h| Path::new(&h) == home);
    let var = if heronite_platform::flatpak::active() { format!("HOST_{var}") } else { var.to_string() };
    std::env::var_os(var).filter(|_| own).map(PathBuf::from).unwrap_or_else(|| home.join(default))
}
