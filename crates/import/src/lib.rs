//! Games of the other Linux launchers brought into the Collection.
//!
//! * [`lutris`] reads Lutris' library (`pga.db`), [`bottles`] the programs
//!   added to each bottle of Bottles (`bottle.yml`) and [`faugus`] Faugus
//!   Launcher's (`games.json`). Their installed games become Collection
//!   entries that ask that launcher to start them ([`collection`]), with its
//!   play time and covers. Steam games they list are left to the Steam
//!   import.
//! * [`heroic`] copies the games added by hand in Heroic (with their game
//!   settings: they start here) and its records of installed Epic, GOG and
//!   Amazon games, so those show as installed once logged in here.
//!
//! [`detect`] finds the launchers on this machine (native or Flatpak) and
//! reads their libraries: small files, read only. [`sync`] writes the
//! Collection; a game gone from its launcher stays, as not installed.
//! [`duplicates`] finds the games then in the Collection twice (added here by
//! hand, and brought in from Lutris, Bottles or Faugus).

pub mod bottles;
pub mod collection;
pub mod duplicates;
pub mod faugus;
pub mod heroic;
pub mod lutris;

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
    /// Games another launcher starts (Lutris, Bottles, Faugus).
    pub games: Vec<Game>,
    /// Heroic only: what [`heroic`] copies.
    pub heroic: Option<heroic::Library>,
}

impl Found {
    /// The games it has (for Heroic, the ones added by hand and the
    /// installed store games).
    pub fn count(&self) -> usize {
        self.games.len() + self.heroic.as_ref().map_or(0, |h| h.count())
    }
}

/// An installed game of Lutris, Bottles or Faugus.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Game {
    /// The launcher's id for it (Lutris' numeric id, the program's id in
    /// Bottles, Faugus' `gameid`).
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
}

/// What a [`sync`] changed in the Collection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub added: usize,
    /// Entries whose details, install state or play time changed.
    pub updated: usize,
}

/// The launchers on this machine (`home` is the user's home folder), in
/// [`Launcher::ALL`] order. One that cannot be read is logged and left out.
pub fn detect(home: &Path, data: &DataPaths) -> Vec<Found> {
    let mut out = Vec::new();
    for launcher in Launcher::ALL {
        let found = match launcher {
            Launcher::Heroic => heroic::detect(home, data),
            Launcher::Lutris => lutris::detect(home),
            Launcher::Bottles => bottles::detect(home),
            Launcher::Faugus => faugus::detect(home),
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
/// out (`removed`: their app names). Blocking (small files).
pub fn sync(paths: &DataPaths, found: &Found, removed: &BTreeSet<String>) -> Result<Report> {
    match &found.heroic {
        Some(library) => heroic::sync(paths, library, removed),
        None => collection::sync(paths, found.launcher, &found.games, removed),
    }
}

/// An XDG base folder of `home`: the variable when `home` is this user's
/// home, else `home/default`. In Heronite's Flatpak the variables point into
/// its sandbox; the host's are `HOST_<var>` there.
pub(crate) fn xdg(home: &Path, var: &str, default: &str) -> PathBuf {
    let own = std::env::var_os("HOME").is_some_and(|h| Path::new(&h) == home);
    let var = if heronite_platform::flatpak::active() { format!("HOST_{var}") } else { var.to_string() };
    std::env::var_os(var).filter(|_| own).map(PathBuf::from).unwrap_or_else(|| home.join(default))
}
