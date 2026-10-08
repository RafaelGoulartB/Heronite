//! Heroic-compatible log files (`src/backend/logger/**` of Heroic).
//!
//! * Where: `$XDG_STATE_HOME/Heroic/logs` (`~/.local/state/Heroic/logs`), the
//!   folder Heroic and Heronite write to, so both apps' viewers see the same
//!   files. A game's log is `games/<appName>_<runner>/launch.log`; the
//!   previous run is kept as `launch.log.old`.
//! * How: [`GameLog`] mirrors Heroic's `LogWriter`. The first write of a run
//!   renames the existing file to `.old` (overwriting an older one) and
//!   starts a new file; lines look like `(21:04:41) [INFO]:    message`; a
//!   finished log ends with `============= End of log =============`.
//! * `disableLogs` (global `config.json`) and `verboseLogs` (per game) switch
//!   the launch log off: only "forced" lines (the "Logs are disabled"
//!   warning) are written then.
//!
//! The game writes its stdout / stderr straight into the log file (the launcher
//! hands it an `O_APPEND` descriptor, see [`GameLog::child_stdio`]), so there
//! is no pipe for the launcher to pump and the game does not depend on the
//! launcher staying alive. The launcher appends its own lines (header before
//! the spawn, "exited" and "End of log" after) to the same file.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use heronite_data::HeroicPaths;
use serde_json::{Map, Value};

/// Last line of a finished log.
pub const END_OF_LOG: &str = "============= End of log =============";

/// The launcher's own lines stop at this size. (The game's output goes
/// straight into the file and is not limited.)
pub const MAX_LOG_BYTES: u64 = 512 * 1024 * 1024;

/// `LogLevel` of Heroic, widest name `WARNING` (+ 3 columns).
const LEVEL_WIDTH: usize = 7 + 3;
/// `MaxLogPrefixLength` of Heroic (`DownloadManager`) + 3 columns.
const PREFIX_WIDTH: usize = 15 + 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        }
    }
}

/// `(hh:mm:ss) [LEVEL]:   [Prefix]:       message` without the newline.
/// An empty message gives an empty line (Heroic skips those entirely).
pub fn format_line(time: chrono::NaiveTime, level: Level, prefix: &str, message: &str) -> String {
    use chrono::Timelike;
    if message.is_empty() {
        return String::new();
    }
    let mut parts = vec![
        format!("({:02}:{:02}:{:02})", time.hour(), time.minute(), time.second()),
        format!("{:<LEVEL_WIDTH$}", format!("[{}]:", level.name())),
    ];
    if !prefix.is_empty() {
        parts.push(format!("{:<PREFIX_WIDTH$}", format!("[{prefix}]:")));
    }
    format!("{} {message}", parts.join(" "))
}

/// Which log of a game (Heroic's `GameLogType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameLogType {
    Launch,
    Install,
    Import,
    Repair,
    Update,
    Setup,
}

impl GameLogType {
    pub const ALL: [GameLogType; 6] = [
        GameLogType::Launch,
        GameLogType::Install,
        GameLogType::Import,
        GameLogType::Repair,
        GameLogType::Update,
        GameLogType::Setup,
    ];

    pub fn file_stem(self) -> &'static str {
        match self {
            GameLogType::Launch => "launch",
            GameLogType::Install => "install",
            GameLogType::Import => "import",
            GameLogType::Repair => "repair",
            GameLogType::Update => "update",
            GameLogType::Setup => "setup",
        }
    }

    pub fn from_stem(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.file_stem() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            GameLogType::Launch => "Launch",
            GameLogType::Install => "Install",
            GameLogType::Import => "Import",
            GameLogType::Repair => "Repair",
            GameLogType::Update => "Update",
            GameLogType::Setup => "Setup",
        }
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Where the log files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogPaths {
    /// Heroic's logs folder (`heroic.log`, `games/`, `runners/`).
    pub base: PathBuf,
    /// This app's own log (`heronite-rs-<date>.log`).
    pub launcher_dir: PathBuf,
}

