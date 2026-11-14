//! Reading and changing a Wine prefix's `user.reg` without Wine: the user's
//! folders (for save paths) and the value that enables the EOS overlay.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// The key of the user's folders in `user.reg`'s escaped form.
const SHELL_FOLDERS: &str = r"Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Shell Folders";

pub fn user_reg(prefix: &Path) -> PathBuf {
    prefix.join("user.reg")
}

/// The section name of a `[...] <time>` line.
fn section_of(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    Some(&rest[..rest.rfind(']')?])
}

/// The values of one key (section) of a `.reg` text: name → raw value
/// (quotes removed, `\\` kept as written).
fn section_values(text: &str, key: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(section) = section_of(line) {
            inside = section.eq_ignore_ascii_case(key);
            continue;
        }
        if !inside || !line.starts_with('"') {
            continue;
        }
        if let Some((name, value)) = line.split_once("\"=") {
            out.insert(name.trim_start_matches('"').to_string(), value.trim().trim_matches('"').to_string());
        }
    }
    out
}

/// A Windows path of the prefix (`C:\\users\\...`) as a path on this system.
pub fn to_host_path(prefix: &Path, windows_path: &str) -> PathBuf {
    let p = windows_path.replace(r"\\", "/").replace('\\', "/");
    if let Some(rest) = p.strip_prefix("C:/").or_else(|| p.strip_prefix("c:/")) {
        return prefix.join("drive_c").join(rest);
    }
    if let Some(rest) = p.strip_prefix("Z:").or_else(|| p.strip_prefix("z:")) {
        return PathBuf::from(rest);
    }
    prefix.join("drive_c").join(p)
}

/// The user's folders of the prefix (`AppData`, `Local AppData`, `Personal`,
/// the Saved Games GUID ...) as paths on this system (legendary's
/// `get_shell_folders`).
pub fn shell_folders(prefix: &Path) -> Result<HashMap<String, PathBuf>> {
    let path = user_reg(prefix);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let values = section_values(&text, SHELL_FOLDERS);
    if values.is_empty() {
        bail!("{} has no user folders", path.display());
    }
    Ok(values.into_iter().map(|(k, v)| (k, to_host_path(prefix, &v))).collect())
}

/// The value `name` of `key` (escaped form, as in the file), unescaped.
pub fn value(prefix: &Path, key: &str, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(user_reg(prefix)).ok()?;
    section_values(&text, key).get(name).map(|v| v.replace(r"\\", "\\"))
}

/// Sets the string `name` of `key` (escaped form) in `user.reg`, adding the
/// key when it is missing (legendary's way for the EOS overlay: one line,
/// the rest of the file untouched).
pub fn set_string(prefix: &Path, key: &str, name: &str, value: &str) -> Result<()> {
    let path = user_reg(prefix);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let escaped = value.replace('\\', r"\\");
    let line = format!("\"{name}\"=\"{escaped}\"");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut section_at = None;
    let mut done = false;
    let mut inside = false;
    for (i, l) in lines.iter_mut().enumerate() {
        if let Some(section) = section_of(l) {
            inside = section.eq_ignore_ascii_case(key);
            if inside {
                section_at = Some(i);
            }
            continue;
        }
        if inside && l.starts_with(&format!("\"{name}\"=")) {
            *l = line.clone();
            done = true;
            break;
        }
    }
    if !done {
        match section_at {
            Some(i) => lines.insert(i + 1, line),
            None => {
                lines.push(String::new());
                lines.push(format!("[{key}]"));
                lines.push(line);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    heronite_data::temps::write_atomic(&path, out.as_bytes(), None).with_context(|| format!("writing {}", path.display()))
}

/// Removes the value `name` of `key` (escaped form) from `user.reg`.
pub fn remove_value(prefix: &Path, key: &str, name: &str) -> Result<bool> {
    let path = user_reg(prefix);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut inside = false;
    let mut removed = false;
    let mut kept = Vec::new();
    for l in text.lines() {
        if let Some(section) = section_of(l) {
            inside = section.eq_ignore_ascii_case(key);
        } else if inside && l.starts_with(&format!("\"{name}\"=")) {
            removed = true;
            continue;
        }
        kept.push(l);
    }
    if removed {
        let mut out = kept.join("\n");
        out.push('\n');
        heronite_data::temps::write_atomic(&path, out.as_bytes(), None)?;
    }
    Ok(removed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const USER_REG: &str = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\\\\S-1-5-21\n\n\
[Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Explorer\\\\Shell Folders] 1792530491\n#time=1dd60d71cecf850\n\
\"AppData\"=\"C:\\\\users\\\\steamuser\\\\AppData\\\\Roaming\"\n\
\"Local AppData\"=\"C:\\\\users\\\\steamuser\\\\AppData\\\\Local\"\n\
\"Personal\"=\"C:\\\\users\\\\steamuser\\\\Documents\"\n\
\"{4C5C32FF-BB9D-43B0-B5B4-2D72E54EAAA4}\"=\"C:\\\\users\\\\steamuser\\\\Saved Games\"\n\
\"{A520A1A4-1780-4FF6-BD18-167343C5AF16}\"=\"C:\\\\users\\\\steamuser\\\\AppData\\\\LocalLow\"\n\n\
[Software\\\\Wine] 1\n\"Version\"=\"win10\"\n";

    #[test]
    fn user_folders_are_found_in_the_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(user_reg(dir.path()), USER_REG).unwrap();
        let f = shell_folders(dir.path()).unwrap();
        assert_eq!(f["Local AppData"], dir.path().join("drive_c/users/steamuser/AppData/Local"));
        assert_eq!(f["{4C5C32FF-BB9D-43B0-B5B4-2D72E54EAAA4}"], dir.path().join("drive_c/users/steamuser/Saved Games"));
        assert_eq!(value(dir.path(), r"Software\\Wine", "Version").as_deref(), Some("win10"));
    }

    #[test]
    fn values_are_set_and_removed_in_place() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(user_reg(dir.path()), USER_REG).unwrap();
        let key = r"Software\\Epic Games\\EOS";
        set_string(dir.path(), key, "OverlayPath", "Z:/home/u/overlay").unwrap();
        assert_eq!(value(dir.path(), key, "OverlayPath").as_deref(), Some("Z:/home/u/overlay"));
        set_string(dir.path(), key, "OverlayPath", "C:\\eos").unwrap();
        let text = std::fs::read_to_string(user_reg(dir.path())).unwrap();
        assert_eq!(text.matches("OverlayPath").count(), 1);
        assert!(text.contains("\"OverlayPath\"=\"C:\\\\eos\""));
        assert!(text.contains("\"Version\"=\"win10\""));
        assert!(remove_value(dir.path(), key, "OverlayPath").unwrap());
        assert!(!remove_value(dir.path(), key, "OverlayPath").unwrap());
        assert_eq!(value(dir.path(), key, "OverlayPath"), None);
        assert_eq!(to_host_path(dir.path(), "Z:/x/y"), PathBuf::from("/x/y"));
    }
}
