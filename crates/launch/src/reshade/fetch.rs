//! Downloads and their cache in `<data>/tools/reshade/`. Nothing of
//! ReShade ships with Heronite: the DLLs, shaders and lists come from their
//! official sources when the user installs, over HTTPS.
//!
//! ```text
//! latest.json                     newest version, asked at most daily
//! catalog/EffectPackages.ini      refreshed at most daily
//! catalog/Compatibility.ini
//! <version>/ReShade{32,64}.dll       the standard build
//! <version>-addon/ReShade{32,64}.dll the build with add-on support
//! packages/<sha256 of the url>.zip
//! d3dcompiler_47/{x86,x64}/d3dcompiler_47.dll
//! ```
//! Offline, whatever is cached is used.

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use serde_json::json;

use super::archive;
use super::catalog::{COMPATIBILITY_URL, PACKAGES_URL};
use super::detect::{is_reshade, Arch};
use crate::diagnose::pe;
use crate::gfxmods::download::{download, fresh, https};
pub use crate::gfxmods::hash::{sha256, sha256_file};
use crate::net::{Cancel, Net};

const TAGS_URL: &str = "https://api.github.com/repos/crosire/reshade/tags";
/// The standard build, the default: games with anti-cheat block the add-on
/// one more often.
pub(super) fn setup_url(version: &str) -> String {
    format!("https://reshade.me/downloads/ReShade_Setup_{version}.exe")
}

/// The build with full add-on support, which add-ons such as RenoDX need.
pub(crate) fn addon_setup_url(version: &str) -> String {
    format!("https://reshade.me/downloads/ReShade_Setup_{version}_Addon.exe")
}

/// Which ReShade build a DLL is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Standard,
    /// Full add-on support.
    Addon,
}

impl Build {
    pub fn of(addon: bool) -> Self {
        if addon {
            Build::Addon
        } else {
            Build::Standard
        }
    }

    fn folder(self, version: &str) -> String {
        match self {
            Build::Standard => version.to_string(),
            Build::Addon => format!("{version}-addon"),
        }
    }

    fn setup_url(self, version: &str) -> String {
        match self {
            Build::Standard => setup_url(version),
            Build::Addon => addon_setup_url(version),
        }
    }
}

/// `d3dcompiler_47` from mozilla/fxc2 (Microsoft's redistributable DLL),
/// pinned to a commit and checked. Wine's own compiler fails on some
/// shaders; Windows already has it.
const FXC2_COMMIT: &str = "807d26f4e4e9e9d0d0d6c1e05493b28eafb20e91";
fn compiler_source(arch: Arch) -> (String, &'static str) {
    let (file, sha) = match arch {
        Arch::X64 => ("d3dcompiler_47.dll", "4432bbd1a390874f3f0a503d45cc48d346abc3a8c0213c289f4b615bf0ee84f3"),
        Arch::X86 => ("d3dcompiler_47_32.dll", "2ad0d4987fc4624566b190e747c9d95038443956ed816abfd1e2d389b5ec0851"),
    };
    (format!("https://raw.githubusercontent.com/mozilla/fxc2/{FXC2_COMMIT}/dll/{file}"), sha)
}

/// Whether `sha` is the checked `d3dcompiler_47` of either architecture.
pub fn is_known_compiler(sha: &str) -> bool {
    [Arch::X64, Arch::X86].iter().any(|a| compiler_source(*a).1 == sha)
}

