//! Ludusavi integration (port of `local-library/ludusavi.ts`): binary and
//! config detection, `find --api` / `backup --api` command construction and
//! JSON parsing, and the after-play backup flow.
//!
//! Processes only run through [`CommandRunner`]; tests inject a fake so no
//! real `ludusavi backup` is ever executed. Results are written back to
//! `settings.json` (`ludusavi.lastBackup*`/`lastError`) like Heronite.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{SecondsFormat, Utc};
use heronite_data::models::LocalMetaFile;
use heronite_data::{json_store, settings, HeroicPaths};
use heronite_launch::probe::Probe;
use heronite_launch::settings::{load_game_settings, GlobalSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pathutil::join;
use crate::settings_ext::{record_ludusavi_backup, LudusaviRecord};

pub const SKIP_SESSION_SECONDS: u64 = 5;
pub const LUDUSAVI_TIMEOUT: Duration = Duration::from_secs(180);

// ---------------------------------------------------------------------------
// Process runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Runs `binary args...`. A non-zero exit is `Ok` with its code (Ludusavi
/// exits 1 when some games have no saves yet still prints a useful JSON);
/// `Err` means the process could not run at all (spawn failure, timeout).
pub trait CommandRunner: Send + Sync {
    fn run(&self, binary: &str, args: &[String]) -> Result<CommandOutput, String>;
}

/// Real process execution with a timeout and `/usr/bin:/usr/local/bin`
/// appended to `PATH`.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    pub timeout: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self { timeout: LUDUSAVI_TIMEOUT }
    }
}

impl CommandRunner for SystemRunner {
    fn run(&self, binary: &str, args: &[String]) -> Result<CommandOutput, String> {
        let path = format!(
            "{}:/usr/bin:/usr/local/bin",
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = Command::new(binary)
            .args(args)
            .env("PATH", path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut out = child.stdout.take().expect("piped");
        let mut err = child.stderr.take().expect("piped");
        let t_out = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = out.read_to_end(&mut b);
            b
        });
        let t_err = std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = err.read_to_end(&mut b);
            b
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{binary} timed out after {}s", self.timeout.as_secs()));
                }
                Err(e) => return Err(e.to_string()),
            }
        };
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&t_out.join().unwrap_or_default()).into_owned(),
            stderr: String::from_utf8_lossy(&t_err.join().unwrap_or_default()).into_owned(),
            code: status.code().unwrap_or(1),
        })
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// `LudusaviDetectedConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviDetected {
    pub binary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    /// 'zip' | 'simple'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// 'none' | 'deflate' | 'bzip2' | 'zstd'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// `yamlGet`: a scalar at a nested key path of the (indentation-only) config.
pub fn yaml_get(text: &str, keys: &[&str]) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut from = 0usize;
    let mut indent = 0usize;
    for (depth, key) in keys.iter().enumerate() {
        let needle = format!("{key}:");
        let mut next: Option<(usize, usize)> = None;
        let mut i = from;
        while i < lines.len() {
            let raw = lines[i];
            let trimmed = raw.trim_start();
            if raw.trim().is_empty() || trimmed.starts_with('#') {
                i += 1;
                continue;
            }
            let current = raw.chars().take_while(|c| *c == ' ').count();
            if i != from && current < indent {
                return None;
            }
            if current != indent || !trimmed.starts_with(&needle) {
                i += 1;
                continue;
            }
            let rest = trimmed[needle.len()..].trim();
            if depth == keys.len() - 1 {
                if rest.is_empty() || rest == "|" || rest == ">" {
                    return None;
                }
                let rest = rest.strip_prefix(['\'', '"']).unwrap_or(rest);
                let rest = rest.strip_suffix(['\'', '"']).unwrap_or(rest);
                return Some(rest.to_string());
            }
            next = Some((i + 1, current + 2));
            break;
        }
        let (f, ind) = next?;
        from = f;
        indent = ind;
    }
    None
}

fn expand_home(value: Option<String>, home: &str) -> Option<String> {
    let v = value.filter(|v| !v.is_empty())?;
    if v == "~" {
        Some(home.to_string())
    } else if let Some(rest) = v.strip_prefix("~/") {
        Some(join(home, rest))
    } else {
        Some(v)
    }
}

