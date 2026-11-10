//! `<data>/reshade/<game id>.json`: what Heronite put in a game's folder,
//! so it can switch, update and remove exactly that, and never touch a file
//! it did not write (or one the user changed since: its hash differs).

use std::path::{Path, PathBuf};

use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// ReShade itself, under the proxy's name.
    Dll,
    /// `d3dcompiler_47.dll` (Wine and Proton only).
    Compiler,
    Shader,
    Texture,
    /// The copy of an imported preset.
    Preset,
    /// `ReShade.ini`.
    Config,
    /// A ReShade add-on (`.addon64` / `.addon32`), such as a RenoDX mod;
    /// its source names the mod.
    Addon,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative to `target_dir`, with `/`.
    pub path: String,
    pub sha256: String,
    pub kind: Kind,
    /// Packages and extra sources that brought the file; removed with the
    /// last one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
}

/// The source name of files that were there when a manual install was
/// adopted.
pub const ADOPTED: &str = "adopted";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// `x86` / `x64`.
    pub arch: String,
    /// [`super::detect::Api::key`].
    pub api: String,
    /// The DLL name ReShade has, like `dxgi.dll`.
    pub proxy: String,
    pub target_dir: String,
    /// Found installed by hand and taken over.
    #[serde(default)]
    pub adopted: bool,
    /// `fxc2` when Heronite put `d3dcompiler_47.dll` there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler: Option<String>,
    /// The build with add-on support (`false`: the standard build).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub addon: bool,
    /// The DLL has its name (`false`: renamed with the off suffix).
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub packages: Vec<String>,
    /// Folders and repository addresses added by the user.
    #[serde(default)]
    pub extra_sources: Vec<String>,
    /// File name of the preset copy in `target_dir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    pub installed_at: String,
}

fn yes() -> bool {
    true
}

pub fn path(paths: &DataPaths, game_id: &str) -> PathBuf {
    paths.root.join("reshade").join(format!("{game_id}.json"))
}

pub fn load(paths: &DataPaths, game_id: &str) -> Option<Manifest> {
    let text = std::fs::read_to_string(path(paths, game_id)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(paths: &DataPaths, game_id: &str, m: &Manifest) -> Result<(), String> {
    let file = path(paths, game_id);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    heronite_data::temps::write_atomic(&file, text.as_bytes(), None).map_err(|e| format!("{}: {e}", file.display()))
}

pub fn delete(paths: &DataPaths, game_id: &str) {
    let _ = std::fs::remove_file(path(paths, game_id));
}

/// `a/b/c.fx` below `dir`, with the system's separators.
pub fn join(dir: &Path, rel: &str) -> PathBuf {
    rel.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "." && *p != "..").fold(dir.to_path_buf(), |d, p| d.join(p))
}

impl Manifest {
    pub fn dir(&self) -> PathBuf {
        PathBuf::from(&self.target_dir)
    }

    /// Where the DLL is now (its own name, or the switched-off one).
    pub fn dll_path(&self) -> PathBuf {
        let name = match self.enabled {
            true => self.proxy.clone(),
            false => format!("{}{}", self.proxy, super::detect::OFF_SUFFIX),
        };
        self.dir().join(name)
    }

    pub fn file(&self, rel: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path.eq_ignore_ascii_case(rel))
    }

    /// Adds or updates the entry of `rel`, adding `source` to its sources.
    pub fn record(&mut self, rel: &str, sha256: String, kind: Kind, source: Option<&str>) {
        let i = match self.files.iter().position(|f| f.path.eq_ignore_ascii_case(rel)) {
            Some(i) => i,
            None => {
                self.files.push(FileEntry { path: rel.to_string(), sha256: String::new(), kind, source: Vec::new() });
                self.files.len() - 1
            }
        };
        let f = &mut self.files[i];
        f.sha256 = sha256;
        f.kind = kind;
        if let Some(s) = source {
            if !f.source.iter().any(|x| x == s) {
                f.source.push(s.to_string());
            }
        }
    }

    /// Every package and extra source, catalog packages first.
    pub fn sources(&self) -> Vec<String> {
        self.packages.iter().chain(&self.extra_sources).cloned().collect()
    }

    /// Effect files (`.fx`, by file name) that `source` brought.
    pub fn effects_of(&self, source: &str) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.kind == Kind::Shader && f.source.iter().any(|s| s == source) && f.path.to_ascii_lowercase().ends_with(".fx"))
            .filter_map(|f| f.path.rsplit('/').next().map(str::to_string))
            .collect()
    }

    /// Whether an effect file is installed, from any source.
    pub fn has_effect(&self, file: &str) -> bool {
        self.files.iter().any(|f| f.kind == Kind::Shader && f.path.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case(file)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Manifest {
        Manifest {
            version: "6.8.0".into(),
            arch: "x64".into(),
            api: "dxgi".into(),
            proxy: "dxgi.dll".into(),
            target_dir: "/g".into(),
            adopted: false,
            addon: false,
            compiler: None,
            enabled: true,
            packages: vec!["Standard effects".into()],
            extra_sources: Vec::new(),
            preset: None,
            files: Vec::new(),
            installed_at: "2026-11-05T12:00:00Z".into(),
        }
    }

    #[test]
    fn records_sources_and_effects() {
        let mut m = sample();
        m.record("reshade-shaders/Shaders/ReShade.fxh", "a".into(), Kind::Shader, Some("Standard effects"));
        m.record("reshade-shaders/Shaders/ReShade.fxh", "b".into(), Kind::Shader, Some("SweetFX"));
        m.record("reshade-shaders/Shaders/SweetFX/Tonemap.fx", "c".into(), Kind::Shader, Some("SweetFX"));
        let fxh = m.file("reshade-shaders/shaders/reshade.fxh").unwrap();
        assert_eq!((fxh.sha256.as_str(), fxh.source.clone()), ("b", vec!["Standard effects".to_string(), "SweetFX".to_string()]));
        assert_eq!(m.effects_of("SweetFX"), ["Tonemap.fx"]);
        assert!(m.has_effect("tonemap.fx") && !m.has_effect("Curves.fx"));
        assert_eq!(m.dll_path(), Path::new("/g").join("dxgi.dll"));
        m.enabled = false;
        assert_eq!(m.dll_path(), Path::new("/g").join("dxgi.dll.heronite-off"));
    }

    #[test]
    fn round_trips_through_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(load(&paths, "g").is_none());
        save(&paths, "g", &sample()).unwrap();
        assert_eq!(load(&paths, "g"), Some(sample()));
        delete(&paths, "g");
        assert!(load(&paths, "g").is_none());
        assert_eq!(join(Path::new("/g"), ".\\reshade-shaders\\Shaders"), Path::new("/g").join("reshade-shaders").join("Shaders"));
    }
}
