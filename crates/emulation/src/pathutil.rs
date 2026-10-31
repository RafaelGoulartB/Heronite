//! Node's `path` helpers of this system, so results match Heronite's
//! (`resolve`, `join`, `extname`...) byte for byte: the POSIX ones on Linux,
//! `path.win32` on Windows (`\` and `/` both separate, drive and share
//! roots, `\` in what they give).

use std::path::Path;

/// `path.normalize` (POSIX).
#[cfg(not(windows))]
pub fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if matches!(out.last(), Some(l) if *l != "..") {
                    out.pop();
                } else if !absolute {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    let mut joined = out.join("/");
    if joined.is_empty() && !absolute {
        joined = ".".into();
    }
    if trailing && !joined.is_empty() && joined != "/" {
        joined.push('/');
    }
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// `path.normalize` (`path.win32`).
#[cfg(windows)]
pub fn normalize(path: &str) -> String {
    use std::path::Component;
    if path.is_empty() {
        return ".".into();
    }
    let mut prefix = String::new();
    let mut absolute = false;
    let mut out: Vec<String> = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Prefix(p) => prefix = p.as_os_str().to_string_lossy().into_owned(),
            Component::RootDir => absolute = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.last(), Some(l) if l != "..") {
                    out.pop();
                } else if !absolute {
                    out.push("..".into());
                }
            }
            Component::Normal(s) => out.push(s.to_string_lossy().into_owned()),
        }
    }
    let mut joined = out.join("\\");
    if joined.is_empty() && !absolute && prefix.is_empty() {
        joined = ".".into();
    }
    if path.ends_with(['/', '\\']) && !joined.is_empty() {
        joined.push('\\');
    }
    format!("{prefix}{}{joined}", if absolute { "\\" } else { "" })
}

/// `path.join(a, b)`.
pub fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        return normalize(b);
    }
    if b.is_empty() {
        return normalize(a);
    }
    if cfg!(windows) {
        normalize(&Path::new(a).join(b).to_string_lossy())
    } else {
        normalize(&format!("{a}/{b}"))
    }
}

/// `path.join(base, ...parts)`.
pub fn join_all(base: &str, parts: &[&str]) -> String {
    parts.iter().fold(base.to_string(), |acc, p| join(&acc, p))
}

/// Whether `path` names a file in a folder rather than a bare command
/// (`pcsx2-qt`, found on `$PATH`): it has a separator of this system.
pub fn has_dir(path: &str) -> bool {
    path.chars().any(std::path::is_separator)
}

/// `path.isAbsolute` of this system (`/x`; `C:\x` or `\\server\share` on
/// Windows).
pub fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
}

/// `path.resolve(p)` against the process working directory.
pub fn resolve(path: &str) -> String {
    if is_absolute(path) {
        return resolve_abs(path);
    }
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
    resolve_abs(&join(&cwd, path))
}

/// `path.resolve(base, p)`.
pub fn resolve_from(base: &str, path: &str) -> String {
    if is_absolute(path) {
        return resolve_abs(path);
    }
    resolve_abs(&join(&resolve(base), path))
}

/// Normalized, without a trailing separator unless it is the root.
fn resolve_abs(path: &str) -> String {
    let n = normalize(path);
    if Path::new(&n).parent().is_none() {
        return n;
    }
    n.trim_end_matches(std::path::is_separator).to_string()
}

pub use heronite_launch::emulators::{node_basename as basename, node_dirname as dirname};

/// `path.extname`.
pub fn extname(path: &str) -> String {
    let base = basename(path);
    match base.rfind('.') {
        Some(0) | None => String::new(),
        Some(i) => base[i..].to_string(),
    }
}

/// Lower-cased extension without the dot.
pub fn ext_lower(path: &str) -> String {
    extname(path).trim_start_matches('.').to_lowercase()
}

