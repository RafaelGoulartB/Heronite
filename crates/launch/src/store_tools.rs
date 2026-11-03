//! The command-line tools behind the stores: `gogdl` (GOG downloads, cloud
//! saves) and `nile` (Amazon Games login, library and downloads). Epic needs
//! none (`heronite_epic_api`). Each is found in the settings, the data
//! folder or `PATH`, and downloaded into `<data>/tools/bin` (`nile.exe` there on Windows)
//! from its GitHub release when none is there. The releases are the ones
//! Heroic pins (`RELEASE_TAGS` in its `meta/downloadHelperBinaries.ts`);
//! their Windows builds are standalone programs (PyInstaller), no Python
//! needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use heronite_data::DataPaths;

use crate::net::{Cancel, Net, CANCELLED};
use crate::plan::PlanContext;
use crate::probe::Probe;

/// One downloadable tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreTool {
    /// Binary name in `<data>/tools/bin` (plus `.exe` on Windows) and on `PATH`.
    pub name: &'static str,
    /// Variable that overrides everything (tests, fake tools).
    pub env: &'static str,
    /// Key of the global settings (`config.json` `defaultSettings`) holding a
    /// user-picked binary.
    pub setting: &'static str,
    repo: &'static str,
    version: &'static str,
    /// Release assets: (system, CPU, file), as `std::env::consts` names them.
    assets: &'static [(&'static str, &'static str, &'static str)],
}

/// GOG's downloader (on Linux a Python zipapp: it needs `python3`).
pub const GOGDL: StoreTool = StoreTool {
    name: "gogdl",
    env: "HERONITE_GOGDL_BIN",
    setting: "altGogdlBin",
    repo: "Heroic-Games-Launcher/heroic-gogdl",
    version: "v1.3.0",
    assets: &[
        ("linux", "x86_64", "gogdl_linux_x86_64"),
        ("linux", "aarch64", "gogdl_linux_arm64"),
        ("windows", "x86_64", "gogdl_windows_x86_64.exe"),
        ("windows", "aarch64", "gogdl_windows_arm64.exe"),
    ],
};

/// The Amazon Games client.
pub const NILE: StoreTool = StoreTool {
    name: "nile",
    env: "HERONITE_NILE_BIN",
    setting: "altNileBin",
    repo: "imLinguin/nile",
    version: "v1.2.0",
    assets: &[
        ("linux", "x86_64", "nile_linux_x86_64"),
        ("linux", "aarch64", "nile_linux_arm64"),
        ("windows", "x86_64", "nile_windows_x86_64.exe"),
        // No Arm build: Windows on Arm runs x64 programs.
        ("windows", "aarch64", "nile_windows_x86_64.exe"),
    ],
};

/// The file only NixOS has.
const NIXOS_MARKER: &str = "/etc/NIXOS";

/// Why `bin` cannot start here, when what starts it is missing: the Linux
/// builds of gogdl are Python zipapps (`#!/usr/bin/env
/// python3`), nile's a program for the usual Linux loader
/// (`/lib64/ld-linux-x86-64.so.2`), which NixOS has only with nix-ld.
/// `None` when it has what it needs, or is not a script or such a program.
pub(crate) fn missing_runtime(name: &str, bin: &str, probe: &dyn Probe) -> Option<String> {
    let head = probe.read_head(bin, 4096)?;
    let nixos = probe.exists(NIXOS_MARKER);
    if let Some(line) = head.strip_prefix(b"#!") {
        let line = String::from_utf8_lossy(line.split(|&b| b == b'\n').next().unwrap_or_default()).into_owned();
        let mut words = line.split_whitespace();
        let first = words.next()?;
        let program = if first.ends_with("/env") { words.find(|w| !w.starts_with('-'))? } else { first };
        let found = if program.contains('/') { probe.exists(program) } else { probe.which(program).is_some() };
        if found {
            return None;
        }
        let python = program.rsplit('/').next().unwrap_or(program).starts_with("python");
        let needs = if python { format!("{program} (Python 3)") } else { program.to_string() };
        let fix = if nixos {
            format!("add python3 to your NixOS packages, or install {name} from nixpkgs")
        } else {
            format!("install Python 3 from your system's packages, or {name} itself")
        };
        return Some(format!("{name} needs {needs}, which is not installed: {fix}"));
    }
    if head.starts_with(b"\x7fELF") {
        let interp = elf_interpreter(&head)?;
        if probe.exists(&interp) {
            return None;
        }
        let fix = if nixos {
            format!("enable programs.nix-ld in your NixOS configuration, or install {name} from nixpkgs")
        } else {
            format!("install {name} from your system's packages")
        };
        return Some(format!("{name} is a program for the usual Linux systems and needs {interp}, which this system does not have: {fix}"));
    }
    None
}

