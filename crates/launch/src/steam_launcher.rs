//! Which Steam client a `steam://` URI goes to when the desktop could not
//! hand it over (see `SystemSpawner::open_uri`). `steam` on `$PATH` is not
//! always the Steam that runs: a native package left beside the Flatpak or
//! the Snap would start a second client. The running client says which one
//! it is, through its cgroup (Flatpak and Snap put their apps in scopes
//! named after them) and the program it runs; only when exactly one kind of
//! Steam is seen running is its own launcher used. Otherwise the choice
//! stays [`find_steam_binary`]'s.

use std::path::{Path, PathBuf};

use crate::probe::Probe;
use crate::steam::{find_steam_binary, FLATPAK_STEAM_ID};

/// A kind of Steam client that runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunningSteam {
    /// Steam's own install, at this folder (`steam.sh` in it).
    Native(PathBuf),
    Flatpak,
    Snap,
}

/// What one of this user's running processes says about itself: its name
/// (`/proc/<pid>/comm`), its cgroup (`/proc/<pid>/cgroup`) and its program
/// (`/proc/<pid>/exe`, when it can be read).
#[derive(Debug, Clone, Default)]
pub struct ProcessInfo {
    pub comm: String,
    pub cgroup: String,
    pub exe: Option<PathBuf>,
}

/// The systemd scope `flatpak run` puts an app in: `app-flatpak-<id>-<n>.scope`.
fn in_flatpak_steam(cgroup: &str) -> bool {
    cgroup.contains(&format!("app-flatpak-{FLATPAK_STEAM_ID}-"))
}

/// The scope snapd puts the Steam snap's commands in: `snap.steam.<app>-<id>.scope`.
fn in_snap_steam(cgroup: &str) -> bool {
    cgroup.contains("snap.steam.")
}

/// The folder of a native Steam client program
/// (`<root>/ubuntu12_32/steam`, `<root>/steamrt64/steam`): not one in the
/// Flatpak's or the Snap's data.
fn native_root(exe: &Path) -> Option<PathBuf> {
    if exe.file_name()? != "steam" {
        return None;
    }
    let dir = exe.parent()?;
    if !matches!(dir.file_name()?.to_str()?, "ubuntu12_32" | "ubuntu12_64" | "steamrt64") {
        return None;
    }
    let text = exe.to_string_lossy();
    if text.contains(&format!("/.var/app/{FLATPAK_STEAM_ID}/")) || text.starts_with("/snap/") || text.contains("/snap/steam/") {
        return None;
    }
    dir.parent().map(Path::to_path_buf)
}

/// The one kind of Steam client running among `processes`; `None` when none
/// is seen, or more than one (then nothing tells which got the URI wrong).
/// Only the client itself counts (a process named `steam`): a game left
/// running in the Flatpak's scope after Steam closed is not Steam.
pub fn running_steam(processes: &[ProcessInfo]) -> Option<RunningSteam> {
    let clients: Vec<&ProcessInfo> = processes.iter().filter(|p| p.comm == "steam").collect();
    let flatpak = clients.iter().any(|p| in_flatpak_steam(&p.cgroup));
    let snap = clients.iter().any(|p| in_snap_steam(&p.cgroup));
    let mut native: Vec<PathBuf> = clients
        .iter()
        .filter(|p| !in_flatpak_steam(&p.cgroup) && !in_snap_steam(&p.cgroup))
        .filter_map(|p| p.exe.as_deref().and_then(native_root))
        .collect();
    native.sort();
    native.dedup();
    match (flatpak, snap, native.len()) {
        (true, false, 0) => Some(RunningSteam::Flatpak),
        (false, true, 0) => Some(RunningSteam::Snap),
        (false, false, 1) => native.pop().map(RunningSteam::Native),
        _ => None,
    }
}

/// Whether `path` is a launcher of the Flatpak or the Snap, not of a native Steam.
fn is_sandboxed_launcher(path: &str) -> bool {
    path.starts_with("/snap/") || path.ends_with(&format!("/{FLATPAK_STEAM_ID}")) || path == FLATPAK_STEAM_ID
}

