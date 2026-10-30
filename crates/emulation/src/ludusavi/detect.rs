//! Finding the binary and reading Ludusavi's own `config.yaml`.

use heronite_launch::probe::Probe;
use serde::{Deserialize, Serialize};

use super::runner::CommandRunner;
use crate::pathutil::join;

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
                let rest = rest.strip_prefix(['\'', '"']).unwrap_or(rest);
                let rest = rest.strip_suffix(['\'', '"']).unwrap_or(rest);
                return Some(rest.to_string());
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
    let home = probe.home();
    [
        preferred.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string),
        probe.which("ludusavi"),
        Some("/usr/bin/ludusavi".into()),
        Some("/usr/local/bin/ludusavi".into()),
        Some(join(&home, ".local/bin/ludusavi")),
    ]
    .into_iter()
    .flatten()
    .find(|p| probe.exists(p))
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
        .unwrap_or_else(|| join(&home, ".config/ludusavi/config.yaml"));
    let exists = std::path::Path::new(&config_path).exists();
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
}
