//! Plain data the Epic service exposes to pages, dialogs and the Collection.

use std::collections::HashMap;
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
}

#[derive(Clone, Debug)]
pub enum DownloadsEvent {
    /// The Epic library file or the install state changed.
    LibraryChanged,
    Toast(String, bool),
    /// Text for a message dialog (cloud saves output).
    Message { title: String, text: String },
    /// "Enable EOS Overlay" was asked for a game but the overlay is not
    /// installed: the Collection offers to install it now.
    EosNotInstalled { app_name: String },
}

/// The EOS Overlay as the Epic page shows it.
#[derive(Clone, Debug, Default)]
pub struct EosStatus {
    pub installed: bool,
    pub version: String,
    pub latest: String,
    /// "Check for updates" is running.
    pub checking: bool,
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
