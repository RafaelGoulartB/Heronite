//! Applying a fix of the launch diagnosis.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::diagnose::{max_map_count_script, Fix, MAX_MAP_COUNT};
use heronite_launch::settings::GameSettings;
use heronite_launch::{PlanContext, SystemProbe, SystemSpawner};
use serde_json::Value;

use super::Report;

/// How long the wineserver may take to end.
const WINESERVER_TIMEOUT: Duration = Duration::from_secs(30);

/// Applies `fix` to the game of `report`. `pid` gets the process group of
/// what runs for it (winetricks), which a cancel signals; `stop` set by the
/// cancel keeps the next component from starting. The output goes to
/// `line`. [`Fix::Terminal`] and [`Fix::Repair`] are the UI's (a copy to the
/// clipboard, the store's verify).
pub fn apply(
    paths: &DataPaths,
    report: &Report,
    fix: &Fix,
    pid: &AtomicU32,
    stop: &AtomicBool,
    line: &(dyn Fn(String) + Sync),
) -> Result<(), String> {
    let home = heronite_launch::probe::dirs_home();
    let app = report.app_name.as_str();
    match fix {
        Fix::Winetricks(verbs) => {
            for verb in verbs {
                if stop.load(Ordering::SeqCst) {
                    return Err("cancelled".into());
                }
                line(tr!("Installing {verb}\u{2026}", verb));
                let cancelled = || stop.load(Ordering::SeqCst);
                crate::runtime::winetricks_install_blocking(paths, &home, app, Err(()), verb, pid, &cancelled, line)
                    .map_err(|e| tr!("{verb} could not be installed: {why}", verb, why = e))?;
            }
            Ok(())
        }
        Fix::Settings(changes) => set(paths, app, &home, changes, None),
        Fix::UseProton(changes) => set(paths, app, &home, changes, Some(pick_proton(paths, None)?)),
        Fix::OtherProton => {
            let current = settings(paths, app, &home).wine_version.bin;
            set(paths, app, &home, &[], Some(pick_proton(paths, Some(&current))?))
        }
        Fix::StopWineserver => {
            let s = settings(paths, app, &home);
            let cmd = heronite_launch::wine_tools::kill_prefix_command(&PlanContext::load(paths), &SystemProbe, &s)?;
            line(tr!("Ending the wineserver of the prefix\u{2026}").into());
            heronite_launch::prepare::run_wait(&SystemSpawner::default(), &cmd, WINESERVER_TIMEOUT, &|| false);
            Ok(())
        }
        Fix::RaiseMaxMapCount => raise_max_map_count(line),
        Fix::Terminal(_) | Fix::Repair => Err("not applied here".into()),
    }
}

fn settings(paths: &DataPaths, app: &str, home: &str) -> GameSettings {
    GameSettings::from_map(&heronite_data::game_config::load_game(paths, app, home), home)
}

/// Writes `changes` (and the Wine version `wine`) into the game's settings.
fn set(
    paths: &DataPaths,
    app: &str,
    home: &str,
    changes: &[(&'static str, Value)],
    wine: Option<heronite_wine::WineInstallation>,
) -> Result<(), String> {
    let mut all: Vec<(String, Value)> = changes.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    if let Some(w) = wine {
        all.push(("wineVersion".into(), serde_json::to_value(w).map_err(|e| e.to_string())?));
    }
    heronite_data::game_config::set_game_settings(paths, app, home, &all).map_err(|e| format!("{e:#}"))
}

/// The best Proton on this computer, other than `not` (a Proton binary).
fn pick_proton(paths: &DataPaths, not: Option<&str>) -> Result<heronite_wine::WineInstallation, String> {
    let list: Vec<_> = heronite_wine::list_installations(paths)
        .into_iter()
        .filter(|w| w.kind == "proton" && heronite_wine::default::usable(w) && Some(w.bin.as_str()) != not)
        .collect();
    heronite_wine::default::best(&list).cloned().ok_or_else(|| tr!("No other Proton is installed. Get one in the Wine Manager.").into())
}

/// `pkexec /bin/sh -c <script>`: the desktop asks for the password.
fn raise_max_map_count(line: &(dyn Fn(String) + Sync)) -> Result<(), String> {
    let pkexec = ["/usr/bin/pkexec", "/bin/pkexec"].into_iter().find(|p| std::path::Path::new(p).exists());
    let Some(pkexec) = pkexec else { return Err(tr!("pkexec is not installed").into()) };
    line(tr!("Waiting for the administrator password\u{2026}").into());
    let status = std::process::Command::new(pkexec)
        .args(["/bin/sh", "-c", &max_map_count_script()])
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| tr!("Could not run pkexec: {error}", error = e))?;
    // 126: the dialog was dismissed; 127: not authorised.
    match status.code() {
        Some(0) => {}
        Some(126 | 127) => return Err(tr!("The password was not given: nothing changed.").into()),
        _ => return Err(tr!("The change failed ({status}).", status)),
    }
    let now = std::fs::read_to_string("/proc/sys/vm/max_map_count").ok().and_then(|t| t.trim().parse::<u64>().ok());
    match now {
        Some(n) if n >= MAX_MAP_COUNT => Ok(()),
        _ => Err(tr!("The change failed ({status}).", status)),
    }
}