/// The command (program and arguments, the URI goes last) that hands a URI
/// to `steam`, when its launcher is there and can be run.
pub fn launcher_for(steam: &RunningSteam, probe: &dyn Probe) -> Option<Vec<String>> {
    let home = probe.home();
    match steam {
        RunningSteam::Flatpak => {
            let exports = [
                format!("{}/.local/share/flatpak/exports/bin/{FLATPAK_STEAM_ID}", home.trim_end_matches('/')),
                format!("/var/lib/flatpak/exports/bin/{FLATPAK_STEAM_ID}"),
            ];
            probe
                .which(FLATPAK_STEAM_ID)
                .into_iter()
                .chain(exports)
                .find(|p| probe.is_executable(p))
                .map(|p| vec![p])
                .or_else(|| probe.which("flatpak").map(|f| vec![f, "run".into(), FLATPAK_STEAM_ID.into()]))
        }
        RunningSteam::Snap => ["/snap/bin/steam".to_string()]
            .into_iter()
            .chain(probe.which("steam").filter(|p| p.starts_with("/snap/")))
            .find(|p| probe.is_executable(p))
            .map(|p| vec![p]),
        RunningSteam::Native(root) => {
            // The system's launcher (the package's `steam`), else the
            // install's own `steam.sh`.
            let on_path = probe.which("steam").filter(|p| !is_sandboxed_launcher(p));
            let script = root.join("steam.sh").to_string_lossy().into_owned();
            on_path
                .into_iter()
                .chain(["/usr/bin/steam".to_string(), "/usr/games/steam".to_string(), script])
                .find(|p| probe.is_executable(p))
                .map(|p| vec![p])
        }
    }
}

/// The commands that hand a URI to the Steam client, to try in order: the
/// running client's own launcher when exactly one kind of Steam runs and
/// its launcher is found, then [`find_steam_binary`]'s (what is used when
/// nothing is sure, or the first one cannot be started).
pub fn client_commands(probe: &dyn Probe, processes: &[ProcessInfo]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = running_steam(processes).and_then(|steam| launcher_for(&steam, probe)).into_iter().collect();
    if let Some(usual) = find_steam_binary(probe).map(|b| vec![b]) {
        if !out.contains(&usual) {
            out.push(usual);
        }
    }
    out
}

/// `/proc/<pid>/exe` of a program replaced while it runs (Steam updates
/// itself in place) ends in ` (deleted)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn without_deleted(exe: PathBuf) -> PathBuf {
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(s) => PathBuf::from(s),
        None => exe,
    }
}

/// This user's processes named `steam`, read from `/proc` (another user's
/// Steam on the same computer is not this user's).
#[cfg(target_os = "linux")]
pub fn steam_processes() -> Vec<ProcessInfo> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let dir = format!("/proc/{pid}");
        if !std::fs::metadata(&dir).is_ok_and(|m| m.uid() == me) {
            continue;
        }
        let comm = std::fs::read_to_string(format!("{dir}/comm")).unwrap_or_default().trim().to_string();
        if comm != "steam" {
            continue;
        }
        let cgroup = std::fs::read_to_string(format!("{dir}/cgroup")).unwrap_or_default();
        let exe = std::fs::read_link(format!("{dir}/exe")).ok().map(without_deleted);
        out.push(ProcessInfo { comm, cgroup, exe });
    }
    out
}

/// Nothing tells elsewhere than Linux.
#[cfg(not(target_os = "linux"))]
pub fn steam_processes() -> Vec<ProcessInfo> {
    Vec::new()
}

