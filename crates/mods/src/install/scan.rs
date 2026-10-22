//! What a finished mod folder holds: counts, plugins, proxy DLLs.

use std::path::Path;

use crate::fsutil::{entries, find_ci, totals};

/// Proxy DLLs that mod loaders drop next to the game's executable and that
/// Wine must load instead of its own (`WINEDLLOVERRIDES=<name>=n,b`).
pub const PROXY_DLLS: &[&str] = &["winhttp", "version", "dinput8", "winmm", "dsound", "xinput1_3", "dwmapi"];

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Scan {
    pub files: u64,
    pub size: u64,
    pub plugins: Vec<String>,
    pub dll_overrides: Vec<String>,
}

/// Looks through `mod_dir`; plugins are looked for in `plugins_dir` (relative
/// to it, e.g. `Data`) when given.
pub(crate) fn scan(mod_dir: &Path, plugins_dir: Option<&str>) -> Scan {
    let (files, size) = totals(mod_dir);
    let mut plugins = Vec::new();
    if let Some(rel) = plugins_dir.filter(|r| !r.is_empty()) {
        let mut dir = mod_dir.to_path_buf();
        let mut found = true;
        for c in rel.split('/').filter(|c| !c.is_empty()) {
            match find_ci(&dir, c) {
                Some((name, true)) => dir = dir.join(name),
                _ => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            plugins = entries(&dir)
                .into_iter()
                .filter(|(n, is_dir)| !is_dir && matches!(ext(n).as_str(), "esp" | "esm" | "esl"))
                .map(|(n, _)| n)
                .collect();
        }
    }
    let mut dll_overrides: Vec<String> = entries(mod_dir)
        .into_iter()
        .filter(|(_, is_dir)| !is_dir)
        .filter_map(|(n, _)| n.to_lowercase().strip_suffix(".dll").map(str::to_string))
        .filter(|stem| PROXY_DLLS.contains(&stem.as_str()))
        .collect();
    dll_overrides.dedup();
    Scan { files, size, plugins, dll_overrides }
}

fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn counts_plugins_and_proxy_dlls() {
        let t = tempfile::tempdir().unwrap();
        for f in [
            "Data/Mod.esp",
            "Data/Base.ESM",
            "Data/light.esl",
            "Data/meshes/x.nif",
            "Data/readme.txt",
            "winhttp.dll",
            "Version.DLL",
            "other.dll",
        ] {
            let p = t.path().join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "abc").unwrap();
        }
        let s = scan(t.path(), Some("data"));
        assert_eq!((s.files, s.size), (8, 24));
        assert_eq!(s.plugins, ["Base.ESM", "Mod.esp", "light.esl"]);
        assert_eq!(s.dll_overrides, ["version", "winhttp"]);
        assert!(scan(t.path(), Some("Missing")).plugins.is_empty());
        assert!(scan(t.path(), None).plugins.is_empty());
    }
}
