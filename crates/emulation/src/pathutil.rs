//! POSIX flavours of Node's `path` helpers, so results match Heronite's
//! (`resolve`, `join`, `extname`...) byte for byte.

/// `path.normalize` (POSIX).
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

/// `path.join(a, b)`.
pub fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        return normalize(b);
    }
    if b.is_empty() {
        return normalize(a);
    }
    normalize(&format!("{a}/{b}"))
}

/// `path.resolve(p)` against the process working directory.
pub fn resolve(path: &str) -> String {
    if path.starts_with('/') {
        return resolve_abs(path);
    }
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
    resolve_abs(&format!("{cwd}/{path}"))
}

/// `path.resolve(base, p)`.
pub fn resolve_from(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        return resolve_abs(path);
    }
    resolve_abs(&format!("{}/{path}", resolve(base)))
}

fn resolve_abs(path: &str) -> String {
    let n = normalize(path);
    if n.len() > 1 {
        n.trim_end_matches('/').to_string()
    } else {
        n
    }
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

    #[test]
    fn normalize_and_join() {
        assert_eq!(normalize("/a/b/../c/./d"), "/a/c/d");
        assert_eq!(normalize("a//b/"), "a/b/");
        assert_eq!(normalize("../x"), "../x");
        assert_eq!(normalize("/.."), "/");
        assert_eq!(join("/roms", "../x/y.iso"), "/x/y.iso");
        assert_eq!(join("/roms/", "g/a.cue"), "/roms/g/a.cue");
        assert_eq!(resolve("/a/b/"), "/a/b");
        assert_eq!(resolve_from("/base", "sub/x"), "/base/sub/x");
        assert_eq!(resolve_from("/base", "/abs"), "/abs");
    }

    #[test]
    fn ext_helpers() {
        assert_eq!(extname("/x/Game.v2.ISO"), ".ISO");
        assert_eq!(ext_lower("/x/Game.v2.ISO"), "iso");
        assert_eq!(extname("/x/.hidden"), "");
        assert_eq!(base_no_ext("/x/Game (USA).iso"), "Game (USA)");
        assert_eq!(normalize_key("/Roms/Game/"), "/roms/game");
        assert_eq!(normalize_key("/Roms\\Game"), "/roms\\game".replace('\\', "/"));
    }
}
