//! Finding the binary and reading Ludusavi's own `config.yaml`.
//!
//! Linux: `$PATH`, `/usr/bin`, `/usr/local/bin`, `~/.local/bin`, config in
//! `~/.config/ludusavi`. Windows: `ludusavi.exe` on `PATH`, the folder the
//! download is unpacked to, winget and Scoop; config in `%APPDATA%\ludusavi`
//! (next to the program for a portable copy).

use std::path::Path;

use heronite_launch::probe::Probe;
use serde::{Deserialize, Serialize};

use super::runner::CommandRunner;
use crate::pathutil::{join, join_all};

/// `LudusaviDetectedConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LudusaviDetected {
    pub binary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    /// Where Ludusavi restores from (`restore.path`), often another folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_path: Option<String>,
    /// 'zip' | 'simple'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// 'none' | 'deflate' | 'bzip2' | 'zstd'
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// `yamlGet`: a scalar at a nested key path of the (indentation-only) config.
pub fn yaml_get(text: &str, keys: &[&str]) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut from = 0usize;
    let mut indent = 0usize;
    for (depth, key) in keys.iter().enumerate() {
        let needle = format!("{key}:");
        let mut next: Option<(usize, usize)> = None;
        let mut i = from;
        while i < lines.len() {
            let raw = lines[i];
            let trimmed = raw.trim_start();
            if raw.trim().is_empty() || trimmed.starts_with('#') {
                i += 1;
                continue;
            }
            let current = raw.chars().take_while(|c| *c == ' ').count();
            if i != from && current < indent {
                return None;
            }
            if current != indent || !trimmed.starts_with(&needle) {
                i += 1;
                continue;
            }
            let rest = trimmed[needle.len()..].trim();
            if depth == keys.len() - 1 {
                if rest.is_empty() || rest == "|" || rest == ">" {
                    return None;
                }
                // Ludusavi on Windows writes a picked folder double-quoted, its
                // backslashes escaped (`"D:\\Saves"`).
                let escaped = cfg!(windows) && rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"');
                let rest = rest.strip_prefix(['\'', '"']).unwrap_or(rest);
                let rest = rest.strip_suffix(['\'', '"']).unwrap_or(rest);
                return Some(if escaped { unescape_double_quoted(rest) } else { rest.to_string() });
            }
            next = Some((i + 1, current + 2));
            break;
        }
        let (f, ind) = next?;
        from = f;
        indent = ind;
    }
    None
}

/// The text of a double-quoted YAML scalar: `\\`, `\"` and `\/` are one
/// character.
fn unescape_double_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&next @ ('\\' | '"' | '/'))) => {
                out.push(next);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

fn expand_home(value: Option<String>, home: &str) -> Option<String> {
    let v = value.filter(|v| !v.is_empty())?;
    if v == "~" {
        Some(home.to_string())
    } else if let Some(rest) = v.strip_prefix("~/") {
        Some(join(home, rest))
    } else {
        Some(v)
    }
}

/// `resolveBinary`: the preferred path, `$PATH`, then the usual places.
pub fn resolve_binary(probe: &dyn Probe, preferred: Option<&str>) -> Option<String> {
    let preferred = preferred.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string);
    if cfg!(windows) {
        // A Scoop shim on `PATH` starts the real program: run that one, so a
        // timeout stops Ludusavi itself.
        let on_path = probe.which("ludusavi").map(|p| crate::detect::scoop_shim_target(&p).unwrap_or(p));
        return [preferred, on_path].into_iter().flatten().chain(windows_places(probe)).find(|p| probe.exists(p));
    }
    let home = probe.home();
    [
        preferred,
        probe.which("ludusavi"),
        Some("/usr/bin/ludusavi".into()),
        Some("/usr/local/bin/ludusavi".into()),
        Some(join(&home, ".local/bin/ludusavi")),
    ]
    .into_iter()
    .flatten()
    .find(|p| probe.exists(p))
}

/// `%VAR%` of the probe, `None` when unset or empty.
fn var(probe: &dyn Probe, key: &str) -> Option<String> {
    probe.env_var(key).filter(|v| !v.is_empty())
}