/// `resolveBinary`: the preferred path, `$PATH`, then the usual places.
pub fn resolve_binary(probe: &dyn Probe, preferred: Option<&str>) -> Option<String> {
    let home = probe.home();
    [
        preferred.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string),
        probe.which("ludusavi"),
        Some("/usr/bin/ludusavi".into()),
        Some("/usr/local/bin/ludusavi".into()),
        Some(join(&home, ".local/bin/ludusavi")),
    ]
    .into_iter()
    .flatten()
    .find(|p| probe.exists(p))
}

/// Parses the `backup:` part of Ludusavi's `config.yaml`.
pub fn parse_config(yaml: &str, home: &str) -> (Option<String>, Option<String>, Option<String>) {
    let backup_path = expand_home(yaml_get(yaml, &["backup", "path"]), home);
    let format = yaml_get(yaml, &["backup", "format", "chosen"]).filter(|f| f == "zip" || f == "simple");
    let compression = yaml_get(yaml, &["backup", "format", "zip", "compression"])
        .filter(|c| matches!(c.as_str(), "none" | "deflate" | "bzip2" | "zstd"));
    (backup_path, format, compression)
}

/// `detectLudusavi`.
pub fn detect_ludusavi(
    runner: &dyn CommandRunner,
    probe: &dyn Probe,
    preferred: Option<&str>,
) -> Option<LudusaviDetected> {
    let binary = resolve_binary(probe, preferred)?;
    let home = probe.home();

    let version = runner.run(&binary, &["--version".into()]).ok().and_then(|o| {
        let text = if o.stdout.is_empty() { o.stderr } else { o.stdout };
        let re = regex::Regex::new(r"(?i)ludusavi\s+([\w.-]+)").unwrap();
        re.captures(&text).map(|c| c[1].to_string())
    });

    let config_path = runner
        .run(&binary, &["config".into(), "path".into()])
        .ok()
        .map(|o| o.stdout.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| join(&home, ".config/ludusavi/config.yaml"));
    let exists = std::path::Path::new(&config_path).exists();
    let (mut backup_path, mut format, mut compression) = (None, None, None);
    if exists {
        match std::fs::read_to_string(&config_path) {
            Ok(yaml) => (backup_path, format, compression) = parse_config(&yaml, &home),
            Err(e) => log::error!("Failed to read Ludusavi config: {e}"),
        }
    }
    Some(LudusaviDetected {
        binary,
        config_path: exists.then_some(config_path),
        backup_path,
        format,
        compression,
        version,
    })
}

// ---------------------------------------------------------------------------
// Commands and output
// ---------------------------------------------------------------------------

/// `ludusavi find --api ...` lookups tried in order (Steam id, GOG id, title).
pub fn find_commands(title: &str, steam_app_id: Option<&str>, runner_name: &str, store_game_id: Option<&str>) -> Vec<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut out = Vec::new();
    if let Some(id) = steam_app_id.filter(|i| !i.is_empty()) {
        out.push(s(&["find", "--api", "--steam-id", id]));
    }
    if runner_name == "gog" {
        if let Some(id) = store_game_id.filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit())) {
            out.push(s(&["find", "--api", "--gog-id", id]));
        }
    }
    out.push(s(&["find", "--api", title]));
    out
}

/// Inputs of [`backup_command`].
#[derive(Debug, Clone)]
pub struct BackupCommand<'a> {
    /// Use Ludusavi's own config (no `--path`/`--format` overrides).
    pub use_installed: bool,
    pub configured_path: &'a str,
    pub format: &'a str,
    pub compression: &'a str,
    pub wine_prefix: Option<&'a str>,
    pub game_name: &'a str,
}

