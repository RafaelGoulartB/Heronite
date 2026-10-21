//! Installing and removing DXVK / DXVK-NVAPI / VKD3D in a Wine prefix (port of
//! `DXVK.installRemove` in `backend/tools/index.ts`): copy the release's DLLs
//! into the prefix, register the `native,builtin` overrides and remember the
//! version in `<prefix>/current_<tool>`.
//!
//! Only plain Wine prefixes are touched (a Proton prefix has no
//! `.update-timestamp`; Proton brings its own DXVK).

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use heronite_i18n::tr;

use crate::net::{Cancel, Net, CANCELLED};
use crate::probe::Probe;
use crate::settings::GameSettings;
use crate::tools::{self, Tool, Vulkan};

/// Runs `wine <parts>` inside the game's prefix and waits for it
/// (Heroic's `runWineCommand({ wait: true, protonVerb: 'run' })`).
pub trait WineRunner {
    fn run(&self, parts: &[String]) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Heroic's `'backup'`: install (or update) the tool in the prefix.
    Install,
    /// Heroic's `'restore'`: remove it and bring back Wine's own DLLs.
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Installed(String),
    AlreadyInstalled(String),
    Removed,
    /// Nothing to do here (Proton prefix, GPTK, prefix not created yet).
    Skipped(String),
}

pub struct Env<'a> {
    pub paths: &'a DataPaths,
    pub net: &'a dyn Net,
    pub probe: &'a dyn Probe,
    pub runner: &'a dyn WineRunner,
    pub vulkan: Vulkan,
    pub cancel: Cancel<'a>,
    /// Told what is about to take a while ("Downloading DXVK...").
    /// Called with the English text (logs) and the text in the interface language.
    pub progress: &'a dyn Fn(&str, &str),
}

fn dll_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_ascii_lowercase().ends_with(".dll"))
        .collect();
    v.sort();
    v
}

const OVERRIDES_KEY: &str = r"HKEY_CURRENT_USER\Software\Wine\DllOverrides";

fn override_name(dll: &str) -> String {
    dll.trim_end_matches(".dll").trim_end_matches(".DLL").to_string()
}

/// Override names of both DLL sets, each once (64-bit ones only in 64-bit prefixes).
fn override_names(is64: bool, dlls64: &[String], dlls32: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let sets: &[&[String]] = if is64 { &[dlls64, dlls32] } else { &[dlls32] };
    sets.iter().flat_map(|s| s.iter()).map(|d| override_name(d)).filter(|n| seen.insert(n.clone())).collect()
}

fn reg_add(name: &str) -> Vec<String> {
    ["reg", "add", OVERRIDES_KEY, "/v", name, "/d", "native,builtin", "/f"].iter().map(|s| s.to_string()).collect()
}

fn reg_delete(name: &str) -> Vec<String> {
    ["reg", "delete", OVERRIDES_KEY, "/v", name, "/f"].iter().map(|s| s.to_string()).collect()
}

/// Folder holding `nvngx.dll` (shipped by the NVIDIA driver), for DLSS.
pub fn nvngx_dir(probe: &dyn Probe) -> Option<String> {
    ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib32"]
        .iter()
        .map(|base| format!("{base}/nvidia/wine"))
        .find(|dir| probe.exists(&format!("{dir}/nvngx.dll")))
}

fn copy_dll(from: &Path, to: &Path) {
    if let Some(dir) = to.parent() {
        let _ = fs::create_dir_all(dir);
    }
    // A link left in the prefix is replaced, not written through: a program
    // in the sandbox could point it at any file of the user.
    if fs::symlink_metadata(to).is_ok_and(|m| m.file_type().is_symlink()) {
        let _ = fs::remove_file(to);
    }
    if let Err(e) = fs::copy(from, to) {
        log::error!("Error when copying {}: {e}", from.display());
    }
}

/// Whether `dir` (when it exists) is really inside `prefix`, links followed:
/// programs in the sandbox write the prefix, and this app then copies into it.
fn stays_inside(dir: &Path, prefix: &Path) -> bool {
    let (Ok(dir), Ok(prefix)) = (fs::canonicalize(dir), fs::canonicalize(prefix)) else { return true };
    dir.starts_with(prefix)
}