/// The loader a dynamic Linux program names (its `PT_INTERP`, near the
/// start of the file): the `/...ld-linux...` path in `head`.
fn elf_interpreter(head: &[u8]) -> Option<String> {
    let at = head.windows(8).position(|w| w == b"ld-linux")?;
    let start = head[..at].iter().rposition(|&b| b == 0).map_or(0, |i| i + 1);
    let end = head[at..].iter().position(|&b| b == 0).map_or(head.len(), |i| at + i);
    let path = std::str::from_utf8(&head[start..end]).ok()?;
    path.starts_with('/').then(|| path.to_string())
}

/// One download of each tool at a time per process: a login and a refresh
/// can both ask.
fn busy(tool: &StoreTool) -> &'static Mutex<()> {
    static BUSY: [Mutex<()>; 2] = [Mutex::new(()), Mutex::new(())];
    let i = [GOGDL.name, NILE.name].iter().position(|n| *n == tool.name).unwrap_or(0);
    &BUSY[i]
}

impl StoreTool {
    pub const fn version(&self) -> &'static str {
        self.version
    }

    /// Where a downloaded copy lives: `<data>/tools/bin/<name>` (`.exe` on
    /// Windows).
    pub fn data_bin(&self, paths: &DataPaths) -> PathBuf {
        paths.tools_dir().join("bin").join(format!("{}{}", self.name, std::env::consts::EXE_SUFFIX))
    }

    /// Release asset for this system and CPU, `None` where the tool has no
    /// build.
    pub fn asset_name(&self) -> Option<&'static str> {
        self.asset_for(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// Release asset for a system and CPU (`std::env::consts` names).
    pub fn asset_for(&self, os: &str, arch: &str) -> Option<&'static str> {
        self.assets.iter().find(|(o, a, _)| *o == os && *a == arch).map(|(_, _, file)| *file)
    }

    pub fn download_url(&self, asset: &str) -> String {
        format!("https://github.com/{}/releases/download/{}/{asset}", self.repo, self.version)
    }

    /// Where the binary may live, in priority order (`HERONITE_*_BIN`
    /// overrides all, then the settings, then the data folder's copy).
    pub fn candidates(&self, ctx: &PlanContext, probe: &dyn Probe) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(p) = probe.env_var(self.env).filter(|s| !s.is_empty()) {
            out.push(p);
        }
        if let Some(p) = ctx.global.string(self.setting) {
            out.push(p);
        }
        out.push(self.data_bin(&ctx.paths).to_string_lossy().into_owned());
        out
    }

    /// First existing candidate, else the tool on `PATH` (`<name>.exe` on
    /// Windows). On NixOS the one on `PATH` comes before the data folder's
    /// copy: a tool from nixpkgs runs as is, the downloaded build only with
    /// what NixOS leaves out ([`missing_runtime`]).
    pub fn find(&self, ctx: &PlanContext, probe: &dyn Probe) -> Option<String> {
        let mut candidates = self.candidates(ctx, probe);
        if probe.exists(NIXOS_MARKER) {
            if let Some(on_path) = probe.which(self.name) {
                candidates.insert(candidates.len() - 1, on_path);
            }
        }
        candidates.into_iter().find(|p| probe.exists(p)).or_else(|| probe.which(self.name))
    }

    /// The binary to run: an existing one, else a fresh download into the
    /// data folder. Blocking, network.
    pub fn ensure(&self, paths: &DataPaths, probe: &dyn Probe, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
        let _guard = busy(self).lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = self.find(&PlanContext::load(paths), probe) {
            if let Some(why) = missing_runtime(self.name, &found, probe) {
                return Err(why);
            }
            return Ok(PathBuf::from(found));
        }
        let asset = self
            .asset_name()
            .ok_or_else(|| format!("{} has no build for {} {}", self.name, std::env::consts::OS, std::env::consts::ARCH))?;
        if !net.is_online() {
            return Err(format!("{} is not installed and there is no internet connection", self.name));
        }
        let target = self.data_bin(paths);
        install_binary(self.name, &self.download_url(asset), &target, net, cancel)?;
        log::info!("{} {} downloaded to {}", self.name, self.version, target.display());
        Ok(target)
    }
}

/// Where a download waits until it proved it runs: `<name>.part` next to
/// `target` (`<name>.part.exe` on Windows, which only starts `.exe` files
/// as programs).
fn part_file(target: &Path) -> PathBuf {
    target.with_extension(format!("part{}", std::env::consts::EXE_SUFFIX))
}