/// `ludusavi backup --force --api --no-cloud-sync [overrides] <game>`.
pub fn backup_command(c: &BackupCommand) -> Vec<String> {
    let mut cmd: Vec<String> = ["backup", "--force", "--api", "--no-cloud-sync"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if !c.use_installed {
        cmd.extend(["--path".into(), c.configured_path.into()]);
        cmd.extend(["--format".into(), c.format.into()]);
        if c.format == "zip" {
            cmd.extend(["--compression".into(), c.compression.into()]);
        }
    }
    if let Some(p) = c.wine_prefix {
        cmd.extend(["--wine-prefix".into(), p.into()]);
    }
    cmd.push(c.game_name.into());
    cmd
}

/// `parseJson`: `{}` for empty or invalid output.
pub fn parse_json(text: &str) -> Value {
    let t = text.trim();
    if t.is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(t).unwrap_or_else(|_| Value::Object(Default::default()))
}

/// `bestFindMatch`: the game with the highest `score` of a `find --api` reply
/// (the first one on ties).
pub fn best_find_match(payload: &Value) -> Option<String> {
    let games = payload.get("games")?.as_object()?;
    let mut best: Option<(&String, f64)> = None;
    for (name, info) in games {
        let score = info.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        if best.map_or(true, |(_, b)| score > b) {
            best = Some((name, score));
        }
    }
    best.map(|(n, _)| n.clone())
}

/// `overall.totalGames` of a backup reply.
pub fn total_games(payload: &Value) -> u64 {
    payload
        .get("overall")
        .and_then(|o| o.get("totalGames"))
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .unwrap_or(0)
}

/// `gameNamesFromPayload`.
pub fn game_names(payload: &Value) -> Vec<String> {
    payload
        .get("games")
        .and_then(Value::as_object)
        .map(|g| g.keys().cloned().collect())
        .unwrap_or_default()
}

/// `winePrefixForLudusavi`: the prefix itself or its `pfx` child, whichever
/// has `drive_c`.
pub fn wine_prefix_for_ludusavi(prefix: &str, probe: &dyn Probe) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    if probe.exists(&join(prefix, "drive_c")) {
        return Some(prefix.to_string());
    }
    let pfx = join(prefix, "pfx");
    probe.exists(&join(&pfx, "drive_c")).then_some(pfx)
}

// ---------------------------------------------------------------------------
// Backup flow
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupReason {
    Auto,
    Manual,
}

/// Arguments of `runLudusaviBackup`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviGame {
    pub app_name: String,
    pub title: String,
    pub runner: String,
    #[serde(default)]
    pub steam_app_id: Option<String>,
    #[serde(default)]
    pub store_game_id: Option<String>,
}

/// `LudusaviBackupResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviBackupResult {
    pub ran: bool,
    /// 'disabled' | 'not-found' | 'no-match' | 'already-running'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_games: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct InFlight;

impl InFlight {
    fn acquire() -> Option<Self> {
        IN_FLIGHT
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| InFlight)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// Everything the flow reads from or runs on the machine.
pub struct LudusaviEnv<'a> {
    pub paths: &'a HeroicPaths,
    pub runner: &'a dyn CommandRunner,
    pub probe: &'a dyn Probe,
}

fn iso_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn record_error(paths: &HeroicPaths, error: &str) {
    if let Err(e) = record_ludusavi_backup(
        paths,
        &LudusaviRecord { error: Some(error.to_string()), ..Default::default() },
    ) {
        log::error!("could not record Ludusavi error: {e:#}");
    }
}

fn extra_wine_prefix(env: &LudusaviEnv, app_name: &str) -> Option<String> {
    let metas: LocalMetaFile = json_store::read(&env.paths.local_meta()).ok()?;
    if metas.games.get(app_name).is_some_and(|m| m.launch_kind == "steam-uri") {
        return None;
    }
    let global = GlobalSettings::load(env.paths);
    let game = load_game_settings(env.paths, app_name, &global, env.probe);
    wine_prefix_for_ludusavi(&game.wine_prefix, env.probe)
}

/// `resolveLudusaviTitle`: Ludusavi's own name for the game (Steam/GOG id
/// first, then the title), falling back to the title.
fn resolve_title(env: &LudusaviEnv, binary: &str, game: &LudusaviGame) -> String {
    for cmd in find_commands(&game.title, game.steam_app_id.as_deref(), &game.runner, game.store_game_id.as_deref()) {
        if let Ok(out) = env.runner.run(binary, &cmd) {
            if let Some(m) = best_find_match(&parse_json(&out.stdout)) {
                return m;
            }
        }
    }
    game.title.clone()
}