/// Heroic's logs folder on this platform (`getBaseLogPath`).
pub fn heroic_default_base() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "windows") {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Local"));
        local.join("Heroic").join("logs")
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Logs").join("Heroic Games Launcher")
    } else {
        let state = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local").join("state"));
        state.join("Heroic").join("logs")
    }
}

/// `dirs::config_dir()` (kept here so the crate needs no extra dependency).
fn config_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
    }
}

impl LogPaths {
    /// The real locations when `heroic` is the real Heroic config folder;
    /// any other folder (tests, `HERONITE_HEROIC_DIR` copies) keeps its logs
    /// inside itself so nothing leaks into the user's real log folders.
    /// `HERONITE_LOG_DIR` overrides the base explicitly.
    pub fn for_heroic(heroic: &HeroicPaths) -> Self {
        if let Some(dir) = std::env::var_os("HERONITE_LOG_DIR").filter(|v| !v.is_empty()) {
            let base = PathBuf::from(dir);
            return Self { launcher_dir: base.join("launcher"), base };
        }
        let config = config_dir();
        let real_root = config.join("heroic");
        if real_root == heroic.root {
            let launcher = config.join("heronite-rs").join("logs");
            Self { base: heroic_default_base(), launcher_dir: launcher }
        } else {
            let base = heroic.root.join("logs");
            Self { launcher_dir: base.join("launcher"), base }
        }
    }

    /// `games/<appName>_<runner>/<type>.log`.
    pub fn game(&self, app_name: &str, runner: &str, ty: GameLogType) -> PathBuf {
        self.base.join("games").join(format!("{app_name}_{runner}")).join(format!("{}.log", ty.file_stem()))
    }

    /// `runners/<runner>.log`.
    pub fn runner(&self, runner: &str) -> PathBuf {
        self.base.join("runners").join(format!("{runner}.log"))
    }

    /// `heroic.log` (written by Heroic / Heronite).
    pub fn heroic_log(&self) -> PathBuf {
        self.base.join("heroic.log")
    }

    /// Today's launcher log.
    pub fn launcher_log(&self, date: chrono::NaiveDate) -> PathBuf {
        self.launcher_dir.join(format!("heronite-rs-{}.log", date.format("%Y-%m-%d")))
    }
}

/// `file` with `.old` appended (`launch.log` -> `launch.log.old`).
pub fn old_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

struct State {
    file: Option<File>,
    /// The first write archives the previous log; true once done.
    started: bool,
    written: u64,
    truncated: bool,
    closed: bool,
    failed: bool,
}

/// One log file of one run (Heroic's `LogWriter`), safe to share between the
/// launch thread and the output pump.
pub struct GameLog {
    path: PathBuf,
    /// Lines that are not forced are dropped (`disableLogs` / no `verboseLogs`).
    disabled: bool,
    state: Mutex<State>,
}

impl GameLog {
    pub fn new(path: PathBuf, disabled: bool) -> Arc<Self> {
        Arc::new(Self {
            path,
            disabled,
            state: Mutex::new(State { file: None, started: false, written: 0, truncated: false, closed: false, failed: false }),
        })
    }

