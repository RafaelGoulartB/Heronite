//! The command-line tools behind the GOG and Amazon Games stores: `gogdl`
//! (GOG downloads, cloud saves) and `nile` (Amazon Games login, library and
//! downloads). Like legendary for Epic, each is found in the settings, the
//! data folder or `$PATH`, and downloaded into `<data>/tools/bin` from its
//! GitHub release when none is there.

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
    /// Binary name in `<data>/tools/bin` and on `$PATH`.
    pub name: &'static str,
    /// Variable that overrides everything (tests, fake tools).
    pub env: &'static str,
    /// Key of the global settings (`config.json` `defaultSettings`) holding a
    /// user-picked binary.
    pub setting: &'static str,
    repo: &'static str,
    version: &'static str,
    asset_x86_64: &'static str,
    asset_aarch64: Option<&'static str>,
}

/// GOG's downloader (a Python zipapp: it needs `python3`).
pub const GOGDL: StoreTool = StoreTool {
    name: "gogdl",
    env: "HERONITE_GOGDL_BIN",
    setting: "altGogdlBin",
    repo: "Heroic-Games-Launcher/heroic-gogdl",
    version: "v1.3.0",
    asset_x86_64: "gogdl_linux_x86_64",
    asset_aarch64: Some("gogdl_linux_arm64"),
};

/// The Amazon Games client.
pub const NILE: StoreTool = StoreTool {
    name: "nile",
    env: "HERONITE_NILE_BIN",
    setting: "altNileBin",
    repo: "imLinguin/nile",
    version: "v1.2.0",
    asset_x86_64: "nile_linux_x86_64",
    asset_aarch64: Some("nile_linux_arm64"),
};

/// One download at a time per process: a login and a refresh can both ask.
static BUSY: Mutex<()> = Mutex::new(());

impl StoreTool {
    pub fn version(&self) -> &'static str {
        self.version
    }

    /// Where a downloaded copy lives.
    pub fn data_bin(&self, paths: &DataPaths) -> PathBuf {
        paths.tools_dir().join("bin").join(self.name)
    }

    /// Release asset for this CPU, `None` where the tool has no Linux build.
    pub fn asset_name(&self) -> Option<&'static str> {
        match std::env::consts::ARCH {
            "x86_64" => Some(self.asset_x86_64),
            "aarch64" => self.asset_aarch64,
            _ => None,
        }
    }

    pub fn download_url(&self, asset: &str) -> String {
        format!("https://github.com/{}/releases/download/{}/{asset}", self.repo, self.version)
    }

    /// Where the binary may live, in priority order.
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

    /// First existing candidate, else the tool on `$PATH`.
    pub fn find(&self, ctx: &PlanContext, probe: &dyn Probe) -> Option<String> {
        self.candidates(ctx, probe).into_iter().find(|p| probe.exists(p)).or_else(|| probe.which(self.name))
    }

    /// The binary to run: an existing one, else a fresh download into the
    /// data folder. Blocking, network.
    pub fn ensure(&self, paths: &DataPaths, probe: &dyn Probe, net: &dyn Net, cancel: Cancel) -> Result<PathBuf, String> {
        let _guard = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = self.find(&PlanContext::load(paths), probe) {
            return Ok(PathBuf::from(found));
        }
        let asset = self.asset_name().ok_or_else(|| format!("{} has no build for {}", self.name, std::env::consts::ARCH))?;
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
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::probe::FakeProbe;

    fn fake_bin(ok: bool) -> Vec<u8> {
        format!("#!/bin/sh\nexit {}\n", if ok { 0 } else { 1 }).into_bytes()
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
            assert!(!got.with_extension("part").exists());

            let probe = FakeProbe::default().with_path(got.to_str().unwrap());
            tool.ensure(&paths, &probe, &net, &|| false).unwrap();
            assert_eq!(net.requests().len(), 1, "{}", tool.name);
        }
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