/// `6.10.1` sorts after `6.9.0`.
fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(paths: &DataPaths) -> Self {
        Self { root: paths.tools_dir().join("reshade") }
    }

    #[cfg(test)]
    pub fn at(root: &Path) -> Self {
        Self { root: root.to_path_buf() }
    }

    /// The newest ReShade version (`refresh`: ask even within a day).
    /// `None` offline with nothing cached.
    pub fn latest_version(&self, net: &dyn Net, refresh: bool) -> Option<String> {
        let file = self.root.join("latest.json");
        let cached = heronite_data::json_store::read_value(&file).ok().and_then(|v| v["version"].as_str().map(str::to_string));
        if cached.is_some() && !refresh && fresh(&file) {
            return cached;
        }
        let latest = net.get_json(TAGS_URL).ok().and_then(|tags| {
            tags.as_array()?
                .iter()
                .filter_map(|t| t["name"].as_str()?.strip_prefix('v').map(str::to_string))
                .filter(|v| v.split('.').count() == 3 && v.split('.').all(|p| p.parse::<u32>().is_ok()))
                .max_by_key(|v| version_key(v))
        });
        match latest {
            Some(v) => {
                let _ = std::fs::create_dir_all(&self.root);
                let _ = heronite_data::temps::write_atomic(&file, json!({ "version": v }).to_string().as_bytes(), None);
                Some(v)
            }
            None => cached,
        }
    }

    /// `EffectPackages.ini` (`packages`) or `Compatibility.ini`.
    pub fn catalog(&self, net: &dyn Net, packages: bool, refresh: bool) -> Option<String> {
        let (url, name) = match packages {
            true => (PACKAGES_URL, "EffectPackages.ini"),
            false => (COMPATIBILITY_URL, "Compatibility.ini"),
        };
        let file = self.root.join("catalog").join(name);
        if refresh || !fresh(&file) {
            if let Err(e) = std::fs::create_dir_all(self.root.join("catalog"))
                .map_err(|e| e.to_string())
                .and_then(|_| download(net, url, &file, &|| false))
            {
                log::info!("ReShade {name} not refreshed: {e}");
            }
        }
        std::fs::read_to_string(&file).ok()
    }

    /// The cached `EffectPackages.ini` (`packages`) or `Compatibility.ini`,
    /// without asking the network.
    pub fn cached_catalog(&self, packages: bool) -> Option<String> {
        let name = if packages { "EffectPackages.ini" } else { "Compatibility.ini" };
        std::fs::read_to_string(self.root.join("catalog").join(name)).ok()
    }

    /// The standard DLL of `version` for `arch`, downloaded and checked once.
    pub fn dll(&self, net: &dyn Net, version: &str, arch: Arch, cancel: Cancel) -> Result<PathBuf, String> {
        self.build_dll(net, version, Build::Standard, arch, cancel)
    }

    /// The DLL of `version` and `build` for `arch`, downloaded and checked
    /// once.
    pub fn build_dll(&self, net: &dyn Net, version: &str, build: Build, arch: Arch, cancel: Cancel) -> Result<PathBuf, String> {
        let dir = self.root.join(build.folder(version));
        let dest = dir.join(arch.dll());
        if dest.is_file() && validate_dll(&dest, arch).is_ok() {
            return Ok(dest);
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let setup = dir.join("setup.exe");
        download(net, &build.setup_url(version), &setup, cancel)?;
        let res = [Arch::X64, Arch::X86].iter().try_for_each(|a| {
            let out = dir.join(a.dll());
            archive::extract_file(&setup, a.dll(), &out)?;
            validate_dll(&out, *a).inspect_err(|_| {
                let _ = std::fs::remove_file(&out);
            })
        });
        let _ = std::fs::remove_file(&setup);
        res.map(|_| dest)
    }

    /// The zip of `url` (`refresh`: download again).
    pub fn package(&self, net: &dyn Net, url: &str, refresh: bool, cancel: Cancel) -> Result<PathBuf, String> {
        let dir = self.root.join("packages");
        let dest = dir.join(format!("{}.zip", sha256(url.as_bytes())));
        if dest.is_file() && !refresh {
            return Ok(dest);
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        download(net, url, &dest, cancel)?;
        Ok(dest)
    }

    /// `d3dcompiler_47.dll` for `arch`, checked against its pinned hash.
    pub fn compiler(&self, net: &dyn Net, arch: Arch, cancel: Cancel) -> Result<PathBuf, String> {
        let (url, sha) = compiler_source(arch);
        let dir = self.root.join("d3dcompiler_47").join(arch.key());
        let dest = dir.join("d3dcompiler_47.dll");
        if dest.is_file() && sha256_file(&dest)? == sha {
            return Ok(dest);
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        download(net, &url, &dest, cancel)?;
        let got = sha256_file(&dest)?;
        if got != sha {
            let _ = std::fs::remove_file(&dest);
            return Err(format!("d3dcompiler_47 did not match its checksum ({got})"));
        }
        Ok(dest)
    }

    /// The cached DLL of `version`, without downloading (recovery before a
    /// launch must not wait on the network).
    pub fn cached_dll(&self, version: &str, arch: Arch) -> Option<PathBuf> {
        self.cached_build_dll(version, Build::Standard, arch)
    }

    pub fn cached_build_dll(&self, version: &str, build: Build, arch: Arch) -> Option<PathBuf> {
        Some(self.root.join(build.folder(version)).join(arch.dll())).filter(|p| p.is_file())
    }
}

/// The DLL is a PE for `arch` whose product is ReShade.
pub fn validate_dll(path: &Path, arch: Arch) -> Result<(), String> {
    let info = pe::info(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if Arch::of_machine(info.machine) != Some(arch) {
        return Err(format!("{} is not a {}-bit DLL", path.display(), arch.bits()));
    }
    match info.product_name.as_deref() {
        Some(p) if is_reshade(p) => Ok(()),
        other => Err(format!("{} is not ReShade ({})", path.display(), other.unwrap_or("no product name"))),
    }
}

/// A GitHub repository's address as the zip of its default branch; other
/// HTTPS addresses are taken as zips.
pub fn repo_zip_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    https(url)?;
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() == 2 {
            let repo = parts[1].trim_end_matches(".git");
            return Ok(format!("https://github.com/{}/{repo}/archive/HEAD.zip", parts[0]));
        }
    }
    Ok(url.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::diagnose::pe::tests::pe_with;
    use crate::diagnose::pe::{MACHINE_X64, MACHINE_X86};
    use crate::net::FakeNet;
    use crate::reshade::archive::tests::zip_bytes;

    pub(crate) fn fake_setup() -> Vec<u8> {
        let x64 = pe_with(MACHINE_X64, &[], Some("ReShade"), 0);
        let x86 = pe_with(MACHINE_X86, &[], Some("ReShade"), 0);
        zip_bytes(&pe_with(MACHINE_X86, &[], Some("ReShade Setup"), 0), &[("ReShade64.dll", &x64), ("ReShade32.dll", &x86)])
    }

    #[test]
    fn the_newest_version_is_cached_for_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let net = FakeNet::online().with_json(TAGS_URL, json!([{"name": "v6.9.0"}, {"name": "v6.10.1"}, {"name": "nightly"}]));
        assert_eq!(cache.latest_version(&net, false).as_deref(), Some("6.10.1"));
        assert_eq!(cache.latest_version(&FakeNet::offline(), false).as_deref(), Some("6.10.1"));
        assert_eq!(net.requests().len(), 1);
        assert_eq!(cache.latest_version(&FakeNet::offline(), true).as_deref(), Some("6.10.1"));
    }

    #[test]
    fn the_addon_build_has_its_own_installer_and_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let net = FakeNet::online().with_file(&addon_setup_url("6.8.0"), fake_setup());
        let dll = cache.build_dll(&net, "6.8.0", Build::Addon, Arch::X64, &|| false).unwrap();
        assert!(dll.starts_with(dir.path().join("6.8.0-addon")));
        assert!(cache.cached_build_dll("6.8.0", Build::Addon, Arch::X86).is_some());
        assert!(cache.cached_dll("6.8.0", Arch::X64).is_none());
        assert_eq!(addon_setup_url("6.8.0"), "https://reshade.me/downloads/ReShade_Setup_6.8.0_Addon.exe");
    }

    #[test]
    fn the_dll_comes_from_the_installer_and_is_checked() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let net = FakeNet::online().with_file(&setup_url("6.8.0"), fake_setup());
        let dll = cache.dll(&net, "6.8.0", Arch::X64, &|| false).unwrap();
        assert!(validate_dll(&dll, Arch::X64).is_ok());
        assert!(validate_dll(&dll, Arch::X86).is_err());
        assert!(cache.cached_dll("6.8.0", Arch::X86).is_some());
        assert!(!dir.path().join("6.8.0/setup.exe").exists());
        cache.dll(&net, "6.8.0", Arch::X86, &|| false).unwrap();
        assert_eq!(net.requests().len(), 1);
        let bad =
            FakeNet::online().with_file(&setup_url("6.7.0"), zip_bytes(b"MZ", &[("ReShade64.dll", b"not a dll"), ("ReShade32.dll", b"x")]));
        assert!(cache.dll(&bad, "6.7.0", Arch::X64, &|| false).is_err());
        assert!(cache.cached_dll("6.7.0", Arch::X64).is_none());
    }

    #[test]
    fn the_compiler_must_match_its_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let net = FakeNet::online().with_file(&compiler_source(Arch::X64).0, b"tampered".to_vec());
        assert!(cache.compiler(&net, Arch::X64, &|| false).unwrap_err().contains("checksum"));
        assert!(!dir.path().join("d3dcompiler_47/x64/d3dcompiler_47.dll").exists());
    }

    #[test]
    fn catalogs_fall_back_to_the_cache_offline() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        assert_eq!(cache.catalog(&FakeNet::offline(), true, false), None);
        let net = FakeNet::online().with_file(PACKAGES_URL, b"[00]\nPackageName=A\n".to_vec());
        assert!(cache.catalog(&net, true, false).unwrap().contains("PackageName=A"));
        assert!(cache.catalog(&FakeNet::offline(), true, true).unwrap().contains("PackageName=A"));
    }

    #[test]
    fn repository_addresses() {
        assert_eq!(
            repo_zip_url("https://github.com/mj-ehsan/NiceGuy-Shaders/").unwrap(),
            "https://github.com/mj-ehsan/NiceGuy-Shaders/archive/HEAD.zip"
        );
        assert_eq!(repo_zip_url("https://x.org/a.zip").unwrap(), "https://x.org/a.zip");
        assert!(repo_zip_url("http://github.com/a/b").is_err());
        assert!(is_known_compiler("4432bbd1a390874f3f0a503d45cc48d346abc3a8c0213c289f4b615bf0ee84f3"));
    }
}
