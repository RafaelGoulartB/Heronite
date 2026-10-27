//! Wine's registry files (`system.reg`, `user.reg`): the text format of
//! `server/registry.c`, read without Wine. Only the keys under the asked
//! paths are kept, so a 4 MB `system.reg` costs one pass and a few small
//! allocations.
//!
//! ```text
//! [Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs\\635] 1792274630
//! #time=1dd5e836462b4c8
//! "InstallDir"="C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Far Cry 3/"
//! "Size"=dword:0000000a
//! ```

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    /// `REG_SZ` and `REG_EXPAND_SZ` (`str(2):`).
    Str(String),
    Dword(u32),
    /// Binary, multi-string and the other types: not read.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegKey {
    /// The key's path relative to the file's root, single backslashes
    /// (`Software\Wow6432Node\Ubisoft\Launcher\Installs\635`).
    pub path: String,
    /// Values by lowercase name; the default value is `""`.
    pub values: BTreeMap<String, RegValue>,
}

impl RegKey {
    /// The last part of the path (`635`).
    pub fn name(&self) -> &str {
        self.path.rsplit('\\').next().unwrap_or(&self.path)
    }

    /// A string value (case-insensitive name), trimmed; `None` when empty.
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.values.get(&name.to_lowercase()) {
            Some(RegValue::Str(s)) if !s.trim().is_empty() => Some(s.trim()),
            _ => None,
        }
    }

    pub fn dword(&self, name: &str) -> Option<u32> {
        match self.values.get(&name.to_lowercase()) {
            Some(RegValue::Dword(n)) => Some(*n),
            _ => None,
        }
    }

    /// Whether this key is `parent` itself or directly below it
    /// (case-insensitive).
    pub fn is_child_of(&self, parent: &str) -> bool {
        let (path, parent) = (self.path.to_lowercase(), parent.to_lowercase());
        path.strip_prefix(&parent).and_then(|rest| rest.strip_prefix('\\')).is_some_and(|rest| !rest.is_empty() && !rest.contains('\\'))
    }
}

/// The keys of `file` whose path starts with one of `under` (single
/// backslashes, case-insensitive), in file order. A missing file has none.
pub fn read_keys(file: &Path, under: &[&str]) -> std::io::Result<Vec<RegKey>> {
    match std::fs::read(file) {
        Ok(bytes) => Ok(parse_keys(&String::from_utf8_lossy(&bytes), under)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// [`read_keys`] on the text of a registry file.
pub fn parse_keys(text: &str, under: &[&str]) -> Vec<RegKey> {
    let wanted: Vec<String> = under.iter().map(|u| u.trim_end_matches('\\').to_lowercase()).collect();
    let matches = |path: &str| {
        let lower = path.to_lowercase();
        wanted.iter().any(|w| lower == *w || lower.strip_prefix(w.as_str()).is_some_and(|r| r.starts_with('\\')))
    };
    let mut out: Vec<RegKey> = Vec::new();
    let mut current: Option<RegKey> = None;
    let mut continued = false;
    for line in text.lines() {
        // The rest of a long hex value.
        if continued {
            continued = line.trim_end().ends_with('\\');
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            out.extend(current.take());
            let Some(end) = key_end(rest) else { continue };
            let path = unescape(&rest[..end]);
            if matches(&path) {
                current = Some(RegKey { path, values: BTreeMap::new() });
            }
            continue;
        }
        let Some(key) = current.as_mut() else {
            continue;
        };
        let (name, value) = if let Some(rest) = line.strip_prefix('@') {
            (String::new(), rest.strip_prefix('='))
        } else if line.starts_with('"') {
            match string_end(&line[1..]) {
                Some(end) => (unescape(&line[1..1 + end]).to_lowercase(), line[2 + end..].strip_prefix('=')),
                None => continue,
            }
        } else {
            continue;
        };
        let Some(value) = value else { continue };
        if value.starts_with("hex") {
            continued = value.trim_end().ends_with('\\');
        }
        key.values.insert(name, parse_value(value));
    }
    out.extend(current);
    out
}

fn parse_value(v: &str) -> RegValue {
    let string = v.strip_prefix("str(2):").unwrap_or(v);
    if let Some(body) = string.strip_prefix('"') {
        let end = string_end(body).unwrap_or(body.len());
        return RegValue::Str(unescape(&body[..end]));
    }
    if let Some(hex) = v.strip_prefix("dword:") {
        return u32::from_str_radix(hex.trim(), 16).map_or(RegValue::Other, RegValue::Dword);
    }
    RegValue::Other
}

/// Where the key name of `[name] time` ends (the first `]` not escaped).
fn key_end(s: &str) -> Option<usize> {
    closing(s, b']')
}

/// Where a quoted string ends (the first `"` not escaped).
fn string_end(s: &str) -> Option<usize> {
    closing(s, b'"')
}

fn closing(s: &str, close: u8) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == close => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Undoes Wine's `dump_strW`: `\\`, `\"`, C escapes, `\x` with up to four
/// hex digits (UTF-16 code units) and `\` with up to three octal digits.
pub fn unescape(s: &str) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
            continue;
        }
        let Some(e) = chars.next() else {
            units.push(u16::from(b'\\'));
            break;
        };
        let unit = match e {
            'x' => {
                let mut n: u32 = 0;
                let mut digits = 0;
                while digits < 4 {
                    match chars.peek().and_then(|c| c.to_digit(16)) {
                        Some(d) => {
                            n = n * 16 + d;
                            chars.next();
                            digits += 1;
                        }
                        None => break,
                    }
                }
                if digits == 0 {
                    u16::from(b'x')
                } else {
                    n as u16
                }
            }
            '0'..='7' => {
                let mut n = e.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            n = n * 8 + d;
                            chars.next();
                        }
                        None => break,
                    }
                }
                n as u16
            }
            'n' => 10,
            'r' => 13,
            't' => 9,
            'a' => 7,
            'b' => 8,
            'e' => 27,
            'f' => 12,
            'v' => 11,
            other => {
                let mut buf = [0u16; 2];
                units.extend_from_slice(other.encode_utf16(&mut buf));
                continue;
            }
        };
        units.push(unit);
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"WINE REGISTRY Version 2
;; All keys relative to REGISTRY\\Machine