/// Where `ludusavi.exe` is on Windows besides `PATH`: the folder the
/// download is unpacked to (`%LOCALAPPDATA%\Programs\Ludusavi`), winget's
/// link and package folder (`winget install mtkennerly.ludusavi`), Scoop's
/// app (`scoop install ludusavi`, the user's and the global one) and
/// Program Files.
fn windows_places(probe: &dyn Probe) -> Vec<String> {
    let home = probe.home();
    let local = var(probe, "LOCALAPPDATA").unwrap_or_else(|| join_all(&home, &["AppData", "Local"]));
    let winget = join_all(&local, &["Microsoft", "WinGet"]);
    let packages = join(&winget, "Packages");
    let mut out = vec![join_all(&local, &["Programs", "Ludusavi", "ludusavi.exe"]), join_all(&winget, &["Links", "ludusavi.exe"])];
    let mut by_winget: Vec<String> = probe
        .list_dir(&packages)
        .into_iter()
        .filter(|name| name.to_ascii_lowercase().starts_with("mtkennerly.ludusavi_"))
        .map(|name| join_all(&packages, &[&name, "ludusavi.exe"]))
        .collect();
    by_winget.sort();
    out.extend(by_winget);
    let scoop = var(probe, "SCOOP").unwrap_or_else(|| join(&home, "scoop"));
    let scoop_global = var(probe, "SCOOP_GLOBAL").or_else(|| var(probe, "ProgramData").map(|d| join(&d, "scoop")));
    for root in [Some(scoop), scoop_global].into_iter().flatten() {
        out.push(join_all(&root, &["apps", "ludusavi", "current", "ludusavi.exe"]));
    }
    if let Some(program_files) = var(probe, "ProgramFiles") {
        out.push(join_all(&program_files, &["Ludusavi", "ludusavi.exe"]));
    }
    out
}

/// Ludusavi's `config.yaml` on Windows when `config path` gives none: next
/// to a portable copy (a `ludusavi.portable` file beside the program), else
/// in `%APPDATA%\ludusavi`.
pub(super) fn windows_config_file(probe: &dyn Probe, binary: Option<&str>) -> String {
    let dir = binary.and_then(|b| Path::new(b).parent()).map(|d| d.to_string_lossy().into_owned()).filter(|d| !d.is_empty());
    if let Some(dir) = dir.filter(|d| probe.exists(&join(d, "ludusavi.portable"))) {
        return join(&dir, "config.yaml");
    }
    let appdata = var(probe, "APPDATA").unwrap_or_else(|| join_all(&probe.home(), &["AppData", "Roaming"]));
    join_all(&appdata, &["ludusavi", "config.yaml"])
}

/// Parses the `backup:` part of Ludusavi's `config.yaml`.
pub fn parse_config(yaml: &str, home: &str) -> (Option<String>, Option<String>, Option<String>) {
    let backup_path = expand_home(yaml_get(yaml, &["backup", "path"]), home);
    let format = yaml_get(yaml, &["backup", "format", "chosen"]).filter(|f| f == "zip" || f == "simple");
    let compression =
        yaml_get(yaml, &["backup", "format", "zip", "compression"]).filter(|c| matches!(c.as_str(), "none" | "deflate" | "bzip2" | "zstd"));
    (backup_path, format, compression)
}

/// `restore.path` of Ludusavi's `config.yaml`.
pub fn parse_restore_path(yaml: &str, home: &str) -> Option<String> {
    expand_home(yaml_get(yaml, &["restore", "path"]), home)
}