    /// A log that writes nothing (tests, callers without a place for logs).
    pub fn null() -> Arc<Self> {
        Self::new(PathBuf::new(), true)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn debug(&self, message: &str) {
        self.line(Level::Debug, message, false)
    }
    pub fn info(&self, message: &str) {
        self.line(Level::Info, message, false)
    }
    pub fn warning(&self, message: &str) {
        self.line(Level::Warning, message, false)
    }
    pub fn error(&self, message: &str) {
        self.line(Level::Error, message, false)
    }
    /// Written even when logs are disabled (Heroic's `forceLog`).
    pub fn forced_warning(&self, message: &str) {
        self.line(Level::Warning, message, true)
    }

    fn line(&self, level: Level, message: &str, force: bool) {
        if message.trim().is_empty() {
            return;
        }
        let mut text = format_line(chrono::Local::now().time(), level, "", message);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.write(text.as_bytes(), force);
    }

    /// Where a game's stdout / stderr should go: a descriptor of the log file
    /// itself (shared `O_APPEND` description, so launcher and game lines
    /// interleave without overwriting each other). `Stdio::null()` when the
    /// log is disabled, closed or unwritable. Creates the file (archiving the
    /// previous one) if nothing was written yet.
    pub fn child_stdio(&self) -> std::process::Stdio {
        use std::process::Stdio;
        if self.path.as_os_str().is_empty() || self.disabled {
            return Stdio::null();
        }
        let mut st = self.state.lock().unwrap();
        if st.closed || st.failed {
            return Stdio::null();
        }
        if st.file.is_none() && !st.started {
            st.started = true;
            match self.open_new() {
                Ok(f) => st.file = Some(f),
                Err(e) => {
                    st.failed = true;
                    log::warn!("cannot write the log {}: {e}", self.path.display());
                    return Stdio::null();
                }
            }
        }
        match st.file.as_ref().map(File::try_clone) {
            Some(Ok(f)) => Stdio::from(f),
            _ => Stdio::null(),
        }
    }

    /// Raw output of the game (stdout / stderr chunks), stored as it comes.
    /// (Only for output that reaches the launcher; games write to the file
    /// directly, see [`GameLog::child_stdio`].)
    pub fn raw(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.write(bytes, false);
        }
    }

    /// Writes the final line and stops accepting writes.
    pub fn close(&self) {
        let has_file = {
            let st = self.state.lock().unwrap();
            if st.closed {
                return;
            }
            st.file.is_some()
        };
        // A log that exists (something was written) always gets its end.
        self.write(format!("{END_OF_LOG}\n").as_bytes(), has_file);
        let mut st = self.state.lock().unwrap();
        st.closed = true;
        st.file = None;
    }

    fn write(&self, data: &[u8], force: bool) {
        if self.path.as_os_str().is_empty() || (self.disabled && !force) {
            return;
        }
        let mut st = self.state.lock().unwrap();
        if st.closed || st.failed {
            return;
        }
        if st.file.is_none() && !st.started {
            st.started = true;
            match self.open_new() {
                Ok(f) => st.file = Some(f),
                Err(e) => {
                    st.failed = true;
                    log::warn!("cannot write the log {}: {e}", self.path.display());
                    return;
                }
            }
        }
        let st = &mut *st;
        let Some(file) = st.file.as_mut() else { return };
        if st.written + data.len() as u64 > MAX_LOG_BYTES {
            if !st.truncated {
                st.truncated = true;
                let note = format!("\n[log truncated: the size limit of {} MB was reached; the rest of the output is not stored]\n", MAX_LOG_BYTES / 1024 / 1024);
                let _ = file.write_all(note.as_bytes());
            }
            return;
        }
        if file.write_all(data).is_err() {
            st.failed = true;
            return;
        }
        st.written += data.len() as u64;
    }

    /// Renames the previous log to `.old` and creates the new file.
    fn open_new(&self) -> std::io::Result<File> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if self.path.exists() {
            std::fs::rename(&self.path, old_path(&self.path))?;
        }
        OpenOptions::new().create(true).append(true).open(&self.path)
    }
}

// ---------------------------------------------------------------------------
// The app's own log
// ---------------------------------------------------------------------------

/// A launcher log file stops growing at this size; the next record moves it
/// to `.old` and starts again.
pub const MAX_LAUNCHER_LOG_BYTES: u64 = 32 * 1024 * 1024;
/// Daily launcher logs kept (the newest ones).
pub const KEEP_LAUNCHER_LOGS: usize = 7;

struct LauncherState {
    date: chrono::NaiveDate,
    file: Option<File>,
    written: u64,
}

/// Writes the app's log records (`log` crate) to
/// `heronite-rs-<date>.log`, one file per day, newest [`KEEP_LAUNCHER_LOGS`]
/// kept. Several instances may append to the same day's file.
pub struct LauncherLogger {
    paths: LogPaths,
    disabled: bool,
    state: Mutex<LauncherState>,
}

