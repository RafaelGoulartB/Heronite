//! Plain data the store service exposes to pages, dialogs and the Collection.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::*;
use heronite_epic::cmd::Cancel;
use serde_json::Value;

/// A one-off operation running for a game.
#[derive(Clone)]
pub struct Busy {
    pub label: SharedString,
    /// 0..=100 when known.
    pub percent: Option<f32>,
    pub cancel: Option<Arc<Cancel>>,
}

#[derive(Clone, Debug, Default)]
pub struct Account {
    pub logged_in: bool,
    pub name: String,
    /// The saved sign-in stopped working: shown as logged out, with "Log in
    /// again", until the next login (or logout).
    pub expired: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub owned: usize,
    pub installed: usize,
    /// Facts the Collection's slimmed-down game model leaves out.
    pub(super) flags: Arc<HashMap<String, GameFlags>>,
}

/// What the library file says about a game that [`CollectionGame`] drops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameFlags {
    /// `dlcList` is not empty.
    pub has_dlcs: bool,
    /// EA App / Ubisoft Connect games (`thirdPartyManagedApp`).
    pub third_party: bool,
}

/// The stores besides Epic: their games install through gogdl / nile but
/// share the download queue, the Collection glue and the pages with Epic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Store {
    Gog,
    Amazon,
}

impl Store {
    pub const ALL: [Store; 2] = [Store::Gog, Store::Amazon];

    /// The part of the app it is (`heronite_platform::features`).
    pub fn feature(self) -> heronite_platform::Feature {
        match self {
            Store::Gog => heronite_platform::Feature::GogStore,
            Store::Amazon => heronite_platform::Feature::AmazonStore,
        }
    }

    /// Whether this system has the store.
    pub fn available(self) -> bool {
        heronite_platform::supported(self.feature())
    }

    /// `GameInfo.runner` of its games.
    pub fn runner(self) -> &'static str {
        match self {
            Store::Gog => "gog",
            Store::Amazon => "nile",
        }
    }

    pub fn from_runner(runner: &str) -> Option<Store> {
        match runner {
            "gog" => Some(Store::Gog),
            "nile" => Some(Store::Amazon),
            _ => None,
        }
    }

    /// The store's name (a brand: not translated).
    pub fn name(self) -> &'static str {
        match self {
            Store::Gog => "GOG",
            Store::Amazon => "Amazon Games",
        }
    }
}

/// Account, library counts and updates of a [`Store`].
#[derive(Clone, Debug, Default)]
pub struct StoreState {
    pub account: Account,
    pub owned: usize,
    pub installed: usize,
    pub updates: HashSet<String>,
    pub refreshing: bool,
    pub logging_in: bool,
    /// Last problem of login / refresh, for the store's page.
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueState {
    Downloading,
    Paused,
    Waiting,
}

/// What the install dialog (or an update) asks the queue for.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub game_info: Value,
    pub path: String,
    pub platform: String,
    pub sdl_tags: Vec<String>,
    pub dlcs: Vec<String>,
    pub language: String,
    /// Download size as text.
    pub size: Option<String>,
    /// Wine settings to store for the game (Linux): prefix and wine version.
    pub wine_prefix: Option<String>,
    pub wine_version: Option<Value>,
    /// "Force install" with too little free space: legendary skips its own
    /// check (gogdl and nile cannot).
    pub ignore_free_space: bool,
}

#[derive(Clone, Debug)]
pub enum DownloadsEvent {
    /// The Epic library file or the install state changed.
    LibraryChanged,
    Toast(String, bool),
    /// Text for a message dialog (cloud saves output).
    Message {
        title: String,
        text: String,
    },
    /// "Enable EOS Overlay" was asked for a game but the overlay is not
    /// installed: the Collection offers to install it now.
    EosNotInstalled {
        app_name: String,
    },
}

/// The EOS Overlay as the Epic page shows it.
#[derive(Clone, Debug, Default)]
pub struct EosStatus {
    pub installed: bool,
    pub version: String,
    pub latest: String,
    /// "Check for updates" is running.
    pub checking: bool,
    /// Windows: whether the overlay is on, for every game at once (`None`
    /// until read, and on Linux, where it is per prefix).
    pub enabled: Option<bool>,
}

/// Whether the overlay is on for the prefix of one game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EosGame {
    Loading,
    Enabled,
    Disabled,
    /// Not applicable here (the overlay is per prefix on Linux only).
    Unavailable,
}