#arch=win64

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install 635] 1792274630
#time=1dd5e836462b4c8
"DisplayIcon"="C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\games\\Far Cry 3\\FC3.ico"
"DisplayName"="Far Cry\xae 3"
"EstimatedSize"=dword:0000000a
"Blob"=hex:01,02,03,\
  04,05
"InstallLocation"="C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\games\\Far Cry 3\\"

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Wine Mono] 1792274630
@="default"
"DisplayName"="Wine Mono \"Windows\" Support"
"Path"=str(2):"%ProgramFiles%\\Mono"

[Software\\Wine\\Drives] 1792274630
"c:"="hd"
"#;

    #[test]
    fn reads_the_keys_under_a_path_with_their_values() {
        let keys = parse_keys(SAMPLE, &["Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"]);
        assert_eq!(keys.len(), 2);
        let fc3 = &keys[0];
        assert_eq!(fc3.name(), "Uplay Install 635");
        assert_eq!(fc3.str("displayname"), Some("Far Cry® 3"));
        assert_eq!(fc3.dword("EstimatedSize"), Some(10));
        assert_eq!(fc3.values.get("blob"), Some(&RegValue::Other));
        // The value after a continued hex value is still read.
        assert_eq!(fc3.str("InstallLocation"), Some("C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher\\games\\Far Cry 3\\"));
        let mono = &keys[1];
        assert_eq!(mono.str(""), Some("default"));
        assert_eq!(mono.str("DisplayName"), Some("Wine Mono \"Windows\" Support"));
        assert_eq!(mono.str("Path"), Some("%ProgramFiles%\\Mono"));
        assert!(mono.is_child_of("software\\microsoft\\windows\\currentversion\\uninstall"));
        assert!(!mono.is_child_of("Software\\Microsoft"));
    }

    #[test]
    fn a_path_that_only_starts_alike_is_not_under_it() {
        assert!(parse_keys(SAMPLE, &["Software\\Win"]).is_empty());
        assert_eq!(parse_keys(SAMPLE, &["software\\wine\\"]).len(), 1);
    }

    #[test]
    fn unescapes_what_wine_escapes() {
        assert_eq!(unescape(r#"a\\b\"c"#), "a\\b\"c");
        assert_eq!(unescape(r"\x00e9t\xe9"), "été");
        assert_eq!(unescape(r"caf\x00e9s"), "cafés");
        assert_eq!(unescape(r"tab\there\n"), "tab\there\n");
        assert_eq!(unescape(r"\0"), "\0");
        assert_eq!(unescape(r"\x30c4\x30fc"), "ツー");
    }

    #[test]
    fn a_missing_file_has_no_keys() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_keys(&dir.path().join("system.reg"), &["Software"]).unwrap().is_empty());
    }
}