/// `backupLudusaviForGame`.
pub fn backup_for_game(env: &LudusaviEnv, game: &LudusaviGame, reason: BackupReason) -> LudusaviBackupResult {
    log::info!("Ludusavi {reason:?} backup requested for {}", game.title);
    let Some(_guard) = InFlight::acquire() else {
        return LudusaviBackupResult { skipped_reason: Some("already-running".into()), ..Default::default() };
    };
    match backup_inner(env, game, reason) {
        Ok(r) => r,
        Err(e) => {
            let message = format!("{e:#}");
            record_error(env.paths, &message);
            log::error!("Ludusavi backup failed: {message}");
            LudusaviBackupResult { error: Some(message), ..Default::default() }
        }
    }
}

fn backup_inner(env: &LudusaviEnv, game: &LudusaviGame, reason: BackupReason) -> Result<LudusaviBackupResult> {
    let cfg = settings::load(env.paths)?.ludusavi;
    if reason == BackupReason::Auto && !cfg.enabled {
        return Ok(LudusaviBackupResult { skipped_reason: Some("disabled".into()), ..Default::default() });
    }
    let preferred = Some(cfg.binary_path.as_str()).filter(|p| !p.is_empty());
    let detected = detect_ludusavi(env.runner, env.probe, preferred);
    let binary = Some(cfg.binary_path.trim().to_string())
        .filter(|b| !b.is_empty())
        .or_else(|| detected.as_ref().map(|d| d.binary.clone()));
    let Some(binary) = binary.filter(|b| env.probe.exists(b)) else {
        let error = "Ludusavi was not found".to_string();
        record_error(env.paths, &error);
        return Ok(LudusaviBackupResult {
            skipped_reason: Some("not-found".into()),
            error: Some(error),
            ..Default::default()
        });
    };

    let use_installed = cfg.use_installed_config && detected.as_ref().is_some_and(|d| d.config_path.is_some());
    let configured = cfg.backup_path.trim().to_string();
    let detected_backup = detected.as_ref().and_then(|d| d.backup_path.clone());
    let backup_root = if use_installed {
        detected_backup
    } else {
        Some(configured.clone()).filter(|c| !c.is_empty()).or(detected_backup)
    };
    if !use_installed && configured.is_empty() {
        let error = "Set a Ludusavi backup folder in Collection settings".to_string();
        record_error(env.paths, &error);
        return Ok(LudusaviBackupResult { error: Some(error), ..Default::default() });
    }

    let wine_prefix = extra_wine_prefix(env, &game.app_name);
    let run_backup = |name: &str| -> Result<CommandOutput> {
        let cmd = backup_command(&BackupCommand {
            use_installed,
            configured_path: &configured,
            format: &cfg.format,
            compression: &cfg.compression,
            wine_prefix: wine_prefix.as_deref(),
            game_name: name,
        });
        log::info!("Running {binary} {}", cmd.join(" "));
        env.runner.run(&binary, &cmd).map_err(anyhow::Error::msg)
    };

    let mut game_name = game.title.clone();
    let mut result = run_backup(&game_name)?;
    let mut payload = parse_json(&result.stdout);
    let mut total = total_games(&payload);

    if result.code != 0 && total == 0 && (game.steam_app_id.as_deref().is_some_and(|s| !s.is_empty()) || game.runner == "gog") {
        game_name = resolve_title(env, &binary, game);
        if game_name != game.title {
            result = run_backup(&game_name)?;
            payload = parse_json(&result.stdout);
            total = total_games(&payload);
        }
    }

    if let Some(first) = game_names(&payload).into_iter().next() {
        game_name = first;
    }
    let backup_path = backup_root.as_deref().map(|r| join(r, &game_name));
    let stderr = result.stderr.trim().to_string();

    if result.code != 0 && total == 0 {
        let error = if stderr.is_empty() {
            format!("Ludusavi did not find saves for {}", game.title)
        } else {
            stderr
        };
        record_error(env.paths, &error);
        log::error!("Ludusavi backup failed: {error}");
        return Ok(LudusaviBackupResult {
            skipped_reason: Some("no-match".into()),
            game_name: Some(game_name),
            error: Some(error),
            ..Default::default()
        });
    }

    record_ludusavi_backup(
        env.paths,
        &LudusaviRecord {
            at: Some(iso_now()),
            game: Some(game_name.clone()),
            path: backup_path.clone(),
            error: None,
        },
    )?;
    Ok(LudusaviBackupResult {
        ran: true,
        game_name: Some(game_name),
        backup_path,
        total_games: Some(total),
        ..Default::default()
    })
}

