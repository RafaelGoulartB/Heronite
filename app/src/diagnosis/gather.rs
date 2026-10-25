//! Everything the analysis reads for one game, read from disk.

use std::path::{Path, PathBuf};

use heronite_data::{CollectionGame, DataPaths};
use heronite_launch::diagnose::{self, Fix, Inputs, RunKind, System};
use heronite_launch::log_writer::{GameLogType, LogPaths};
use heronite_launch::settings::GameSettings;
use heronite_launch::SystemProbe;

use super::{Ended, Report};

/// How much of the end of the launch log is read (a chatty game's log
/// reaches hundreds of MB; the cause is near the start of its output, which
/// a log this size still holds for a run that ended within seconds).
const LOG_TAIL: u64 = 1 << 20;

/// Diagnoses the last launch of `game` (`log`: another log file to read,
/// for the debug hooks). `None`: a game this cannot help with (Steam starts
/// it, or another launcher or an emulator).
pub fn gather(paths: &DataPaths, game: &CollectionGame, ended: Option<Ended>, log: Option<&Path>) -> Option<Report> {
    if game.via_steam || game.runner == "steam" || matches!(game.launch_kind.as_deref(), Some("steam-uri" | "emulator" | "launcher")) {
        return None;
    }
    let path =
        log.map(Path::to_path_buf).unwrap_or_else(|| LogPaths::for_data(paths).game(&game.app_name, &game.runner, GameLogType::Launch));
    let text = heronite_launch::log_reader::read_tail(&path, LOG_TAIL).ok().map(|t| t.text);
    let home = heronite_launch::probe::dirs_home();
    let settings = GameSettings::from_map(&heronite_data::game_config::load_game(paths, &game.app_name, &home), &home);
    let kind = text.as_deref().and_then(|t| diagnose::run_kind(t, &settings)).or_else(|| fallback_kind(game, &settings))?;
    let windows = kind.windows();
    let imports = match windows {
        true => executable(game, &settings).and_then(|exe| diagnose::pe::imports(&exe).ok()).unwrap_or_default(),
        false => Vec::new(),
    };
    let winetricks = if windows { heronite_launch::winetricks::installed(&settings.wine_prefix) } else { Vec::new() };
    let anticheat = windows.then(|| heronite_data::anticheat::lookup(paths, game.steam_app_id.as_deref(), &game.title)).flatten();
    let system = System::read(&SystemProbe);
    let failed = match ended {
        Some(e) => e.failed(),
        None => text.as_deref().is_some_and(last_run_failed),
    };
    let protons = protons(paths);
    let best_proton = heronite_wine::default::best(&protons).cloned();
    let others: Vec<_> = protons.iter().filter(|w| w.bin != settings.wine_version.bin).cloned().collect();
    let other_proton = heronite_wine::default::best(&others).cloned();
    let mut findings = diagnose::diagnose(&Inputs {
        log: text.as_deref(),
        kind,
        settings: &settings,
        imports: &imports,
        winetricks: &winetricks,
        anticheat: anticheat.as_ref(),
        system: &system,
        failed,
        can_repair: game.installed && matches!(game.runner.as_str(), "legendary" | "gog" | "nile"),
    });
    // A switch needs a Proton to switch to.
    for f in &mut findings {
        f.fixes.retain(|fix| match fix {
            Fix::UseProton(_) => best_proton.is_some(),
            Fix::OtherProton => other_proton.is_some(),
            _ => true,
        });
    }
    Some(Report {
        key: game.key.clone(),
        app_name: game.app_name.clone(),
        title: game.title.clone(),
        kind,
        ended,
        failed,
        findings,
        has_log: text.is_some(),
        steam_app_id: game.steam_app_id.clone(),
        best_proton: best_proton.map(|w| w.name),
        other_proton: other_proton.map(|w| w.name),
    })
}

/// No log header to read it from: the game's platform and Wine.
fn fallback_kind(game: &CollectionGame, settings: &GameSettings) -> Option<RunKind> {
    let platform = game.game.install.platform.as_deref().unwrap_or_default().to_ascii_lowercase();
    if game.game.is_linux_native == Some(true) || platform == "linux" {
        return Some(RunKind::Native);
    }
    match settings.wine_version.kind.as_str() {
        "proton" => Some(RunKind::Proton),
        "wine" => Some(RunKind::Wine),
        _ => None,
    }
}

/// The Windows program the game starts: its target executable, else the
/// store's or the sideload entry's (relative to the install folder).
fn executable(game: &CollectionGame, settings: &GameSettings) -> Option<PathBuf> {
    let install = &game.game.install;
    let exe = Some(settings.target_exe.trim()).filter(|t| !t.is_empty()).or(install.executable.as_deref())?;
    let path = match Path::new(exe).is_absolute() {
        true => PathBuf::from(exe),
        false => Path::new(install.install_path.as_deref()?).join(exe.replace('\\', "/")),
    };
    (heronite_launch::wine_tools::is_windows_program(&path.to_string_lossy()) && path.is_file()).then_some(path)
}

/// The Protons on this computer that can run games.
fn protons(paths: &DataPaths) -> Vec<heronite_wine::WineInstallation> {
    heronite_wine::list_installations(paths).into_iter().filter(|w| w.kind == "proton" && heronite_wine::default::usable(w)).collect()
}

/// The log's last exit line says the game ended badly.
fn last_run_failed(log: &str) -> bool {
    let Some(line) = log.lines().rev().find(|l| l.contains("Game process exited with code") || l.contains("terminated by signal")) else {
        return false;
    };
    line.contains("terminated by signal") || !line.trim_end().ends_with("code 0")
}

#[cfg(test)]
mod tests {
    use super::last_run_failed;

    #[test]
    fn the_last_exit_line_tells_a_failed_run() {
        assert!(last_run_failed("(10:00:01) [INFO]:    Game process exited with code 53\n============= End of log ============="));
        assert!(last_run_failed("Game process was terminated by signal 11"));
        assert!(!last_run_failed("Game process exited with code 0\n"));
        assert!(!last_run_failed("no exit line"));
    }
}