impl LauncherLogger {
    /// Opens today's log and prunes old ones. `disabled` (the global
    /// `disableLogs`) keeps everything but the warning out of the file.
    pub fn open(paths: LogPaths, disabled: bool) -> Self {
        let this = Self {
            paths,
            disabled,
            state: Mutex::new(LauncherState { date: chrono::Local::now().date_naive(), file: None, written: 0 }),
        };
        {
            let mut st = this.state.lock().unwrap();
            this.rotate(&mut st);
        }
        if disabled {
            this.write_line(Level::Warning, "", "IMPORTANT: Logs are disabled. Enable logs before reporting any issue", true);
        }
        this
    }

    fn rotate(&self, st: &mut LauncherState) {
        let path = self.paths.launcher_log(st.date);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() >= MAX_LAUNCHER_LOG_BYTES {
                let _ = std::fs::rename(&path, old_path(&path));
            }
        }
        st.written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        st.file = OpenOptions::new().create(true).append(true).open(&path).ok();
        prune_launcher_logs(&self.paths.launcher_dir, KEEP_LAUNCHER_LOGS);
    }

    /// Appends one record (`target` is the `log` target; `heronite_` is
    /// dropped from it for the prefix column).
    pub fn record(&self, level: Level, target: &str, message: &str) {
        let prefix = target.strip_prefix("heronite_").unwrap_or(target);
        self.write_line(level, prefix, message, false);
    }

    /// A free-form line without a prefix (session markers).
    pub fn note(&self, message: &str) {
        self.write_line(Level::Info, "", message, true);
    }

    fn write_line(&self, level: Level, prefix: &str, message: &str, force: bool) {
        if self.disabled && !force {
            return;
        }
        let now = chrono::Local::now();
        let mut text = format_line(now.time(), level, prefix, message);
        if text.is_empty() {
            return;
        }
        text.push('\n');
        let mut st = self.state.lock().unwrap();
        if st.date != now.date_naive() || st.written >= MAX_LAUNCHER_LOG_BYTES {
            st.date = now.date_naive();
            self.rotate(&mut st);
        }
        let st = &mut *st;
        if let Some(f) = st.file.as_mut() {
            if f.write_all(text.as_bytes()).is_ok() {
                st.written += text.len() as u64;
            }
        }
    }
}

/// Deletes all but the newest `keep` `heronite-rs-*.log` files (and their
/// `.old`).
pub fn prune_launcher_logs(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with("heronite-rs-") && n.ends_with(".log"))
        .collect();
    names.sort();
    names.reverse();
    for name in names.into_iter().skip(keep) {
        let path = dir.join(&name);
        let _ = std::fs::remove_file(old_path(&path));
        let _ = std::fs::remove_file(path);
    }
}

// ---------------------------------------------------------------------------
// What goes into a launch log
// ---------------------------------------------------------------------------

/// Whether launches of `app_name` are logged, from the global
/// `disableLogs` and the per-game `verboseLogs` (default on). Returns the
/// merged settings too, for the header.
pub fn launch_log_switches(heroic: &HeroicPaths, app_name: &str) -> (bool, bool, Map<String, Value>) {
    let global = crate::settings::GlobalSettings::load(heroic);
    let merged = merged_settings(heroic, app_name, &global);
    let disabled_globally = truthy(global.defaults.get("disableLogs"));
    let verbose = merged.get("verboseLogs").map(|v| truthy(Some(v))).unwrap_or(true);
    (disabled_globally, verbose, merged)
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

/// Global defaults overlaid with the game's section (like Heroic's
/// `GameConfig.getSettings()`).
pub fn merged_settings(heroic: &HeroicPaths, app_name: &str, global: &crate::settings::GlobalSettings) -> Map<String, Value> {
    let mut merged = global.defaults.clone();
    let file = heronite_data::json_store::read_value(&heroic.game_config(app_name)).unwrap_or(Value::Null);
    if let Some(Value::Object(section)) = file.get(app_name) {
        for (k, v) in section {
            merged.insert(k.clone(), v.clone());
        }
    }
    merged
}

fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["apikey", "api_key", "token", "secret", "password", "passwd", "credential"].iter().any(|s| k.contains(s))
}