/// Arguments of `backupLudusaviAfterPlay`.
#[derive(Debug, Clone)]
pub struct AfterPlay {
    pub app_name: String,
    pub title: String,
    pub runner: String,
    pub elapsed_seconds: u64,
}

/// `backupLudusaviAfterPlay`: skips sessions shorter than
/// [`SKIP_SESSION_SECONDS`] (returns `None`), otherwise runs an automatic
/// backup with the ids stored in the game's overlay meta.
pub fn backup_after_play(env: &LudusaviEnv, play: &AfterPlay) -> Option<LudusaviBackupResult> {
    if play.elapsed_seconds < SKIP_SESSION_SECONDS {
        log::info!("Skipping Ludusavi auto backup; session was only {}s", play.elapsed_seconds);
        return None;
    }
    let metas: LocalMetaFile = json_store::read(&env.paths.local_meta()).unwrap_or_default();
    let meta = metas.games.get(&play.app_name);
    let game = LudusaviGame {
        app_name: play.app_name.clone(),
        title: play.title.clone(),
        runner: play.runner.clone(),
        steam_app_id: meta.and_then(|m| m.steam_app_id.clone()),
        store_game_id: meta.and_then(|m| m.store_game_id.clone()),
    };
    let result = backup_for_game(env, &game, BackupReason::Auto);
    if !result.ran {
        if let Some(e) = &result.error {
            log::error!("Ludusavi save backup after play failed: {e}");
        }
    }
    Some(result)
}