/// `DXVK.installRemove` for one tool in the prefix of `settings`.
pub fn install_remove(env: &Env, settings: &GameSettings, tool: Tool, action: Action) -> Result<Outcome, String> {
    if settings.wine_version.bin.contains("toolkit") {
        // It would break Apple's implementation.
        return Ok(Outcome::Skipped(tr!("Game Porting Toolkit prefix").into()));
    }
    let prefix = PathBuf::from(&settings.wine_prefix);
    if !prefix.join(".update-timestamp").exists() {
        // Proton prefix or not created yet; not an error, or the toggle would stick.
        return Ok(Outcome::Skipped(tr!("not a Wine prefix (yet)").into()));
    }
    let is64 = prefix.join("drive_c/windows/syswow64").exists();
    if !is64 {
        log::warn!("32-bit prefix detected!");
    }
    if tools::installed_version(env.paths, tool).is_none() {
        (env.progress)(&format!("Downloading {}...", tool.label()), &tr!("Downloading {tool}...", tool = tool.label()));
    }
    let release = tools::ensure(env.paths, env.net, tool, env.vulkan, env.cancel)?;
    let version = release.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let dir32 = release.join(tool.dir32());
    let dir64 = release.join("x64");
    let dlls32 = dll_names(&dir32);
    let dlls64 = dll_names(&dir64);
    let system32 = prefix.join("drive_c/windows/system32");
    let syswow64 = prefix.join("drive_c/windows/syswow64");
    if settings.sandbox.enabled && ![&system32, &syswow64].iter().all(|d| stays_inside(d, &prefix)) {
        return Err(tr!("The prefix's system folders lead outside it, so nothing was copied into them").into());
    }
    let marker = prefix.join(format!("current_{}", tool.name()));
    // Not through a link (see `copy_dll`).
    let current = fs::symlink_metadata(&marker)
        .ok()
        .filter(|m| m.is_file())
        .and_then(|_| fs::read_to_string(&marker).ok())
        .and_then(|t| t.lines().next().map(str::to_string))
        .unwrap_or_default();

    if action == Action::Restore {
        let _ = fs::remove_file(&marker);
        for name in override_names(is64, &dlls64, &dlls32) {
            env.runner.run(&reg_delete(&name));
        }
        let mut doomed: Vec<PathBuf> = Vec::new();
        if is64 {
            doomed.extend(dlls64.iter().map(|d| system32.join(d)));
            doomed.extend(dlls32.iter().map(|d| syswow64.join(d)));
        } else {
            doomed.extend(dlls32.iter().map(|d| system32.join(d)));
        }
        for f in doomed {
            if let Err(e) = fs::remove_file(&f) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::error!("Error removing {}: {e}", f.display());
                }
            }
        }
        // Restore Wine's stock libraries.
        env.runner.run(&["wineboot".to_string(), "-u".to_string()]);
        return Ok(Outcome::Removed);
    }

    if current == version {
        return Ok(Outcome::AlreadyInstalled(version));
    }
    // A cancel anywhere below leaves the marker unwritten: the next launch
    // installs again instead of trusting a half-done prefix.
    if (env.cancel)() {
        return Err(CANCELLED.to_string());
    }
    (env.progress)(
        &format!("Installing {} in the prefix...", tool.label()),
        &tr!("Installing {tool} in the prefix...", tool = tool.label()),
    );
    if is64 {
        for d in &dlls32 {
            copy_dll(&dir32.join(d), &syswow64.join(d));
        }
        for d in &dlls64 {
            copy_dll(&dir64.join(d), &system32.join(d));
        }
    } else {
        for d in &dlls32 {
            copy_dll(&dir32.join(d), &system32.join(d));
        }
    }
    for name in override_names(is64, &dlls64, &dlls32) {
        if !env.runner.run(&reg_add(&name)) && (env.cancel)() {
            return Err(CANCELLED.to_string());
        }
    }

    // nvngx.dll makes DLSS work on NVIDIA GPUs.
    if tool == Tool::DxvkNvapi {
        match nvngx_dir(env.probe) {
            Some(dir) => {
                for dll in ["nvngx.dll", "_nvngx.dll"] {
                    let from = Path::new(&dir).join(dll);
                    if from.exists() {
                        copy_dll(&from, &system32.join(dll));
                    }
                }
                env.runner.run(
                    &[
                        "reg",
                        "add",
                        r"HKEY_LOCAL_MACHINE\SOFTWARE\NVIDIA Corporation\Global\NGXCore",
                        "/v",
                        "FullPath",
                        "/d",
                        r"C:\windows\system32",
                        "/f",
                    ]
                    .map(String::from),
                );
            }
            None => log::warn!("Could not find nvngx.dll for DLSS!"),
        }
    }
    if (env.cancel)() {
        return Err(CANCELLED.to_string());
    }
    tools::write_atomic(&marker, &version)?;
    Ok(Outcome::Installed(version))
}