// Detection is for Linux; Windows finds Steam by its folders, not `$PATH`.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::probe::FakeProbe;

    /// The client (`steam`) in this cgroup.
    fn proc_in(cgroup: &str) -> ProcessInfo {
        ProcessInfo { comm: "steam".into(), cgroup: cgroup.into(), exe: None }
    }

    fn native(exe: &str) -> ProcessInfo {
        ProcessInfo {
            comm: "steam".into(),
            cgroup: "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-steam-4242.scope\n".into(),
            exe: Some(exe.into()),
        }
    }

    const FLATPAK_CG: &str =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-com.valvesoftware.Steam-123456.scope\n";
    const SNAP_CG: &str =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/snap.steam.steam-0f3c8e5a-1b2c-4d5e-8f90-123456789abc.scope\n";

    #[test]
    fn the_running_kind_is_told_only_when_it_is_one() {
        assert_eq!(running_steam(&[proc_in(FLATPAK_CG), proc_in(FLATPAK_CG)]), Some(RunningSteam::Flatpak));
        assert_eq!(running_steam(&[proc_in(SNAP_CG)]), Some(RunningSteam::Snap));
        assert_eq!(
            running_steam(&[
                native("/home/u/.local/share/Steam/ubuntu12_32/steam"),
                native("/home/u/.local/share/Steam/ubuntu12_32/steam")
            ]),
            Some(RunningSteam::Native("/home/u/.local/share/Steam".into()))
        );
        assert_eq!(
            running_steam(&[native("/home/u/.steam/debian-installation/steamrt64/steam")]),
            Some(RunningSteam::Native("/home/u/.steam/debian-installation".into()))
        );
        // Nothing running, or two kinds at once: nothing is told.
        assert_eq!(running_steam(&[]), None);
        assert_eq!(running_steam(&[proc_in(FLATPAK_CG), native("/home/u/.local/share/Steam/ubuntu12_32/steam")]), None);
        assert_eq!(running_steam(&[proc_in(FLATPAK_CG), proc_in(SNAP_CG)]), None);
        assert_eq!(
            running_steam(&[native("/home/u/.local/share/Steam/ubuntu12_32/steam"), native("/opt/other/Steam/ubuntu12_32/steam")]),
            None,
            "two native installs"
        );
    }

    #[test]
    fn programs_that_are_not_a_native_client_say_nothing() {
        for exe in [
            // The launcher script's interpreter, a tool called steam, the
            // Flatpak's and the Snap's clients seen from the host.
            "/usr/bin/bash",
            "/usr/local/bin/steam",
            "/home/u/.var/app/com.valvesoftware.Steam/.local/share/Steam/ubuntu12_32/steam",
            "/home/u/snap/steam/common/.local/share/Steam/ubuntu12_32/steam",
            "/snap/steam/200/usr/lib/steam/ubuntu12_32/steam",
        ] {
            assert_eq!(running_steam(&[native(exe)]), None, "{exe}");
        }
        // A program named like the client inside the Flatpak's scope is the Flatpak.
        let inside = ProcessInfo {
            comm: "steam".into(),
            cgroup: FLATPAK_CG.into(),
            exe: Some("/home/u/.local/share/Steam/ubuntu12_32/steam".into()),
        };
        assert_eq!(running_steam(&[inside]), Some(RunningSteam::Flatpak));
        // Without a systemd scope (no cgroup names) a process tells nothing by its cgroup.
        assert_eq!(running_steam(&[proc_in("0::/\n")]), None);
        // A game left in the Flatpak's scope after Steam closed is not Steam.
        let game = ProcessInfo { comm: "game.exe".into(), cgroup: FLATPAK_CG.into(), exe: None };
        assert_eq!(running_steam(&[game]), None);
        // cgroup v1 (hybrid): several lines, the scope on some.
        let v1 = "12:pids:/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-com.valvesoftware.Steam-77.scope\n\
                  1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-com.valvesoftware.Steam-77.scope\n\
                  0::/user.slice\n";
        assert_eq!(running_steam(&[proc_in(v1)]), Some(RunningSteam::Flatpak));
        // Other apps whose names start alike are not Steam.
        assert_eq!(running_steam(&[proc_in("0::/app.slice/app-flatpak-com.valvesoftware.SteamLink-5.scope\n")]), None);
        assert_eq!(running_steam(&[proc_in("0::/app.slice/snap.steam-tools.run-5.scope\n")]), None);
    }

    #[test]
    fn a_client_replaced_while_it_runs_is_still_seen() {
        let exe = without_deleted("/home/u/.local/share/Steam/ubuntu12_32/steam (deleted)".into());
        assert_eq!(running_steam(&[native(&exe.to_string_lossy())]), Some(RunningSteam::Native("/home/u/.local/share/Steam".into())));
    }

    #[test]
    fn each_kind_gets_its_own_launcher() {
        let flatpak_export = "/var/lib/flatpak/exports/bin/com.valvesoftware.Steam";
        let probe = FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam").with_path(flatpak_export);
        assert_eq!(launcher_for(&RunningSteam::Flatpak, &probe), Some(vec![flatpak_export.to_string()]));
        let probe = FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam").with_bin("flatpak", "/usr/bin/flatpak");
        assert_eq!(
            launcher_for(&RunningSteam::Flatpak, &probe),
            Some(vec!["/usr/bin/flatpak".into(), "run".into(), "com.valvesoftware.Steam".into()])
        );
        assert_eq!(launcher_for(&RunningSteam::Flatpak, &FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam")), None);

        let probe = FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam").with_path("/snap/bin/steam");
        assert_eq!(launcher_for(&RunningSteam::Snap, &probe), Some(vec!["/snap/bin/steam".into()]));
        assert_eq!(launcher_for(&RunningSteam::Snap, &FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam")), None);

        let native = RunningSteam::Native(PathBuf::from("/home/u/.local/share/Steam"));
        // The Snap's `steam` first on PATH is not the native one's launcher.
        let probe = FakeProbe::new("/home/u").with_bin("steam", "/snap/bin/steam").with_path("/usr/games/steam");
        assert_eq!(launcher_for(&native, &probe), Some(vec!["/usr/games/steam".into()]));
        let probe = FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam");
        assert_eq!(launcher_for(&native, &probe), Some(vec!["/usr/bin/steam".into()]));
        let probe = FakeProbe::new("/home/u").with_path("/home/u/.local/share/Steam/steam.sh");
        assert_eq!(launcher_for(&native, &probe), Some(vec!["/home/u/.local/share/Steam/steam.sh".into()]));
        // A launcher that cannot be run is none.
        let probe = FakeProbe::new("/home/u").with_non_executable("/home/u/.local/share/Steam/steam.sh");
        assert_eq!(launcher_for(&native, &probe), None);
    }

    #[test]
    fn without_a_sure_answer_the_usual_binary_is_used() {
        let export = "/var/lib/flatpak/exports/bin/com.valvesoftware.Steam";
        let probe = FakeProbe::new("/home/u").with_bin("steam", "/usr/bin/steam").with_path(export);
        let usual = vec!["/usr/bin/steam".to_string()];
        // The Flatpak runs: its launcher first, the usual one if it cannot start.
        assert_eq!(client_commands(&probe, &[proc_in(FLATPAK_CG)]), vec![vec![export.to_string()], usual.clone()]);
        // Nothing runs, two kinds run, or the running kind has no launcher: as before.
        assert_eq!(client_commands(&probe, &[]), vec![usual.clone()]);
        assert_eq!(
            client_commands(&probe, &[proc_in(FLATPAK_CG), native("/home/u/.local/share/Steam/ubuntu12_32/steam")]),
            vec![usual.clone()]
        );
        assert_eq!(client_commands(&probe, &[proc_in(SNAP_CG)]), vec![usual.clone()]);
        // The running native Steam's launcher is the usual one: once.
        assert_eq!(client_commands(&probe, &[native("/home/u/.local/share/Steam/ubuntu12_32/steam")]), vec![usual]);
        assert!(client_commands(&FakeProbe::new("/home/u"), &[]).is_empty());
    }
}