/// The desktop notification `(title, body)` for a finished after-play backup.
pub fn notification(result: &LudusaviBackupResult, title: &str) -> Option<(String, String)> {
    if !result.ran {
        return None;
    }
    let name = result.game_name.as_deref().unwrap_or(title);
    let body = match &result.backup_path {
        Some(p) => format!("Saved {name} to {p}"),
        None => format!("Saved a backup for {name}"),
    };
    Some(("Ludusavi".into(), body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::probe::FakeProbe;
    use std::sync::Mutex;

    /// Serializes tests that hit the global in-flight guard.
    fn guard_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<Vec<String>>>,
        /// (prefix of args joined by space) -> output
        replies: Vec<(String, CommandOutput)>,
    }

    impl CommandRunner for Fake {
        fn run(&self, _binary: &str, args: &[String]) -> Result<CommandOutput, String> {
            self.calls.lock().unwrap().push(args.to_vec());
            let joined = args.join(" ");
            for (prefix, out) in &self.replies {
                if joined.starts_with(prefix.as_str()) {
                    return Ok(out.clone());
                }
            }
            Ok(CommandOutput::default())
        }
    }

    fn out(stdout: &str, stderr: &str, code: i32) -> CommandOutput {
        CommandOutput { stdout: stdout.into(), stderr: stderr.into(), code }
    }

    const YAML: &str = "runtime:\n  threads: ~\nbackup:\n  path: ~/Saves\n  ignoredGames:\n    - X\n  format:\n    chosen: zip\n    zip:\n      compression: zstd\n    compression:\n      deflate:\n        level: 6\nrestore:\n  path: /elsewhere\n";

    #[test]
    fn yaml_paths() {
        assert_eq!(yaml_get(YAML, &["backup", "path"]).as_deref(), Some("~/Saves"));
        assert_eq!(yaml_get(YAML, &["backup", "format", "chosen"]).as_deref(), Some("zip"));
        assert_eq!(yaml_get(YAML, &["backup", "format", "zip", "compression"]).as_deref(), Some("zstd"));
        assert_eq!(yaml_get(YAML, &["restore", "path"]).as_deref(), Some("/elsewhere"));
        assert_eq!(yaml_get(YAML, &["backup", "missing"]), None);
        assert_eq!(yaml_get(YAML, &["runtime", "threads"]).as_deref(), Some("~"));
        assert_eq!(yaml_get("a:\n  b: \"quoted\"\n", &["a", "b"]).as_deref(), Some("quoted"));
        assert_eq!(yaml_get("a:\n  b: |\n    text\n", &["a", "b"]), None);
        let (p, f, c) = parse_config(YAML, "/home/u");
        assert_eq!((p.as_deref(), f.as_deref(), c.as_deref()), (Some("/home/u/Saves"), Some("zip"), Some("zstd")));
    }

    #[test]
    fn command_construction() {
        let c = backup_command(&BackupCommand {
            use_installed: true,
            configured_path: "",
            format: "zip",
            compression: "deflate",
            wine_prefix: None,
            game_name: "Tony Hawk's Underground",
        });
        assert_eq!(c, ["backup", "--force", "--api", "--no-cloud-sync", "Tony Hawk's Underground"]);
        let c = backup_command(&BackupCommand {
            use_installed: false,
            configured_path: "/b",
            format: "zip",
            compression: "zstd",
            wine_prefix: Some("/pfx"),
            game_name: "G",
        });
        assert_eq!(
            c,
            ["backup", "--force", "--api", "--no-cloud-sync", "--path", "/b", "--format", "zip", "--compression", "zstd", "--wine-prefix", "/pfx", "G"]
        );
        let c = backup_command(&BackupCommand {
            use_installed: false,
            configured_path: "/b",
            format: "simple",
            compression: "zstd",
            wine_prefix: None,
            game_name: "G",
        });
        assert!(!c.contains(&"--compression".to_string()));
        assert_eq!(
            find_commands("Title", Some("42"), "sideload", None),
            vec![vec!["find", "--api", "--steam-id", "42"], vec!["find", "--api", "Title"]]
        );
        assert_eq!(
            find_commands("T", None, "gog", Some("123")),
            vec![vec!["find", "--api", "--gog-id", "123"], vec!["find", "--api", "T"]]
        );
        assert_eq!(find_commands("T", None, "gog", Some("abc")).len(), 1);
    }

    #[test]
    fn parses_ludusavi_json() {
        let find = parse_json(r#"{"games":{"Low":{"score":0.2},"High":{"score":0.9},"Mid":{"score":0.5}}}"#);
        assert_eq!(best_find_match(&find).as_deref(), Some("High"));
        assert_eq!(best_find_match(&parse_json("")), None);
        assert_eq!(best_find_match(&parse_json("not json")), None);
        let backup = parse_json(r#"{"overall":{"totalGames":2},"games":{"A":{},"B":{}}}"#);
        assert_eq!(total_games(&backup), 2);
        assert_eq!(game_names(&backup), ["A", "B"]);
        assert_eq!(total_games(&parse_json("{}")), 0);
    }

    #[test]
    fn detection_reads_version_and_config() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config.yaml");
        std::fs::write(&cfg, YAML).unwrap();
        let runner = Fake {
            replies: vec![
                ("--version".into(), out("ludusavi v0.30.0\n", "", 0)),
                ("config path".into(), out(&format!("{}\n", cfg.display()), "", 0)),
            ],
            ..Default::default()
        };
        let probe = FakeProbe::new("/home/u").with_bin("ludusavi", "/opt/bin/ludusavi");
        let det = detect_ludusavi(&runner, &probe, None).unwrap();
        assert_eq!(det.binary, "/opt/bin/ludusavi");
        assert_eq!(det.version.as_deref(), Some("v0.30.0"));
        assert_eq!(det.config_path.as_deref(), cfg.to_str());
        assert_eq!(det.backup_path.as_deref(), Some("/home/u/Saves"));
        assert_eq!(det.format.as_deref(), Some("zip"));
        assert_eq!(det.compression.as_deref(), Some("zstd"));
        // Preferred binary wins; nothing found -> None.
        let probe2 = FakeProbe::new("/home/u").with_path("/custom/lud");
        assert_eq!(detect_ludusavi(&runner, &probe2, Some(" /custom/lud ")).unwrap().binary, "/custom/lud");
        assert!(detect_ludusavi(&runner, &FakeProbe::new("/home/u"), None).is_none());
    }

    fn setup(settings_json: &str) -> (tempfile::TempDir, HeroicPaths) {
        let d = tempfile::tempdir().unwrap();
        let paths = HeroicPaths::at(d.path());
        std::fs::create_dir_all(paths.local_library()).unwrap();
        std::fs::write(paths.collection_settings(), settings_json).unwrap();
        (d, paths)
    }

    const ENABLED: &str = r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"binaryPath":"","backupPath":"/backups","format":"zip","compression":"zstd"}}"#;

    #[test]
    fn successful_backup_records_status() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let runner = Fake {
            replies: vec![
                ("backup".into(), out(r#"{"overall":{"totalGames":1},"games":{"Tony Hawk's Underground":{}}}"#, "", 0)),
            ],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame { app_name: "g".into(), title: "Tony Hawks Underground".into(), runner: "sideload".into(), ..Default::default() };
        let r = backup_for_game(&env, &game, BackupReason::Auto);
        assert!(r.ran, "{r:?}");
        assert_eq!(r.game_name.as_deref(), Some("Tony Hawk's Underground"));
        assert_eq!(r.backup_path.as_deref(), Some("/backups/Tony Hawk's Underground"));
        assert_eq!(r.total_games, Some(1));
        let calls = runner.calls.lock().unwrap();
        let backup_call = calls.iter().find(|c| c[0] == "backup").unwrap();
        assert_eq!(
            backup_call[..10],
            ["backup", "--force", "--api", "--no-cloud-sync", "--path", "/backups", "--format", "zip", "--compression", "zstd"]
        );
        assert_eq!(backup_call.last().unwrap(), "Tony Hawks Underground");
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.last_backup_game.as_deref(), Some("Tony Hawk's Underground"));
        assert_eq!(l.last_backup_path.as_deref(), Some("/backups/Tony Hawk's Underground"));
        assert!(l.last_backup_at.is_some());
        assert_eq!(l.last_error, None);
    }

    #[test]
    fn disabled_missing_binary_and_missing_folder() {
        let _l = guard_lock();
        let game = LudusaviGame { app_name: "g".into(), title: "T".into(), runner: "sideload".into(), ..Default::default() };
        let runner = Fake::default();
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        // Disabled: auto skipped, manual proceeds.
        let (_d, paths) = setup(r#"{"ludusavi":{"enabled":false,"useInstalledConfig":false,"backupPath":"/b"}}"#);
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let r = backup_for_game(&env, &game, BackupReason::Auto);
        assert_eq!(r.skipped_reason.as_deref(), Some("disabled"));
        assert!(runner.calls.lock().unwrap().is_empty());
        // Binary not found.
        let (_d2, paths2) = setup(ENABLED);
        let none = FakeProbe::new("/h");
        let env2 = LudusaviEnv { paths: &paths2, runner: &runner, probe: &none };
        let r = backup_for_game(&env2, &game, BackupReason::Manual);
        assert_eq!(r.skipped_reason.as_deref(), Some("not-found"));
        assert_eq!(r.error.as_deref(), Some("Ludusavi was not found"));
        assert_eq!(settings::load(&paths2).unwrap().ludusavi.last_error.as_deref(), Some("Ludusavi was not found"));
        // Own config but no folder.
        let (_d3, paths3) = setup(r#"{"ludusavi":{"enabled":true,"useInstalledConfig":false,"backupPath":"  "}}"#);
        let env3 = LudusaviEnv { paths: &paths3, runner: &runner, probe: &probe };
        let r = backup_for_game(&env3, &game, BackupReason::Auto);
        assert_eq!(r.error.as_deref(), Some("Set a Ludusavi backup folder in Collection settings"));
        assert!(!r.ran);
    }

    #[test]
    fn failed_lookup_retries_with_find_and_reports_no_match() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        let runner = Fake {
            replies: vec![
                ("find --api --steam-id 42".into(), out(r#"{"games":{"Real Name":{"score":1.0}}}"#, "", 0)),
                ("backup --force --api --no-cloud-sync --path /backups --format zip --compression zstd Real Name".into(), out("{}", "No info for these games:\n  - Real Name", 1)),
                ("backup".into(), out("{}", "", 1)),
            ],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let game = LudusaviGame { app_name: "g".into(), title: "Title".into(), runner: "sideload".into(), steam_app_id: Some("42".into()), ..Default::default() };
        let r = backup_for_game(&env, &game, BackupReason::Manual);
        assert!(!r.ran);
        assert_eq!(r.skipped_reason.as_deref(), Some("no-match"));
        assert_eq!(r.game_name.as_deref(), Some("Real Name"));
        assert_eq!(r.error.as_deref(), Some("No info for these games:\n  - Real Name"));
        let calls = runner.calls.lock().unwrap();
        assert!(calls.iter().any(|c| c.join(" ") == "find --api --steam-id 42"));
        assert_eq!(calls.iter().filter(|c| c[0] == "backup").count(), 2);
        drop(calls);
        let l = settings::load(&paths).unwrap().ludusavi;
        assert_eq!(l.last_error.as_deref(), Some("No info for these games:\n  - Real Name"));
    }

    #[test]
    fn after_play_skips_short_sessions_and_uses_meta_ids() {
        let _l = guard_lock();
        let (_d, paths) = setup(ENABLED);
        json_store::update::<LocalMetaFile, _>(&paths.local_meta(), |f| {
            f.games.insert(
                "g".into(),
                heronite_data::models::LocalGameMeta {
                    app_name: "g".into(),
                    runner: "sideload".into(),
                    steam_app_id: Some("777".into()),
                    ..Default::default()
                },
            );
            Ok(())
        })
        .unwrap();
        let runner = Fake {
            replies: vec![("backup".into(), out(r#"{"overall":{"totalGames":0},"games":{"G":{}}}"#, "", 0))],
            ..Default::default()
        };
        let probe = FakeProbe::new("/h").with_bin("ludusavi", "/usr/bin/ludusavi");
        let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &probe };
        let short = AfterPlay { app_name: "g".into(), title: "G".into(), runner: "sideload".into(), elapsed_seconds: 4 };
        assert!(backup_after_play(&env, &short).is_none());
        assert!(runner.calls.lock().unwrap().is_empty());
        let long = AfterPlay { elapsed_seconds: 60, ..short };
        let r = backup_after_play(&env, &long).unwrap();
        assert!(r.ran);
        assert_eq!(r.total_games, Some(0));
        let (title, body) = notification(&r, "G").unwrap();
        assert_eq!(title, "Ludusavi");
        assert_eq!(body, "Saved G to /backups/G");
        assert!(notification(&LudusaviBackupResult::default(), "G").is_none());
    }

    #[test]
    fn wine_prefix_detection() {
        let probe = FakeProbe::new("/h").with_path("/p1/drive_c").with_path("/p2/pfx/drive_c");
        assert_eq!(wine_prefix_for_ludusavi("/p1", &probe).as_deref(), Some("/p1"));
        assert_eq!(wine_prefix_for_ludusavi("/p2", &probe).as_deref(), Some("/p2/pfx"));
        assert_eq!(wine_prefix_for_ludusavi("/p3", &probe), None);
        assert_eq!(wine_prefix_for_ludusavi("", &probe), None);
    }

    #[test]
    fn system_runner_runs_and_times_out() {
        let r = SystemRunner::default();
        let ok = r.run("sh", &["-c".into(), "echo hi; echo err >&2; exit 3".into()]).unwrap();
        assert_eq!((ok.stdout.trim(), ok.stderr.trim(), ok.code), ("hi", "err", 3));
        assert!(r.run("/nonexistent/binary", &[]).is_err());
        let fast = SystemRunner { timeout: Duration::from_millis(100) };
        assert!(fast.run("sleep", &["5".into()]).unwrap_err().contains("timed out"));
    }
}