/// `detectLudusavi`.
pub fn detect_ludusavi(runner: &dyn CommandRunner, probe: &dyn Probe, preferred: Option<&str>) -> Option<LudusaviDetected> {
    let binary = resolve_binary(probe, preferred)?;
    let home = probe.home();

    let version = runner.run(&binary, &["--version".into()]).ok().and_then(|o| {
        let text = if o.stdout.is_empty() { o.stderr } else { o.stdout };
        let re = regex::Regex::new(r"(?i)ludusavi\s+([\w.-]+)").unwrap();
        re.captures(&text).map(|c| c[1].to_string())
    });

    let config_path = runner
        .run(&binary, &["config".into(), "path".into()])
        .ok()
        .map(|o| o.stdout.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                windows_config_file(probe, Some(&binary))
            } else {
                join(&home, ".config/ludusavi/config.yaml")
            }
        });
    let exists = Path::new(&config_path).exists();
    let (mut backup_path, mut format, mut compression, mut restore_path) = (None, None, None, None);
    if exists {
        match std::fs::read_to_string(&config_path) {
            Ok(yaml) => {
                (backup_path, format, compression) = parse_config(&yaml, &home);
                restore_path = parse_restore_path(&yaml, &home);
            }
            Err(e) => log::error!("Failed to read Ludusavi config: {e}"),
        }
    }
    Some(LudusaviDetected { binary, config_path: exists.then_some(config_path), backup_path, restore_path, format, compression, version })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ludusavi::fake::{out, Fake};
    use heronite_launch::probe::FakeProbe;

    const YAML: &str = "runtime:\n  threads: ~\nbackup:\n  path: ~/Saves\n  ignoredGames:\n    - X\n  format:\n    chosen: zip\n    zip:\n      compression: zstd\n    compression:\n      deflate:\n        level: 6\nrestore:\n  path: /elsewhere\n";

    #[test]
    fn yaml_paths() {
        assert_eq!(yaml_get(YAML, &["backup", "path"]).as_deref(), Some("~/Saves"));
        assert_eq!(yaml_get(YAML, &["backup", "format", "chosen"]).as_deref(), Some("zip"));
        assert_eq!(yaml_get(YAML, &["backup", "format", "zip", "compression"]).as_deref(), Some("zstd"));
        assert_eq!(yaml_get(YAML, &["restore", "path"]).as_deref(), Some("/elsewhere"));
        assert_eq!(yaml_get(YAML, &["backup", "missing"]), None);
        assert_eq!(yaml_get(YAML, &["runtime", "threads"]).as_deref(), Some("~"));
        assert_eq!(yaml_get("a:\n  b: \"quoted\"\n", &["a", "b"]).as_deref(), Some("quoted"));
        assert_eq!(yaml_get("a:\n  b: |\n    text\n", &["a", "b"]), None);
        let (p, f, c) = parse_config(YAML, "/home/u");
        assert_eq!((p, f.as_deref(), c.as_deref()), (Some(join("/home/u", "Saves")), Some("zip"), Some("zstd")));
    }

    #[test]
    fn detection_reads_version_and_config() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config.yaml");
        std::fs::write(&cfg, YAML).unwrap();
        let runner = Fake {
            replies: vec![
                ("--version".into(), out("ludusavi v0.30.0\n", "", 0)),
                ("config path".into(), out(&format!("{}\n", cfg.display()), "", 0)),
            ],
            ..Default::default()
        };
        let probe = FakeProbe::new("/home/u").with_bin("ludusavi", "/opt/bin/ludusavi");
        let det = detect_ludusavi(&runner, &probe, None).unwrap();
        assert_eq!(det.binary, "/opt/bin/ludusavi");
        assert_eq!(det.version.as_deref(), Some("v0.30.0"));
        assert_eq!(det.config_path.as_deref(), cfg.to_str());
        assert_eq!(det.backup_path, Some(join("/home/u", "Saves")));
        assert_eq!(det.restore_path.as_deref(), Some("/elsewhere"));
        assert_eq!(det.format.as_deref(), Some("zip"));
        assert_eq!(det.compression.as_deref(), Some("zstd"));
        // Preferred binary wins; nothing found -> None.
        let probe2 = FakeProbe::new("/home/u").with_path("/custom/lud");
        assert_eq!(detect_ludusavi(&runner, &probe2, Some(" /custom/lud ")).unwrap().binary, "/custom/lud");
        assert!(detect_ludusavi(&runner, &FakeProbe::new("/home/u"), None).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_finds_the_exe_in_the_usual_places() {
        let home = r"C:\Users\u";
        let local = r"C:\Users\u\AppData\Local";
        let unpacked = r"C:\Users\u\AppData\Local\Programs\Ludusavi\ludusavi.exe";
        let link = r"C:\Users\u\AppData\Local\Microsoft\WinGet\Links\ludusavi.exe";
        let package =
            r"C:\Users\u\AppData\Local\Microsoft\WinGet\Packages\mtkennerly.ludusavi_Microsoft.Winget.Source_8wekyb3d8bbwe\ludusavi.exe";
        let scoop = r"D:\Scoop\apps\ludusavi\current\ludusavi.exe";
        let global = r"C:\ProgramData\scoop\apps\ludusavi\current\ludusavi.exe";
        let program_files = r"C:\Program Files\Ludusavi\ludusavi.exe";
        let base = || FakeProbe::new(home).with_env("LOCALAPPDATA", local).with_env("ProgramData", r"C:\ProgramData");
        let found = |p: FakeProbe| resolve_binary(&p, None);
        assert_eq!(found(base()), None);
        assert_eq!(found(base().with_path(program_files).with_env("ProgramFiles", r"C:\Program Files")).as_deref(), Some(program_files));
        assert_eq!(found(base().with_path(global)).as_deref(), Some(global));
        assert_eq!(found(base().with_env("SCOOP", r"D:\Scoop").with_path(scoop).with_path(global)).as_deref(), Some(scoop));
        // `~\scoop` without `%SCOOP%`.
        let user_scoop = r"C:\Users\u\scoop\apps\ludusavi\current\ludusavi.exe";
        assert_eq!(found(base().with_path(user_scoop)).as_deref(), Some(user_scoop));
        assert_eq!(found(base().with_path(package).with_path(global)).as_deref(), Some(package));
        assert_eq!(found(base().with_path(link).with_path(package)).as_deref(), Some(link));
        assert_eq!(found(base().with_path(unpacked).with_path(link)).as_deref(), Some(unpacked));
        // `PATH` (found as `ludusavi.exe`) comes first, the program set in Settings before it.
        let on_path = r"C:\Tools\ludusavi.exe";
        let probe = base().with_path(unpacked).with_bin("ludusavi", on_path);
        assert_eq!(resolve_binary(&probe, None).as_deref(), Some(on_path));
        let picked = r"E:\Apps\ludusavi.exe";
        assert_eq!(resolve_binary(&probe.clone().with_path(picked), Some(picked)).as_deref(), Some(picked));
        // Without `%LOCALAPPDATA%`: the home folder's.
        assert_eq!(resolve_binary(&FakeProbe::new(home).with_path(unpacked), None).as_deref(), Some(unpacked));
    }

    #[cfg(windows)]
    #[test]
    fn windows_config_is_in_appdata_or_next_to_a_portable_copy() {
        let probe = FakeProbe::new(r"C:\Users\u").with_env("APPDATA", r"C:\Users\u\AppData\Roaming");
        assert_eq!(windows_config_file(&probe, None), r"C:\Users\u\AppData\Roaming\ludusavi\config.yaml");
        assert_eq!(windows_config_file(&probe, Some(r"D:\Ludusavi\ludusavi.exe")), r"C:\Users\u\AppData\Roaming\ludusavi\config.yaml");
        let portable = probe.clone().with_path(r"D:\Ludusavi\ludusavi.portable");
        assert_eq!(windows_config_file(&portable, Some(r"D:\Ludusavi\ludusavi.exe")), r"D:\Ludusavi\config.yaml");
        assert_eq!(windows_config_file(&FakeProbe::new(r"C:\Users\u"), None), r"C:\Users\u\AppData\Roaming\ludusavi\config.yaml");
        // `config path` failed: the fallback is read.
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_string_lossy().into_owned();
        let exe = join(&dir, "ludusavi.exe");
        std::fs::write(join(&dir, "config.yaml"), "backup:\n  path: \"D:\\\\Game Saves\"\n").unwrap();
        let probe = FakeProbe::new(r"C:\Users\u").with_path(&exe).with_path(&join(&dir, "ludusavi.portable"));
        let det = detect_ludusavi(&Fake::default(), &probe, Some(&exe)).unwrap();
        assert_eq!(det.config_path, Some(join(&dir, "config.yaml")));
        assert_eq!(det.backup_path.as_deref(), Some(r"D:\Game Saves"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_in_the_config() {
        // What Ludusavi writes: forward slashes, or a picked folder with escaped backslashes.
        let yaml = "backup:\n  path: \"C:/Users/u/ludusavi-backup\"\nrestore:\n  path: \"D:\\\\Saves\\\\Old \\\"x\\\"\"\n";
        let (p, _, _) = parse_config(yaml, r"C:\Users\u");
        assert_eq!(p.as_deref(), Some("C:/Users/u/ludusavi-backup"));
        assert_eq!(parse_restore_path(yaml, r"C:\Users\u").as_deref(), Some(r#"D:\Saves\Old "x""#));
        assert_eq!(yaml_get("a: 'C:\\x'\n", &["a"]).as_deref(), Some(r"C:\x"));
        assert_eq!(unescape_double_quoted(r"C:\\a\b\/c"), r"C:\a\b/c");
    }
}