/// Downloads `name` next to `target`, checks that it runs (`--version`), then
/// renames it into place, so an interrupted or broken download never becomes
/// the binary.
fn install_binary(name: &str, url: &str, target: &Path, net: &dyn Net, cancel: Cancel) -> Result<(), String> {
    let dir = target.parent().ok_or_else(|| format!("{name} has no folder"))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let part = part_file(target);
    let _ = fs::remove_file(&part);
    let fail = |msg: String| {
        let _ = fs::remove_file(&part);
        msg
    };
    net.download(url, &part, cancel).map_err(|e| fail(if e == CANCELLED { e } else { format!("downloading {name} failed: {e}") }))?;
    heronite_platform::fs::make_executable(&part).map_err(|e| fail(format!("making {name} executable: {e}")))?;
    if !runs_version(&part) {
        let why = missing_runtime(name, &part.to_string_lossy(), &crate::probe::SystemProbe);
        return Err(fail(why.unwrap_or_else(|| format!("the downloaded {name} does not run"))));
    }
    heronite_platform::fs::rename_replacing(&part, target).map_err(|e| fail(format!("installing {name}: {e}")))
}

/// Whether `bin --version` exits well. It runs as a helper: no console
/// window on Windows, and a file just written that is still "busy"
/// (`ETXTBSY`) is tried again (`heronite_data::proc::spawn`).
fn runs_version(bin: &Path) -> bool {
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    // The first run of a Windows build unpacks itself: give it time.
    let run = heronite_data::proc::Run::new().timeout(Duration::from_secs(120)).grace(Duration::from_secs(2));
    run.run(&mut cmd).is_ok_and(|out| out.success())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::probe::FakeProbe;

    /// A program of this system that runs `--version` well, or a file that
    /// is no program.
    pub(crate) fn fake_bin(ok: bool) -> Vec<u8> {
        if ok {
            std::fs::read(heronite_data::testing::fake_tool_program()).unwrap()
        } else {
            b"not a program\n".to_vec()
        }
    }

    #[test]
    fn downloads_into_the_data_folder_when_none_is_found() {
        for tool in [GOGDL, NILE] {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::at(dir.path().join("heronite"));
            let url = tool.download_url(tool.asset_name().unwrap());
            let net = FakeNet::online().with_file(&url, fake_bin(true));
            let got = tool.ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap();
            assert_eq!(got, tool.data_bin(&paths));
            assert!(got.exists());
            assert!(!part_file(&got).exists());

            let probe = FakeProbe::default().with_path(got.to_str().unwrap());
            tool.ensure(&paths, &probe, &net, &|| false).unwrap();
            assert_eq!(net.requests().len(), 1, "{}", tool.name);
        }
    }

    #[test]
    fn each_system_gets_its_own_build() {
        let url = |tool: StoreTool, os, arch| tool.asset_for(os, arch).map(|a| tool.download_url(a));
        assert_eq!(
            url(GOGDL, "windows", "x86_64").as_deref(),
            Some("https://github.com/Heroic-Games-Launcher/heroic-gogdl/releases/download/v1.3.0/gogdl_windows_x86_64.exe")
        );
        assert_eq!(
            url(NILE, "windows", "x86_64").as_deref(),
            Some("https://github.com/imLinguin/nile/releases/download/v1.2.0/nile_windows_x86_64.exe")
        );
        assert_eq!(url(NILE, "windows", "aarch64"), url(NILE, "windows", "x86_64"), "x64 under emulation");
        assert_eq!(GOGDL.asset_for("linux", "aarch64"), Some("gogdl_linux_arm64"));
        assert_eq!(NILE.asset_for("macos", "aarch64"), None);
        for tool in [GOGDL, NILE] {
            assert!(tool.asset_name().is_some(), "{} has a build for this system", tool.name);
        }
    }

    #[test]
    fn the_data_folder_copy_is_an_exe_on_windows() {
        let paths = DataPaths::at("/hr");
        let bin = GOGDL.data_bin(&paths);
        assert_eq!(bin.file_name().unwrap(), if cfg!(windows) { "gogdl.exe" } else { "gogdl" });
    }

    #[test]
    fn the_override_and_path_come_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let ctx = PlanContext::load(&paths);
        let probe = FakeProbe::default().with_env("HERONITE_NILE_BIN", "/opt/nile").with_path("/opt/nile");
        assert_eq!(NILE.find(&ctx, &probe).as_deref(), Some("/opt/nile"));
        let probe = FakeProbe::default().with_bin("gogdl", "/usr/bin/gogdl");
        assert_eq!(GOGDL.find(&ctx, &probe).as_deref(), Some("/usr/bin/gogdl"));
        assert_eq!(GOGDL.find(&ctx, &FakeProbe::default()), None);
    }

    #[test]
    fn nixos_takes_the_tool_on_path_before_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let ctx = PlanContext::load(&paths);
        let data = GOGDL.data_bin(&paths).to_string_lossy().into_owned();
        let nix = "/run/current-system/sw/bin/gogdl";
        let probe = FakeProbe::default().with_path(&data).with_bin("gogdl", nix);
        assert_eq!(GOGDL.find(&ctx, &probe).as_deref(), Some(data.as_str()));
        let probe = probe.with_path(NIXOS_MARKER);
        assert_eq!(GOGDL.find(&ctx, &probe).as_deref(), Some(nix));
        // The setting still wins.
        let probe = probe.with_env("HERONITE_GOGDL_BIN", "/opt/gogdl").with_path("/opt/gogdl");
        assert_eq!(GOGDL.find(&ctx, &probe).as_deref(), Some("/opt/gogdl"));
    }

    #[test]
    fn what_a_build_needs_to_start_is_checked() {
        let zipapp = b"#!/usr/bin/env python3\nPK\x03\x04...";
        let probe = FakeProbe::default().with_file("/t/gogdl", zipapp);
        let why = missing_runtime("gogdl", "/t/gogdl", &probe).unwrap();
        assert!(why.contains("gogdl needs python3 (Python 3)"), "{why}");
        let probe = probe.with_bin("python3", "/usr/bin/python3");
        assert_eq!(missing_runtime("gogdl", "/t/gogdl", &probe), None);

        let mut elf = b"\x7fELF\x02\x01\x01\0".to_vec();
        elf.extend_from_slice(&[0; 56]);
        elf.extend_from_slice(b"/lib64/ld-linux-x86-64.so.2\0more");
        let probe = FakeProbe::default().with_file("/t/nile", &elf).with_path(NIXOS_MARKER);
        let why = missing_runtime("nile", "/t/nile", &probe).unwrap();
        assert!(why.contains("/lib64/ld-linux-x86-64.so.2") && why.contains("nix-ld"), "{why}");
        let probe = probe.with_path("/lib64/ld-linux-x86-64.so.2");
        assert_eq!(missing_runtime("nile", "/t/nile", &probe), None);

        // Not a script nor a program for a loader: nothing to say.
        let probe = FakeProbe::default().with_file("/t/x", b"MZ\x90\0");
        assert_eq!(missing_runtime("x", "/t/x", &probe), None);
        assert_eq!(missing_runtime("x", "/t/missing", &probe), None);
    }

    #[test]
    fn a_found_tool_that_cannot_start_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let data = GOGDL.data_bin(&paths).to_string_lossy().into_owned();
        let probe = FakeProbe::default().with_file(&data, b"#!/usr/bin/env python3\n");
        let err = GOGDL.ensure(&paths, &probe, &FakeNet::offline(), &|| false).unwrap_err();
        assert!(err.contains("gogdl needs python3"), "{err}");
    }

    #[test]
    fn a_broken_download_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let url = GOGDL.download_url(GOGDL.asset_name().unwrap());
        let net = FakeNet::online().with_file(&url, fake_bin(false));
        let err = GOGDL.ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap_err();
        assert!(err.contains("gogdl does not run"), "{err}");
        assert!(!GOGDL.data_bin(&paths).exists());
        assert!(FakeNet::offline().requests().is_empty());
        let err = NILE.ensure(&paths, &FakeProbe::default(), &FakeNet::offline(), &|| false).unwrap_err();
        assert!(err.contains("no internet"), "{err}");
    }

    /// The real releases, downloaded and run (`--version`) on this system:
    /// `cargo test -p heronite-launch --lib real_releases -- --ignored`.
    #[test]
    #[ignore = "downloads about 30 MB"]
    fn real_releases_install_and_run_here() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        for tool in [GOGDL, NILE] {
            let got = tool.ensure(&paths, &FakeProbe::default(), &crate::net::SystemNet, &|| false).unwrap();
            assert_eq!(got, tool.data_bin(&paths));
            let out = heronite_data::proc::output(std::process::Command::new(&got).arg("--version")).unwrap();
            assert!(out.success(), "{}: {}", tool.name, out.stderr_text());
            let version = tool.version().trim_start_matches('v');
            assert!(out.stdout_text().contains(version), "{}: {}", tool.name, out.stdout_text());
        }
    }
}
