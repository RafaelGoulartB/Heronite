//! What plan building reads (`PlanContext`) and per-launch inputs (`LaunchOptions`).

use heronite_data::DataPaths;
use serde_json::Value;

use crate::emulators::EmulatorsFile;
use crate::settings::GlobalSettings;

/// Everything read from disk that plan building needs.
#[derive(Debug, Clone)]
pub struct PlanContext {
    pub paths: DataPaths,
    pub global: GlobalSettings,
    pub emulators: EmulatorsFile,
    /// `language` from `store/config.json` (legendary's `--language`).
    pub ui_language: Option<String>,
    /// `<data>/known_fixes.json`, if downloaded.
    pub known_fixes: Option<Value>,
    /// `store_cache/umu.json` (umu ids cached by Heroic), if present.
    pub umu_ids: Option<Value>,
    /// The umu database (see [`crate::umu_ids`]); empty before it is downloaded.
    pub umu_database: Vec<crate::umu_ids::Entry>,
}

impl PlanContext {
    pub fn load(paths: &DataPaths) -> Self {
        use heronite_data::json_store::read_value;
        let opt = |p: std::path::PathBuf| read_value(&p).ok().filter(|v| !v.is_null());
        let config = opt(paths.store_config());
        Self {
            paths: paths.clone(),
            global: GlobalSettings::load(paths),
            emulators: heronite_data::json_store::read(&paths.emulators()).unwrap_or_default(),
            ui_language: config
                .as_ref()
                .and_then(|c| c.get("language"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            known_fixes: opt(paths.known_fixes()),
            umu_ids: opt(paths.store_cache("umu")),
            umu_database: crate::umu_ids::load(paths),
        }
    }

    pub fn tools_dir(&self) -> String {
        self.paths.tools_dir().to_string_lossy().into_owned()
    }

    pub fn runtime_dir(&self) -> String {
        crate::env::join(&self.tools_dir(), "runtimes")
    }
}

/// Per-launch inputs from the caller.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Extra arguments after the game's configured `launcherArgs`. For
    /// emulators the first one may name the ROM to run.
    pub extra_args: Vec<String>,
    /// legendary `--skip-version-check`.
    pub skip_version_check: bool,
    /// legendary `--override-exe` for an alternative executable.
    pub alt_exe: Option<String>,
    /// Emulators: arguments that boot into a save state, put before the rest
    /// of argv.
    pub state_args: Vec<String>,
    /// Emulators: the state boots the game by itself, so the ROM argument is
    /// dropped from argv.
    pub state_replaces_image: bool,
}
