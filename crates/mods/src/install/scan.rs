//! What a finished mod folder holds: counts, plugins, proxy DLLs.

use std::path::Path;

use crate::fsutil::{entries, extension as ext, find_ci, totals};

/// Windows system DLLs Wine ships its own copy of. A mod that puts one next
/// to the game's executable is a loader using it as a proxy (BepInEx's
/// `winhttp`, Control's Loose Files Loader `iphlpapi`, ReShade's `dxgi`, ASI
/// loaders' `dinput8`…): Wine must load the mod's first
/// (`WINEDLLOVERRIDES=<name>=n,b`) or the loader never runs.
pub const PROXY_DLLS: &[&str] = &[
    "avrt",
    "bcrypt",
    "cryptbase",
    "d2d1",
    "d3d10",
    "d3d10_1",
    "d3d10core",
    "d3d11",
    "d3d12",
    "d3d8",
    "d3d9",
    "d3dcompiler_43",
    "d3dcompiler_47",
    "dbgcore",
    "dbghelp",
    "ddraw",
    "dinput",
    "dinput8",
    "dsound",
    "dwmapi",
    "dwrite",
    "dxgi",
    "hid",
    "iphlpapi",
    "msacm32",
    "msimg32",
    "mswsock",
    "netapi32",
    "opengl32",
    "powrprof",
    "profapi",
    "secur32",
    "shfolder",
    "userenv",
    "uxtheme",
    "version",
    "wer",
    "winhttp",
    "wininet",
    "winmm",
    "winspool",
    "wintrust",
    "ws2_32",
    "wsock32",
    "xinput1_1",
    "xinput1_2",
    "xinput1_3",
    "xinput1_4",
    "xinput9_1_0",
];

/// A file that is a proxy DLL by its name (`winhttp.dll`, any case).
pub fn is_proxy_dll(file_name: &str) -> bool {
    file_name.to_lowercase().strip_suffix(".dll").is_some_and(|stem| PROXY_DLLS.contains(&stem))
}

/// How deep the game folder is searched for executables.
const EXE_DEPTH: usize = 3;

/// The folders of `game_dir` (relative, `/`-separated, `""` = the folder
/// itself) that hold an executable: where a proxy DLL takes effect.
pub fn exe_dirs(game_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![(game_dir.to_path_buf(), String::new(), 0)];
    while let Some((dir, rel, depth)) = todo.pop() {
        let items = entries(&dir);
        if items.iter().any(|(n, is_dir)| !is_dir && ext(n) == "exe") {
            out.push(rel.clone());
        }
        if depth < EXE_DEPTH {
            for (n, _) in items.into_iter().filter(|(_, d)| *d) {
                let child = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
                todo.push((dir.join(&n), child, depth + 1));
            }
        }
    }
    out.sort();
    out
}

/// The proxy DLLs of a mod folder: at its root, and in the folders `exe_dirs`
/// names (matched without regard to case). Lower case, without `.dll`.
pub fn proxy_dlls(mod_dir: &Path, exe_dirs: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    let mut dirs: Vec<&str> = vec![""];
    dirs.extend(exe_dirs.iter().map(String::as_str).filter(|d| !d.is_empty()));
    for rel in dirs {
        let mut dir = mod_dir.to_path_buf();
        let reached = rel.split('/').filter(|c| !c.is_empty()).all(|c| match find_ci(&dir, c) {
            Some((name, true)) => {
                dir = dir.join(name);
                true
            }
            _ => false,
        });
        if !reached {
            continue;
        }
        found.extend(
            entries(&dir)
                .into_iter()
                .filter(|(_, is_dir)| !is_dir)
                .filter_map(|(n, _)| n.to_lowercase().strip_suffix(".dll").map(str::to_string))
                .filter(|stem| PROXY_DLLS.contains(&stem.as_str())),
        );
    }
    found.sort();
    found.dedup();
    found
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Scan {
    pub files: u64,
    pub size: u64,
    pub plugins: Vec<String>,
    pub dll_overrides: Vec<String>,
}

/// Looks through `mod_dir`; plugins are looked for in `plugins_dir` (relative
/// to it, e.g. `Data`) when given, proxy DLLs at its root and in `exe_dirs`.
pub(crate) fn scan(mod_dir: &Path, plugins_dir: Option<&str>, exe_dirs: &[String]) -> Scan {
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
    let dll_overrides = proxy_dlls(mod_dir, exe_dirs);
    Scan { files, size, plugins, dll_overrides }
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
        let s = scan(t.path(), Some("data"), &[]);
        assert_eq!((s.files, s.size), (8, 24));
        assert_eq!(s.plugins, ["Base.ESM", "Mod.esp", "light.esl"]);
        assert_eq!(s.dll_overrides, ["version", "winhttp"]);
        assert!(scan(t.path(), Some("Missing"), &[]).plugins.is_empty());
        assert!(scan(t.path(), None, &[]).plugins.is_empty());
    }

    #[test]
    fn proxy_dlls_count_where_the_game_has_an_executable() {
        let game = tempfile::tempdir().unwrap();
        for f in ["Control.exe", "bin/x64/Cyberpunk2077.exe", "data/x.rmdp"] {
            let p = game.path().join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        let dirs = exe_dirs(game.path());
        assert_eq!(dirs, ["", "bin/x64"]);
        let m = tempfile::tempdir().unwrap();
        for f in ["iphlpapi.dll", "BIN/X64/version.dll", "data/dxgi.dll", "mymod.dll"] {
            let p = m.path().join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        assert_eq!(proxy_dlls(m.path(), &dirs), ["iphlpapi", "version"], "not in data/, not an unknown name");
    }
}
