//! Why a game closed right after starting, and what fixes it.
//!
//! [`diagnose`] is pure: it reads the run's launch log (the game's output
//! after `Game Output:`), the game's settings, the DLLs its executable
//! imports ([`pe::imports`]), the winetricks components of its prefix, its
//! AreWeAntiCheatYet entry and a few facts about the computer
//! ([`System::read`]). Each [`Finding`] names a known problem, the log lines
//! that show it and the [`Fix`]es that solve it, the one to offer first at
//! the front. Nothing here changes anything: the app applies the fixes.
//!
//! Two kinds of findings: [`Finding::actionable`] ones (the cause is shown
//! and a fix solves it) are worth interrupting the player right after a
//! failed start ([`opens_by_itself`]); the others (a crash the log does not
//! explain, a likely cause, an anti-cheat that refuses Linux) are only for
//! the player who asks ("Troubleshoot"), whenever the game ended.
//!
//! The log-based findings need the launch log, which `verboseLogs: false`
//! turns off (then [`Problem::LogsOff`] asks to turn it on). With the log on,
//! Wine's "DLL not found" errors reach it: through `+fixme` with verbose
//! logs, through [`crate::env::QUIET_WINEDEBUG`] for a Proton game whose
//! settings leave the key out.

mod dlls;
mod log;
pub mod pe;
mod system;
#[cfg(test)]
mod tests;

use heronite_data::anticheat::AntiCheatInfo;
use serde_json::Value;

use crate::settings::GameSettings;

pub use dlls::verb_for_dll;
pub use system::{max_map_count_script, Distro, System, MAX_MAP_COUNT, SYSCTL_FILE};

/// How the run started the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// A Windows program through plain Wine.
    Wine,
    /// A Windows program through Proton (directly or through umu).
    Proton,
    /// A Linux program.
    Native,
}

impl RunKind {
    pub fn windows(self) -> bool {
        self != RunKind::Native
    }
}

/// What AreWeAntiCheatYet says of the game's anti-cheat on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiCheatStatus {
    /// `Supported` / `Running`: works with the anti-cheat runtime.
    Works,
    /// `Broken` / `Planned`: does not work for now.
    Broken,
    /// `Denied`: the developer keeps Linux players out.
    Denied,
}

/// A problem the analysis recognises.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// DLLs the game needs are missing from the prefix, as written in the
    /// log (`MSVCP140.dll`). `verbs`: the winetricks components that bring
    /// them; DLLs no component brings are only named.
    MissingDlls { dlls: Vec<String>, verbs: Vec<String> },
    /// The game runs on .NET and Wine Mono could not run it.
    DotNet,
    /// A DirectX 12 game through plain Wine without VKD3D-Proton.
    Dx12WithoutVkd3d,
    /// A DirectX 9-11 game through plain Wine without DXVK whose log shows
    /// Wine's own Direct3D failing.
    Direct3dWithoutDxvk,
    /// The game uses an anti-cheat (`names`, as AreWeAntiCheatYet writes them).
    AntiCheat { names: Vec<String>, status: AntiCheatStatus },
    /// Vulkan could not start: no driver, or none for 32-bit programs.
    Vulkan,
    /// Wine ran out of file descriptors (esync with a low limit).
    TooManyOpenFiles,
    /// `vm.max_map_count` is below what many games need.
    MaxMapCount { current: u64 },
    /// Linux libraries the game (or Wine) needs are not installed.
    /// `bits32`: the 32-bit ones are the missing part.
    MissingLibs { libs: Vec<String>, bits32: bool },
    /// Wine cannot find the game's executable.
    ExecutableMissing,
    /// A wineserver of another Wine version still runs on the prefix.
    Wineserver,
    /// The game crashed, and the log says no more than that.
    Crash,
    /// The launch log is off for this game, so the cause cannot be read.
    LogsOff,
}