/// `filterGameSettingsForLog`: what is useful for support, nothing secret.
pub fn settings_for_log(merged: &Map<String, Value>, native: bool) -> Value {
    let mut s = merged.clone();
    for k in ["enableQuickSavesMenu", "verboseLogs", "enableMsync", "wineCrossoverBottle", "advertiseAvxForRosetta"] {
        s.remove(k);
    }
    if let Some(Value::Object(g)) = s.get_mut("gamescope") {
        let enabled = |g: &Map<String, Value>, k: &str| truthy(g.get(k));
        if !enabled(g, "enableLimiter") {
            g.remove("fpsLimiter");
            g.remove("fpsLimiterNoFocus");
        }
        if !enabled(g, "enableUpscaling") {
            for k in ["upscaleMethod", "upscaleHeight", "upscaleWidth", "gameHeight", "gameWidth", "windowType"] {
                g.remove(k);
            }
        }
    }
    if native {
        for k in [
            "wineVersion", "winePrefix", "autoInstallDxvk", "autoInstallDxvkNvapi", "autoInstallVkd3d", "enableDXVKFpsLimit", "DXVKFpsCap", "battlEyeRuntime",
            "eacRuntime", "enableEsync", "enableFsync", "enableWineWayland", "enableHDR", "enableWoW64",
        ] {
            s.remove(k);
        }
    }
    for (k, v) in s.iter_mut() {
        if is_secret_key(k) && v.as_str().is_some_and(|x| !x.is_empty()) {
            *v = Value::String("<hidden>".into());
        }
    }
    Value::Object(s)
}

/// Heroic prints `ENV=value ... program args`; values of secret-looking
/// variables are replaced.
pub fn command_line_for_log(cmd: &crate::plan::Command) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in &cmd.env {
        let shown = if is_secret_key(k) { "<hidden>".to_string() } else { v.clone() };
        let pair = format!("{k}={shown}");
        parts.push(shlex::try_quote(&pair).map(|c| c.into_owned()).unwrap_or(pair));
    }
    parts.push(cmd.display());
    parts.join(" ")
}

/// What the header of a launch log is made of.
pub struct HeaderInfo<'a> {
    pub title: &'a str,
    pub runner: &'a str,
    pub app_name: &'a str,
    pub native: bool,
    pub installed_in: Option<&'a str>,
    /// `formatSystemInfo` text, when available.
    pub system_info: Option<&'a str>,
    pub settings: &'a Map<String, Value>,
    pub settings_path: &'a Path,
    pub notes: &'a [String],
}

/// The lines Heroic's `prepareLaunch` writes before the game starts.
pub fn write_launch_header(log: &GameLog, h: &HeaderInfo<'_>) {
    log.info(&format!("Launching \"{}\" ({})", h.title, h.runner));
    log.info(&format!("Native? {}", h.native));
    if let Some(dir) = h.installed_in.filter(|d| !d.is_empty()) {
        log.info(&format!("Installed in: {dir}\n"));
    }
    if let Some(info) = h.system_info {
        log.info(&format!("System Info: \n{}\n", info.trim_end()));
    }
    let json = serde_json::to_string_pretty(&settings_for_log(h.settings, h.native)).unwrap_or_default();
    log.info(&format!("Game Settings: {json} \n Stored at: {}\n", h.settings_path.display()));
    for note in h.notes {
        log.info(note);
    }
}

