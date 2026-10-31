//! The command-line tools behind the stores: `legendary` (Epic), `gogdl`
//! (GOG downloads, cloud saves) and `nile` (Amazon Games login, library and
//! downloads). Each is found in the settings, the data folder or `PATH`, and
//! downloaded into `<data>/tools/bin` (`legendary.exe` there on Windows)
//! from its GitHub release when none is there. The releases are the ones
//! Heroic pins (`RELEASE_TAGS` in its `meta/downloadHelperBinaries.ts`);
//! their Windows builds are standalone programs (PyInstaller), no Python
//! needed.

use std::path::PathBuf;
use std::sync::Mutex;

use heronite_data::DataPaths;

use crate::legendary_install::install_binary;
use crate::net::{Cancel, Net};
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

/// Epic's client.
pub const LEGENDARY: StoreTool = StoreTool {
    name: "legendary",
    env: "HERONITE_LEGENDARY_BIN",
    setting: "altLegendaryBin",
    repo: "legendary-gl/legendary",
    version: "0.21.1",
    assets: &[
        ("linux", "x86_64", "legendary_linux_x64"),
        ("linux", "aarch64", "legendary_linux_arm64"),
        ("windows", "x86_64", "legendary_windows_x64.exe"),
        ("windows", "aarch64", "legendary_windows_arm64.exe"),
    ],
};

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

/// One download of each tool at a time per process: a login and a refresh
/// can both ask.
fn busy(tool: &StoreTool) -> &'static Mutex<()> {
    static BUSY: [Mutex<()>; 3] = [Mutex::new(()), Mutex::new(()), Mutex::new(())];
    let i = [LEGENDARY.name, GOGDL.name, NILE.name].iter().position(|n| *n == tool.name).unwrap_or(0);
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
    /// Windows).
    pub fn find(&self, ctx: &PlanContext, probe: &dyn Probe) -> Option<String> {
        self.candidates(ctx, probe).into_iter().find(|p| probe.exists(p)).or_else(|| probe.which(self.name))
    }

    /// The binary to run: an existing one, else a fresh download into the
    /// data folder. Blocking, network.
    pub fn ensure(&self, paths: &DataPaths, probe: &dyn Probe, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
        let _guard = busy(self).lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = self.find(&PlanContext::load(paths), probe) {
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
        for tool in [LEGENDARY, GOGDL, NILE] {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::at(dir.path().join("heronite"));
            let url = tool.download_url(tool.asset_name().unwrap());
            let net = FakeNet::online().with_file(&url, fake_bin(true));
            let got = tool.ensure(&paths, &FakeProbe::default(), &net, &|| false).unwrap();
            assert_eq!(got, tool.data_bin(&paths));
            assert!(got.exists());
            assert!(!crate::legendary_install::part_file(&got).exists());

            let probe = FakeProbe::default().with_path(got.to_str().unwrap());
            tool.ensure(&paths, &probe, &net, &|| false).unwrap();
            assert_eq!(net.requests().len(), 1, "{}", tool.name);
        }
    }

    #[test]
    fn each_system_gets_its_own_build() {
        let url = |tool: StoreTool, os, arch| tool.asset_for(os, arch).map(|a| tool.download_url(a));
        assert_eq!(
            url(LEGENDARY, "windows", "x86_64").as_deref(),
            Some("https://github.com/legendary-gl/legendary/releases/download/0.21.1/legendary_windows_x64.exe")
        );
        assert_eq!(
            url(GOGDL, "windows", "x86_64").as_deref(),
            Some("https://github.com/Heroic-Games-Launcher/heroic-gogdl/releases/download/v1.3.0/gogdl_windows_x86_64.exe")
        );
        assert_eq!(
            url(NILE, "windows", "x86_64").as_deref(),
            Some("https://github.com/imLinguin/nile/releases/download/v1.2.0/nile_windows_x86_64.exe")
        );
        assert_eq!(url(NILE, "windows", "aarch64"), url(NILE, "windows", "x86_64"), "x64 under emulation");
        assert_eq!(LEGENDARY.asset_for("linux", "x86_64"), Some("legendary_linux_x64"));
        assert_eq!(GOGDL.asset_for("linux", "aarch64"), Some("gogdl_linux_arm64"));
        assert_eq!(NILE.asset_for("macos", "aarch64"), None);
        for tool in [LEGENDARY, GOGDL, NILE] {
            assert!(tool.asset_name().is_some(), "{} has a build for this system", tool.name);
        }
    }

    #[test]
    fn the_data_folder_copy_is_an_exe_on_windows() {
        let paths = DataPaths::at("/hr");
        let bin = GOGDL.data_bin(&paths);
        assert_eq!(bin.file_name().unwrap(), if cfg!(windows) { "gogdl.exe" } else { "gogdl" });
        assert_eq!(LEGENDARY.data_bin(&paths), paths.legendary_bin());
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
}
