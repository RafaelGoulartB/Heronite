//! What `mods.json` holds for one game: the mods in load order, their
//! switches, where each came from and the game's links to Nexus Mods and
//! Thunderstore. Shared by every module of the crate and by the app.

use serde::{Deserialize, Serialize};

/// Version of the `mods.json` format.
pub const FORMAT: u32 = 1;

/// The mods of one game (`<mods root>/<game key>/mods.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModList {
    pub format: u32,
    /// `runner_appName` of the Collection game.
    pub game_key: String,
    /// The game's title when the list was last saved (logs, link pickers).
    pub title: String,
    /// "Play with mods": false = the game starts vanilla, mods stay installed.
    pub active: bool,
    /// Load order: later entries win a file over earlier ones.
    pub mods: Vec<ModEntry>,
    pub links: GameLinks,
    /// Plugins (`.esp` / `.esm` / `.esl`, lower case) the user turned off;
    /// every other plugin of an enabled mod is active.
    pub disabled_plugins: Vec<String>,
}

impl ModList {
    pub fn new(game_key: &str, title: &str) -> Self {
        Self { format: FORMAT, game_key: game_key.to_string(), title: title.to_string(), active: true, ..Default::default() }
    }

    pub fn get(&self, id: &str) -> Option<&ModEntry> {
        self.mods.iter().find(|m| m.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ModEntry> {
        self.mods.iter_mut().find(|m| m.id == id)
    }

    /// The enabled mods in load order (lowest priority first).
    pub fn enabled(&self) -> impl Iterator<Item = &ModEntry> {
        self.mods.iter().filter(|m| m.enabled)
    }

    /// Whether a launch mounts anything.
    pub fn applies(&self) -> bool {
        self.active && self.mods.iter().any(|m| m.enabled)
    }
}

/// Where the game is on the mod sites. `None` = not linked yet (found
/// automatically on first use, or picked by the user).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameLinks {
    /// Nexus Mods game domain (`skyrimspecialedition`).
    pub nexus_domain: Option<String>,
    /// Thunderstore community identifier (`lethal-company`).
    pub thunderstore: Option<String>,
    /// Folder (relative to the game folder) loose mod files go to, overriding
    /// the known-games table (`Data`, `BepInEx/plugins`, `Mods`). `Some("")` =
    /// the game folder itself.
    pub mod_root: Option<String>,
}

/// One installed mod.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModEntry {
    /// Folder name under `mods/`: `nexus-<modId>-<fileId>`,
    /// `ts-<namespace>-<name>`, `file-<slug>`; unique in the list.
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    /// Picture or icon URL (Nexus picture, Thunderstore icon).
    pub picture: Option<String>,
    pub enabled: bool,
    pub source: ModSource,
    /// RFC 3339.
    pub installed_at: String,
    /// Files and bytes in the mod folder.
    pub files: u64,
    pub size: u64,
    /// Archive kept in `downloads/` (file name), for reinstalling.
    pub archive: Option<String>,
    /// Choices made in the FOMOD wizard, to preselect them on reinstall.
    pub fomod: Option<FomodChoices>,
    /// Newer version seen online (Nexus files, Thunderstore latest).
    pub update: Option<UpdateInfo>,
    /// Thunderstore dependencies (`Namespace-Name`), installed with it.
    pub depends_on: Vec<String>,
    /// Plugins (`.esp` / `.esm` / `.esl`) at the root of its `Data`, as named.
    pub plugins: Vec<String>,
    /// Proxy DLLs at the game folder's root that need `WINEDLLOVERRIDES`
    /// (`winhttp`, `version`, `dinput8`, …), lower case without `.dll`.
    pub dll_overrides: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum ModSource {
    /// Added from an archive or a folder on disk.
    #[default]
    File,
    Nexus {
        domain: String,
        mod_id: u64,
        file_id: u64,
    },
    Thunderstore {
        community: String,
        namespace: String,
        name: String,
    },
}

impl ModSource {
    /// The mod's page on its site.
    pub fn page_url(&self) -> Option<String> {
        match self {
            ModSource::File => None,
            ModSource::Nexus { domain, mod_id, .. } => Some(format!("https://www.nexusmods.com/{domain}/mods/{mod_id}")),
            ModSource::Thunderstore { community, namespace, name } => {
                Some(format!("https://thunderstore.io/c/{community}/p/{namespace}/{name}/"))
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateInfo {
    pub version: String,
    /// Nexus file to download for the update (None: Thunderstore, or Nexus
    /// with no single newer main file).
    pub file_id: Option<u64>,
    /// RFC 3339 of the check.
    pub checked_at: String,
}

/// The options picked in a FOMOD installer, by step / group / plugin name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FomodChoices {
    pub steps: Vec<FomodStepChoice>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FomodStepChoice {
    pub step: String,
    pub groups: Vec<FomodGroupChoice>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FomodGroupChoice {
    pub group: String,
    pub plugins: Vec<String>,
}

/// One copy of an installer's result: `source` (relative to the extracted
/// archive) to `destination` (relative to the mod root of the game, see
/// [`GameLinks::mod_root`]). Later rules (higher `priority`, then later in
/// the list) win a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRule {
    pub source: String,
    pub destination: String,
    pub is_folder: bool,
    pub priority: i64,
}

/// Progress of a long job (download, extract, copy), for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub done: u64,
    /// 0 = unknown.
    pub total: u64,
}

impl Progress {
    /// 0.0..=1.0, `None` while the total is unknown.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32)
    }
}