/// A change that solves a [`Problem`], applied by the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// Installs these winetricks components (`winetricks -q`) in the prefix.
    Winetricks(Vec<String>),
    /// Changes these keys of the game's settings (`GamesConfig/<app>.json`).
    Settings(Vec<(&'static str, Value)>),
    /// Runs the game with the best Proton on this computer (the newest
    /// GE-Proton first), then changes these keys too.
    UseProton(Vec<(&'static str, Value)>),
    /// Runs the game with another Proton than the one it has.
    OtherProton,
    /// Ends the wineserver left on the prefix.
    StopWineserver,
    /// Raises `vm.max_map_count` to [`MAX_MAP_COUNT`] now and for good
    /// (through polkit, [`max_map_count_script`]).
    RaiseMaxMapCount,
    /// A command for the user to run in a terminal (installing packages).
    Terminal(String),
    /// Verifies and repairs the game's files through its store.
    Repair,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub problem: Problem,
    /// The log lines (or facts) that show it, as written, at most a few.
    pub evidence: Vec<String>,
    /// The fixes, the one to offer first at the front. Empty: nothing the
    /// app can do (a denied anti-cheat).
    pub fixes: Vec<Fix>,
    /// The cause is known: shown by the log or by the game's data, not
    /// merely likely (a crash alone never says its cause).
    pub sure: bool,
}

impl Finding {
    /// A known cause with a fix: worth opening the dialog for by itself.
    pub fn actionable(&self) -> bool {
        self.sure && !self.fixes.is_empty()
    }
}

/// What [`diagnose`] reads.
pub struct Inputs<'a> {
    /// The run's launch log (or its end). `None`: no log.
    pub log: Option<&'a str>,
    pub kind: RunKind,
    pub settings: &'a GameSettings,
    /// DLLs the game's executable imports, lowercase (empty: unknown).
    pub imports: &'a [String],
    /// winetricks components already in the prefix.
    pub winetricks: &'a [String],
    pub anticheat: Option<&'a AntiCheatInfo>,
    pub system: &'a System,
    /// The run ended badly (a non-zero exit code or a signal).
    pub failed: bool,
    /// The game's store can verify its files.
    pub can_repair: bool,
}

/// Lines kept as evidence of one problem.
const EVIDENCE: usize = 4;

/// The problems of a run, most likely cause first.
pub fn diagnose(i: &Inputs<'_>) -> Vec<Finding> {
    let out = i.log.map(log::game_output).unwrap_or_default();
    let logs_off = i.log.is_some_and(log::is_disabled);
    let mut found = Vec::new();
    if i.kind.windows() {
        found.extend(executable_missing(i, out));
    }
    found.extend(missing_libs(i, out));
    if i.kind.windows() {
        found.extend(wineserver(out));
        found.extend(missing_dlls(i, out));
        found.extend(dotnet(i, out));
    }
    found.extend(vulkan(i, out));
    if i.kind.windows() {
        found.extend(too_many_files(i, out));
        found.extend(anticheat(i));
        found.extend(dx12(i, out));
        found.extend(direct3d(i, out));
    }
    let sure = found.iter().any(|f: &Finding| f.sure);
    found.extend(max_map_count(i, out, sure));
    if !sure && i.kind.windows() {
        found.extend(crash(i, out));
    }
    if logs_off && !sure {
        found.push(Finding {
            problem: Problem::LogsOff,
            evidence: Vec::new(),
            fixes: vec![Fix::Settings(vec![("verboseLogs", Value::Bool(true))])],
            sure: false,
        });
    }
    // What can be fixed first; the order of each part stays.
    found.sort_by_key(|f| !f.actionable());
    found
}

/// Whether the dialog should open by itself after a failed start: only for
/// a known cause with a fix. Anything else waits for "Troubleshoot".
pub fn opens_by_itself(findings: &[Finding]) -> bool {
    findings.iter().any(Finding::actionable)
}

/// `(key, true)` settings changes.
fn on(keys: &[&'static str]) -> Vec<(&'static str, Value)> {
    keys.iter().map(|k| (*k, Value::Bool(true))).collect()
}

fn executable_missing(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let evidence = log::lines(out, log::EXE_MISSING, EVIDENCE);
    (!evidence.is_empty()).then(|| Finding {
        problem: Problem::ExecutableMissing,
        evidence,
        fixes: if i.can_repair { vec![Fix::Repair] } else { Vec::new() },
        sure: true,
    })
}

fn missing_libs(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let missing = log::missing_libs(out);
    let wine32 = i.kind.windows() && log::any(out, log::WINE32_MISSING);
    if missing.libs.is_empty() && !wine32 {
        return None;
    }
    let bits32 = wine32 || missing.wrong_class || missing.libs.iter().any(|l| i.system.has_64bit_lib(l));
    let mut fixes = Vec::new();
    if i.kind == RunKind::Wine {
        // Proton brings its own Linux libraries (the Steam runtime).
        fixes.push(Fix::UseProton(Vec::new()));
    }
    let libs: Vec<String> = missing.libs.clone();
    if let Some(cmd) = i.system.distro.find_packages(&libs, bits32) {
        fixes.push(Fix::Terminal(cmd));
    }
    let mut evidence = missing.lines;
    evidence.extend(log::lines(out, log::WINE32_MISSING, EVIDENCE));
    evidence.truncate(EVIDENCE);
    Some(Finding { problem: Problem::MissingLibs { libs, bits32 }, evidence, fixes, sure: true })
}

fn wineserver(out: &str) -> Option<Finding> {
    let evidence = log::lines(out, log::WINESERVER, EVIDENCE);
    (!evidence.is_empty()).then(|| Finding { problem: Problem::Wineserver, evidence, fixes: vec![Fix::StopWineserver], sure: true })
}

fn missing_dlls(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let missing = log::missing_dlls(out);
    if missing.is_empty() {
        return None;
    }
    let mut verbs: Vec<String> = Vec::new();
    for (dll, _) in &missing {
        if let Some(v) = verb_for_dll(dll) {
            if !verbs.iter().any(|x| x == v) {
                verbs.push(v.to_string());
            }
        }
    }
    // A component already in the prefix that still misses its DLL was
    // installed for the other architecture or got damaged: installing it
    // again (`winetricks -q --force`) is not something to do blindly.
    verbs.retain(|v| !i.winetricks.iter().any(|w| w == v));
    let mut fixes = Vec::new();
    if !verbs.is_empty() {
        fixes.push(Fix::Winetricks(verbs.clone()));
    }
    let unknown = missing.iter().any(|(dll, _)| verb_for_dll(dll).is_none());
    if unknown && i.can_repair {
        fixes.push(Fix::Repair);
    }
    let dlls = missing.iter().map(|(d, _)| d.clone()).collect();
    let evidence = missing.into_iter().map(|(_, line)| line).take(EVIDENCE).collect();
    Some(Finding { problem: Problem::MissingDlls { dlls, verbs }, evidence, fixes, sure: true })
}

fn dotnet(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let evidence = log::lines(out, log::MONO, EVIDENCE);
    if evidence.is_empty() {
        return None;
    }
    let installed = i.winetricks.iter().any(|w| w.starts_with("dotnet"));
    let fixes = if installed { Vec::new() } else { vec![Fix::Winetricks(vec!["dotnet48".into()])] };
    Some(Finding { problem: Problem::DotNet, evidence, fixes, sure: true })
}

fn vulkan(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let evidence = log::lines(out, log::VULKAN, EVIDENCE);
    if evidence.is_empty() {
        return None;
    }
    let fixes = i.system.distro.vulkan_packages(i.system.gpu, i.system.nvidia_driver).map(Fix::Terminal).into_iter().collect();
    Some(Finding { problem: Problem::Vulkan, evidence, fixes, sure: true })
}

fn too_many_files(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let evidence = log::lines(out, log::TOO_MANY_FILES, EVIDENCE);
    if evidence.is_empty() {
        return None;
    }
    let s = i.settings;
    let mut off = Vec::new();
    if s.enable_esync {
        off.push(("enableEsync", Value::Bool(false)));
    }
    // fsync needs no file descriptors: the better sync to keep.
    if !s.enable_fsync {
        off.push(("enableFsync", Value::Bool(true)));
    }
    let fixes = if s.enable_esync { vec![Fix::Settings(off)] } else { Vec::new() };
    Some(Finding { problem: Problem::TooManyOpenFiles, evidence, fixes, sure: true })
}

fn anticheat(i: &Inputs<'_>) -> Option<Finding> {
    let info = i.anticheat?;
    if info.anticheats.is_empty() {
        return None;
    }
    let status = match info.status.as_str() {
        "Supported" | "Running" => AntiCheatStatus::Works,
        "Broken" | "Planned" => AntiCheatStatus::Broken,
        "Denied" => AntiCheatStatus::Denied,
        _ => return None,
    };
    let names = info.anticheats.clone();
    let evidence = vec![format!("AreWeAntiCheatYet: {} ({})", info.status, names.join(", "))];
    if status != AntiCheatStatus::Works {
        return Some(Finding { problem: Problem::AntiCheat { names, status }, evidence, fixes: Vec::new(), sure: true });
    }
    let s = i.settings;
    let mut keys = Vec::new();
    if names.iter().any(|n| n == "Easy Anti-Cheat") && !s.eac_runtime {
        keys.push("eacRuntime");
    }
    if names.iter().any(|n| n == "BattlEye") && !s.battleye_runtime {
        keys.push("battlEyeRuntime");
    }
    // Only Proton loads the runtimes.
    let fix = match (keys.is_empty(), i.kind) {
        (true, RunKind::Proton) => return None,
        (_, RunKind::Wine) => Fix::UseProton(on(&keys)),
        _ => Fix::Settings(on(&keys)),
    };
    Some(Finding { problem: Problem::AntiCheat { names, status }, evidence, fixes: vec![fix], sure: true })
}

fn dx12(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    if i.kind != RunKind::Wine || i.settings.auto_install_vkd3d || !i.imports.iter().any(|d| d == "d3d12.dll") {
        return None;
    }
    let mut evidence = vec!["d3d12.dll".to_string()];
    evidence.extend(log::lines(out, log::D3D12, EVIDENCE - 1));
    Some(Finding {
        problem: Problem::Dx12WithoutVkd3d,
        evidence,
        fixes: vec![Fix::Settings(on(&["autoInstallVkd3d"])), Fix::UseProton(Vec::new())],
        sure: true,
    })
}

fn direct3d(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    const D3D: [&str; 5] = ["d3d9.dll", "d3d10.dll", "d3d10_1.dll", "d3d11.dll", "dxgi.dll"];
    if i.kind != RunKind::Wine || i.settings.auto_install_dxvk {
        return None;
    }
    let evidence = log::lines(out, log::WINED3D, EVIDENCE);
    if evidence.is_empty() || !(i.imports.is_empty() || i.imports.iter().any(|d| D3D.contains(&d.as_str()))) {
        return None;
    }
    Some(Finding { problem: Problem::Direct3dWithoutDxvk, evidence, fixes: vec![Fix::Settings(on(&["autoInstallDxvk"]))], sure: true })
}

/// Proton writes its warning only with `PROTON_LOG`; otherwise a low value
/// is a likely cause of a crash the log does not explain.
fn max_map_count(i: &Inputs<'_>, out: &str, explained: bool) -> Option<Finding> {
    let current = i.system.max_map_count.filter(|n| *n < MAX_MAP_COUNT)?;
    let evidence = log::lines(out, log::MAX_MAP_COUNT, 1);
    let sure = !evidence.is_empty();
    if !sure && (explained || !i.failed) {
        return None;
    }
    let evidence = if sure { evidence } else { vec![format!("vm.max_map_count = {current}")] };
    Some(Finding { problem: Problem::MaxMapCount { current }, evidence, fixes: vec![Fix::RaiseMaxMapCount], sure })
}

fn crash(i: &Inputs<'_>, out: &str) -> Option<Finding> {
    let evidence = log::lines(out, log::CRASH, EVIDENCE);
    if evidence.is_empty() && !i.failed {
        return None;
    }
    let fixes = if i.kind == RunKind::Wine { vec![Fix::UseProton(Vec::new())] } else { vec![Fix::OtherProton] };
    // Another Proton may help; the cause is still unknown.
    Some(Finding { problem: Problem::Crash, evidence, fixes, sure: false })
}

/// How the run was started, from its log's header (`Native? true`) and the
/// game's Wine. `None`: a Steam game (Steam started it) or no header.
pub fn run_kind(log: &str, settings: &GameSettings) -> Option<RunKind> {
    if log.contains("Steam game, opened through") {
        return None;
    }
    if log.contains("Native? true") {
        return Some(RunKind::Native);
    }
    if !log.contains("Native? false") {
        return None;
    }
    Some(match settings.wine_version.kind.as_str() {
        "proton" => RunKind::Proton,
        _ => RunKind::Wine,
    })
}