/// `normalizePath` of scan.ts / import-roms.ts: resolved, `/`-separated,
/// no trailing slash, lower-cased.
pub fn normalize_key(value: &str) -> String {
    resolve(value).replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Name without its extension (`basename(p, extname(p))`).
pub fn base_no_ext(path: &str) -> String {
    let base = basename(path);
    let ext = extname(&base);
    base[..base.len() - ext.len()].to_string()
}

/// Non-empty values without repeats, in first-seen order (candidate lists).
pub fn unique(values: impl IntoIterator<Item = Option<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values.into_iter().flatten() {
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn normalize_and_join() {
        assert_eq!(normalize("/a/b/../c/./d"), "/a/c/d");
        assert_eq!(normalize("a//b/"), "a/b/");
        assert_eq!(normalize("../x"), "../x");
        assert_eq!(normalize("/.."), "/");
        assert_eq!(join("/roms", "../x/y.iso"), "/x/y.iso");
        assert_eq!(join("/roms/", "g/a.cue"), "/roms/g/a.cue");
        assert_eq!(join_all("/home/u", &[".config", "retroarch"]), "/home/u/.config/retroarch");
        assert_eq!(resolve("/a/b/"), "/a/b");
        assert_eq!(resolve("/"), "/");
        assert_eq!(resolve_from("/base", "sub/x"), "/base/sub/x");
        assert_eq!(resolve_from("/base", "/abs"), "/abs");
        assert_eq!(normalize_key("/Roms/Game/"), "/roms/game");
        assert_eq!(normalize_key("/Roms\\Game"), "/roms\\game".replace('\\', "/"));
        assert!(has_dir("/usr/bin/pcsx2-qt") && !has_dir("pcsx2-qt") && !has_dir(r"C:\x"));
        assert!(is_absolute("/roms") && !is_absolute("roms"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_and_join_on_windows() {
        assert_eq!(normalize(r"C:\a\b\..\c\.\d"), r"C:\a\c\d");
        assert_eq!(normalize("C:/ROMs//ps2/"), "C:\\ROMs\\ps2\\");
        assert_eq!(normalize(r"..\x"), r"..\x");
        assert_eq!(normalize(r"C:\.."), "C:\\");
        assert_eq!(normalize(r"\\nas\roms\ps2\..\gba"), r"\\nas\roms\gba");
        assert_eq!(join(r"C:\ROMs", r"..\x/y.iso"), r"C:\x\y.iso");
        assert_eq!(join("C:\\ROMs\\", "g/a.cue"), r"C:\ROMs\g\a.cue");
        assert_eq!(join("C:/ROMs", "Game (USA).cue"), r"C:\ROMs\Game (USA).cue");
        assert_eq!(join_all(r"C:\Users\u", &["scoop", "apps"]), r"C:\Users\u\scoop\apps");
        assert_eq!(resolve("C:\\a\\b\\"), r"C:\a\b");
        assert_eq!(resolve("C:\\"), "C:\\");
        assert_eq!(resolve_from(r"C:\base", r"sub\x"), r"C:\base\sub\x");
        assert_eq!(resolve_from(r"C:\base", r"D:\abs"), r"D:\abs");
        assert_eq!(normalize_key("C:\\ROMs\\Game\\"), "c:/roms/game");
        assert_eq!(normalize_key("c:/roms/game"), normalize_key(r"C:\ROMs\GAME"));
        assert!(has_dir(r"C:\PCSX2\pcsx2-qt.exe") && has_dir("C:/PCSX2/pcsx2-qt.exe") && !has_dir("pcsx2-qt.exe"));
        assert!(is_absolute(r"D:\ROMs") && is_absolute(r"\\nas\roms\x") && !is_absolute("ROMs"));
        assert_eq!(base_no_ext(r"D:\ROMs\Game (USA).iso"), "Game (USA)");
        assert_eq!(ext_lower(r"D:\ROMs.v2\Game.ISO"), "iso");
    }

    #[test]
    fn ext_helpers() {
        assert_eq!(extname("/x/Game.v2.ISO"), ".ISO");
        assert_eq!(ext_lower("/x/Game.v2.ISO"), "iso");
        assert_eq!(extname("/x/.hidden"), "");
        assert_eq!(base_no_ext("/x/Game (USA).iso"), "Game (USA)");
    }
}