/// The "not verbose" notice at the top of a launch log (forced).
pub fn write_disabled_notice(log: &GameLog) {
    log.forced_warning("IMPORTANT: Logs are disabled");
    log.forced_warning("Enable verbose logs in Game's settings > Advanced tab > 'Enable verbose logs' before reporting an issue.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;
    use serde_json::json;

    #[test]
    fn lines_are_formatted_like_heroic() {
        let t = NaiveTime::from_hms_opt(21, 4, 5).unwrap();
        assert_eq!(format_line(t, Level::Info, "", "Native? false"), "(21:04:05) [INFO]:    Native? false");
        assert_eq!(format_line(t, Level::Warning, "", "x"), "(21:04:05) [WARNING]: x");
        assert_eq!(format_line(t, Level::Error, "Backend", "boom"), "(21:04:05) [ERROR]:   [Backend]:         boom");
        assert_eq!(format_line(t, Level::Info, "", ""), "");
    }

    #[test]
    fn paths_follow_heroics_layout() {
        let p = LogPaths { base: PathBuf::from("/s/Heroic/logs"), launcher_dir: PathBuf::from("/c/logs") };
        assert_eq!(p.game("Calluna", "legendary", GameLogType::Launch), PathBuf::from("/s/Heroic/logs/games/Calluna_legendary/launch.log"));
        assert_eq!(p.game("a", "gog", GameLogType::Install), PathBuf::from("/s/Heroic/logs/games/a_gog/install.log"));
        assert_eq!(p.runner("legendary"), PathBuf::from("/s/Heroic/logs/runners/legendary.log"));
        assert_eq!(old_path(&p.heroic_log()), PathBuf::from("/s/Heroic/logs/heroic.log.old"));
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert_eq!(p.launcher_log(d), PathBuf::from("/c/logs/heronite-rs-2026-10-08.log"));
    }

    #[test]
    fn other_config_folders_keep_logs_inside_themselves() {
        let dir = tempfile::tempdir().unwrap();
        let p = LogPaths::for_heroic(&HeroicPaths::at(dir.path()));
        assert_eq!(p.base, dir.path().join("logs"));
        assert!(p.launcher_dir.starts_with(dir.path()));
    }

    #[test]
    fn a_new_run_archives_the_previous_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("games/g_sideload/launch.log");
        let first = GameLog::new(path.clone(), false);
        first.info("first run");
        first.raw(b"game says hi\n");
        first.close();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("first run") && text.contains("game says hi\n"));
        assert!(text.ends_with(&format!("{END_OF_LOG}\n")), "{text}");
        assert!(!old_path(&path).exists());

        let second = GameLog::new(path.clone(), false);
        second.info("second run");
        second.close();
        assert!(std::fs::read_to_string(old_path(&path)).unwrap().contains("first run"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("second run") && !text.contains("first run"));
        // Nothing is archived before the first write.
        let third = GameLog::new(path.clone(), false);
        assert!(std::fs::read_to_string(&path).unwrap().contains("second run"));
        drop(third);
    }

    #[test]
    fn disabled_logs_only_take_forced_lines_and_then_still_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("launch.log");
        let log = GameLog::new(path.clone(), true);
        log.info("hidden");
        log.raw(b"hidden output");
        assert!(!path.exists(), "no file and no rotation when nothing is written");
        write_disabled_notice(&log);
        log.info("still hidden");
        log.close();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[WARNING]: IMPORTANT: Logs are disabled"), "{text}");
        assert!(!text.contains("hidden output") && !text.contains("still hidden"));
        assert!(text.contains(END_OF_LOG));

        // Fully disabled and nothing forced: the file is never created.
        let path2 = dir.path().join("other.log");
        let quiet = GameLog::new(path2.clone(), true);
        quiet.info("x");
        quiet.close();
        assert!(!path2.exists());
    }

    #[test]
    fn writes_after_close_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("l.log");
        let log = GameLog::new(path.clone(), false);
        log.info("a");
        log.close();
        log.raw(b"late output");
        log.close();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("late output"));
        assert_eq!(text.matches(END_OF_LOG).count(), 1);
    }

    #[test]
    fn switches_follow_global_disable_and_game_verbose() {
        let dir = tempfile::tempdir().unwrap();
        let heroic = HeroicPaths::at(dir.path());
        std::fs::create_dir_all(heroic.games_config_dir()).unwrap();
        std::fs::write(heroic.root.join("config.json"), r#"{"defaultSettings":{"disableLogs":false,"verboseLogs":true,"steamGridDbApiKey":"K"},"version":"v0"}"#).unwrap();
        std::fs::write(heroic.game_config("quiet"), r#"{"quiet":{"verboseLogs":false},"version":"v0"}"#).unwrap();
        let (off, verbose, merged) = launch_log_switches(&heroic, "loud");
        assert!(!off && verbose);
        let shown = settings_for_log(&merged, true).to_string();
        assert!(!shown.contains("\"K\"") && shown.contains("<hidden>"), "{shown}");
        let (_, verbose, _) = launch_log_switches(&heroic, "quiet");
        assert!(!verbose);
        std::fs::write(heroic.root.join("config.json"), r#"{"defaultSettings":{"disableLogs":true},"version":"v0"}"#).unwrap();
        let (off, verbose, _) = launch_log_switches(&heroic, "anything");
        assert!(off, "disableLogs wins");
        assert!(verbose, "verboseLogs defaults to on like Heroic's defaults");
    }

    #[test]
    fn settings_for_log_drops_noise() {
        let m = json!({"verboseLogs":true,"enableQuickSavesMenu":true,"winePrefix":"/p","useGameMode":false,
            "gamescope":{"enableLimiter":false,"fpsLimiter":"30","enableUpscaling":true,"upscaleMethod":"fsr"}});
        let v = settings_for_log(m.as_object().unwrap(), false);
        assert!(v.get("verboseLogs").is_none() && v.get("enableQuickSavesMenu").is_none());
        assert_eq!(v["winePrefix"], "/p");
        assert!(v["gamescope"].get("fpsLimiter").is_none());
        assert_eq!(v["gamescope"]["upscaleMethod"], "fsr");
        assert!(settings_for_log(m.as_object().unwrap(), true).get("winePrefix").is_none());
    }

    #[test]
    fn command_lines_hide_secrets() {
        let mut env = crate::plan::Command::default();
        env.program = "legendary".into();
        env.args = vec!["launch".into(), "x y".into()];
        env.env.insert("WINEPREFIX".into(), "/p".into());
        env.env.insert("MY_TOKEN".into(), "abc".into());
        let line = command_line_for_log(&env);
        assert!(line.contains("WINEPREFIX=/p") && line.contains("MY_TOKEN=\\<hidden\\>") || line.contains("MY_TOKEN='<hidden>'") || line.contains("MY_TOKEN=<hidden>"), "{line}");
        assert!(!line.contains("abc"));
        assert!(line.ends_with("legendary launch 'x y'"), "{line}");
    }
    #[test]
    fn launcher_log_appends_prunes_and_respects_disable() {
        let dir = tempfile::tempdir().unwrap();
        let paths = LogPaths { base: dir.path().join("base"), launcher_dir: dir.path().join("l") };
        std::fs::create_dir_all(&paths.launcher_dir).unwrap();
        for d in 1..=9 {
            std::fs::write(paths.launcher_dir.join(format!("heronite-rs-2020-01-0{d}.log")), "x").unwrap();
        }
        std::fs::write(paths.launcher_dir.join("keep-me.txt"), "x").unwrap();
        let logger = LauncherLogger::open(paths.clone(), false);
        logger.record(Level::Info, "heronite_launch::runtime", "hello");
        logger.record(Level::Error, "wgpu", "bad");
        let today = paths.launcher_log(chrono::Local::now().date_naive());
        let text = std::fs::read_to_string(&today).unwrap();
        assert!(text.contains("[INFO]:    [launch::runtime]: hello\n"), "{text}");
        assert!(text.contains("[ERROR]:   [wgpu]:") && text.contains("bad"));
        let kept = crate::log_reader::list_logs(&paths, &HeroicPaths::at(dir.path().join("h"))).len();
        assert_eq!(kept, KEEP_LAUNCHER_LOGS);
        assert!(paths.launcher_dir.join("keep-me.txt").exists());
        assert!(!paths.launcher_dir.join("heronite-rs-2020-01-01.log").exists());

        let paths2 = LogPaths { base: dir.path().join("b2"), launcher_dir: dir.path().join("l2") };
        let off = LauncherLogger::open(paths2.clone(), true);
        off.record(Level::Info, "heronite_x", "secret chatter");
        let text = std::fs::read_to_string(paths2.launcher_log(chrono::Local::now().date_naive())).unwrap();
        assert!(text.contains("Logs are disabled") && !text.contains("chatter"));
    }
}