/// What flipping an "Auto Install/Update ... on Prefix" switch in the game
/// settings does to the prefix right away, following the conditions of Heroic's
/// `AutoDXVK`, `AutoDXVKNVAPI` and `AutoVKD3D` components (the switch itself is
/// saved either way; `None`: only the setting changes, the launch installs the
/// tool later if it applies).
///
/// * DXVK: only Wine builds (not Proton, CrossOver or Game Porting Toolkit,
///   not `-DXMT` builds);
/// * VKD3D: anything but Proton (CrossOver has no such switch at all);
/// * DXVK-NVAPI: anything but Proton (the switch is saved without installing);
/// * switching on installs ("backup"), switching off removes ("restore").
pub fn toggle_action(tool: Tool, wine: &crate::settings::WineVersion, enabled: bool) -> Option<Action> {
    let applies = match tool {
        Tool::Dxvk => wine.kind == "wine" && !wine.bin.contains("toolkit") && !wine.name.ends_with("-DXMT"),
        Tool::Vkd3d | Tool::DxvkNvapi => wine.kind != "proton" && wine.kind != "crossover",
    };
    applies.then_some(if enabled { Action::Install } else { Action::Restore })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::probe::FakeProbe;
    use crate::settings::WineVersion;
    use std::cell::RefCell;

    struct Recorder(RefCell<Vec<Vec<String>>>);
    impl WineRunner for Recorder {
        fn run(&self, parts: &[String]) -> bool {
            self.0.borrow_mut().push(parts.to_vec());
            true
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        paths: DataPaths,
        prefix: PathBuf,
        settings: GameSettings,
    }

    /// A Wine prefix and an installed `dxvk-1.0` with 32/64-bit DLLs.
    fn fixture(tool: Tool) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heroic"));
        let prefix = dir.path().join("pfx");
        for d in ["drive_c/windows/system32", "drive_c/windows/syswow64"] {
            fs::create_dir_all(prefix.join(d)).unwrap();
        }
        fs::write(prefix.join(".update-timestamp"), "1").unwrap();
        let release = tools::tool_dir(&paths, tool).join("rel-1");
        for (d, files) in [(tool.dir32(), ["a.dll", "b.dll", "readme.txt"]), ("x64", ["a.dll", "c.dll", "tests.exe"])] {
            fs::create_dir_all(release.join(d)).unwrap();
            for f in files {
                fs::write(release.join(d).join(f), format!("{d}/{f}")).unwrap();
            }
        }
        fs::write(tools::tool_dir(&paths, tool).join(format!("latest_{}", tool.name())), "rel-1\n").unwrap();
        let settings = GameSettings {
            wine_prefix: prefix.to_string_lossy().into_owned(),
            wine_version: WineVersion { bin: "/w/bin/wine".into(), name: "W".into(), kind: "wine".into(), ..Default::default() },
            ..Default::default()
        };
        Fixture { _dir: dir, paths, prefix, settings }
    }

    fn run(fx: &Fixture, tool: Tool, action: Action, rec: &Recorder, probe: &FakeProbe, net: &FakeNet) -> Result<Outcome, String> {
        let env = Env { paths: &fx.paths, net, probe, runner: rec, vulkan: Vulkan::Modern, cancel: &|| false, progress: &|_, _| {} };
        install_remove(&env, &fx.settings, tool, action)
    }

    #[test]
    fn install_copies_dlls_registers_overrides_and_remembers_the_version() {
        let fx = fixture(Tool::Dxvk);
        let rec = Recorder(RefCell::new(vec![]));
        let out = run(&fx, Tool::Dxvk, Action::Install, &rec, &FakeProbe::new("/h"), &FakeNet::offline()).unwrap();
        assert_eq!(out, Outcome::Installed("rel-1".into()));
        let sys32 = fx.prefix.join("drive_c/windows/system32");
        let wow = fx.prefix.join("drive_c/windows/syswow64");
        // 64-bit DLLs in system32, 32-bit ones in syswow64; non-DLL files stay out.
        assert_eq!(fs::read_to_string(sys32.join("c.dll")).unwrap(), "x64/c.dll");
        assert_eq!(fs::read_to_string(wow.join("b.dll")).unwrap(), "x32/b.dll");
        assert!(!sys32.join("tests.exe").exists() && !wow.join("readme.txt").exists());
        assert_eq!(fs::read_to_string(fx.prefix.join("current_dxvk")).unwrap(), "rel-1");
        let calls = rec.0.borrow();
        let names: Vec<&str> = calls.iter().map(|c| c[4].as_str()).collect();
        assert_eq!(names, ["a", "c", "b"], "each override once");
        assert_eq!(calls[0], ["reg", "add", OVERRIDES_KEY, "/v", "a", "/d", "native,builtin", "/f"]);
    }

    #[test]
    fn a_cancelled_install_is_not_remembered() {
        // The user cancels while the first override is being registered.
        struct Cancelling(std::cell::Cell<bool>);
        impl WineRunner for Cancelling {
            fn run(&self, _parts: &[String]) -> bool {
                self.0.set(true);
                false
            }
        }
        let fx = fixture(Tool::Dxvk);
        let runner = Cancelling(std::cell::Cell::new(false));
        let (probe, net) = (FakeProbe::new("/h"), FakeNet::offline());
        let cancel = || runner.0.get();
        let env = Env {
            paths: &fx.paths,
            net: &net,
            probe: &probe,
            runner: &runner,
            vulkan: Vulkan::Modern,
            cancel: &cancel,
            progress: &|_, _| {},
        };
        assert_eq!(install_remove(&env, &fx.settings, Tool::Dxvk, Action::Install), Err(CANCELLED.to_string()));
        assert!(!fx.prefix.join("current_dxvk").exists());
        // Cancelled before anything was copied: the prefix is untouched.
        let env = Env { cancel: &|| true, ..env };
        assert_eq!(install_remove(&env, &fx.settings, Tool::Dxvk, Action::Install), Err(CANCELLED.to_string()));
    }

    #[test]
    fn installing_twice_is_a_no_op() {
        let fx = fixture(Tool::Dxvk);
        let rec = Recorder(RefCell::new(vec![]));
        let (probe, net) = (FakeProbe::new("/h"), FakeNet::offline());
        run(&fx, Tool::Dxvk, Action::Install, &rec, &probe, &net).unwrap();
        let calls = rec.0.borrow().len();
        assert_eq!(run(&fx, Tool::Dxvk, Action::Install, &rec, &probe, &net).unwrap(), Outcome::AlreadyInstalled("rel-1".into()));
        assert_eq!(rec.0.borrow().len(), calls);
    }

    #[test]
    fn restore_removes_dlls_overrides_and_reboots_the_prefix() {
        let fx = fixture(Tool::Dxvk);
        let rec = Recorder(RefCell::new(vec![]));
        let (probe, net) = (FakeProbe::new("/h"), FakeNet::offline());
        run(&fx, Tool::Dxvk, Action::Install, &rec, &probe, &net).unwrap();
        rec.0.borrow_mut().clear();
        assert_eq!(run(&fx, Tool::Dxvk, Action::Restore, &rec, &probe, &net).unwrap(), Outcome::Removed);
        assert!(!fx.prefix.join("current_dxvk").exists());
        for f in ["system32/a.dll", "system32/c.dll", "syswow64/a.dll", "syswow64/b.dll"] {
            assert!(!fx.prefix.join("drive_c/windows").join(f).exists(), "{f}");
        }
        let calls = rec.0.borrow();
        assert_eq!(calls.first().unwrap()[1], "delete");
        assert_eq!(calls.last().unwrap(), &["wineboot", "-u"]);
    }

    #[test]
    fn a_32_bit_prefix_gets_everything_in_system32() {
        let fx = fixture(Tool::Dxvk);
        fs::remove_dir_all(fx.prefix.join("drive_c/windows/syswow64")).unwrap();
        let rec = Recorder(RefCell::new(vec![]));
        run(&fx, Tool::Dxvk, Action::Install, &rec, &FakeProbe::new("/h"), &FakeNet::offline()).unwrap();
        assert!(fx.prefix.join("drive_c/windows/system32/b.dll").exists());
        assert!(!fx.prefix.join("drive_c/windows/system32/c.dll").exists());
    }

    #[test]
    fn proton_toolkit_and_missing_prefixes_are_skipped() {
        let mut fx = fixture(Tool::Dxvk);
        let rec = Recorder(RefCell::new(vec![]));
        let (probe, net) = (FakeProbe::new("/h"), FakeNet::offline());
        fs::remove_file(fx.prefix.join(".update-timestamp")).unwrap();
        assert!(matches!(run(&fx, Tool::Dxvk, Action::Install, &rec, &probe, &net).unwrap(), Outcome::Skipped(_)));
        fx.settings.wine_version.bin = "/gptk/toolkit/bin/wine".into();
        assert!(matches!(run(&fx, Tool::Dxvk, Action::Install, &rec, &probe, &net).unwrap(), Outcome::Skipped(_)));
        assert!(rec.0.borrow().is_empty());
        assert!(!fx.prefix.join("current_dxvk").exists());
    }

    #[test]
    fn vkd3d_uses_its_x86_folder_and_a_missing_tool_offline_is_an_error() {
        let fx = fixture(Tool::Vkd3d);
        let rec = Recorder(RefCell::new(vec![]));
        let probe = FakeProbe::new("/h");
        run(&fx, Tool::Vkd3d, Action::Install, &rec, &probe, &FakeNet::offline()).unwrap();
        assert_eq!(fs::read_to_string(fx.prefix.join("drive_c/windows/syswow64/b.dll")).unwrap(), "x86/b.dll");
        // DXVK is not installed in this fixture.
        let err = run(&fx, Tool::Dxvk, Action::Install, &rec, &probe, &FakeNet::offline()).unwrap_err();
        assert!(err.contains("DXVK"), "{err}");
    }

    #[test]
    fn nvapi_also_copies_nvngx_when_the_driver_ships_it() {
        let fx = fixture(Tool::DxvkNvapi);
        let rec = Recorder(RefCell::new(vec![]));
        assert_eq!(nvngx_dir(&FakeProbe::new("/h")), None);
        assert_eq!(nvngx_dir(&FakeProbe::new("/h").with_path("/usr/lib/nvidia/wine/nvngx.dll")).as_deref(), Some("/usr/lib/nvidia/wine"));
        run(&fx, Tool::DxvkNvapi, Action::Install, &rec, &FakeProbe::new("/h"), &FakeNet::offline()).unwrap();
        assert!(rec.0.borrow().iter().all(|c| c[2] != r"HKEY_LOCAL_MACHINE\SOFTWARE\NVIDIA Corporation\Global\NGXCore"));
    }

    fn wine_of(kind: &str, bin: &str, name: &str) -> crate::settings::WineVersion {
        WineVersion { kind: kind.into(), bin: bin.into(), name: name.into(), ..Default::default() }
    }

    #[test]
    fn toggles_install_and_remove_following_heroics_conditions() {
        let wine = wine_of("wine", "/w/bin/wine", "Wine-GE");
        let proton = wine_of("proton", "/p/proton", "GE-Proton");
        let crossover = wine_of("crossover", "/c/wine", "CrossOver");
        let gptk = wine_of("wine", "/gptk/toolkit/bin/wine", "GPTK");
        let dxmt = wine_of("wine", "/w/bin/wine", "Wine Staging-DXMT");
        // Switching on installs, switching off removes.
        for tool in Tool::ALL {
            assert_eq!(toggle_action(tool, &wine, true), Some(Action::Install), "{tool:?}");
            assert_eq!(toggle_action(tool, &wine, false), Some(Action::Restore), "{tool:?}");
        }
        // Proton brings its own DXVK/VKD3D/NVAPI: the setting is saved, nothing is touched.
        for tool in Tool::ALL {
            assert_eq!(toggle_action(tool, &proton, true), None, "{tool:?}");
            assert_eq!(toggle_action(tool, &crossover, true), None, "{tool:?}");
        }
        // DXVK is not offered for the toolkit and DXMT builds.
        assert_eq!(toggle_action(Tool::Dxvk, &gptk, true), None);
        assert_eq!(toggle_action(Tool::Dxvk, &dxmt, false), None);
        // VKD3D and NVAPI are (the backend then skips the toolkit prefix itself).
        assert_eq!(toggle_action(Tool::Vkd3d, &gptk, true), Some(Action::Install));
        assert_eq!(toggle_action(Tool::DxvkNvapi, &dxmt, true), Some(Action::Install));
    }
}
