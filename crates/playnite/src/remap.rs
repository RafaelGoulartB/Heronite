//! Windows drive -> Linux path remapping: port of `playnite/path-remap.ts`.

use std::collections::BTreeSet;

use heronite_launch::probe::Probe;
use serde::{Deserialize, Serialize};

/// `DriveRemap`: `windowsRoot` like `D:\` (or `D:`), `linuxPath` like
/// `/mnt/games`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveRemap {
    pub windows_root: String,
    pub linux_path: String,
}

/// `^([A-Za-z]:)(.*)$` on an already trimmed string: `(drive, rest)`.
fn split_drive(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        let rest = &s[2..];
        // `.` does not match line terminators in the JS regex.
        if rest.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
            return None;
        }
        Some((&s[..2], rest))
    } else {
        None
    }
}

/// `extractWindowsDrive`: `C:` upper-cased.
pub fn extract_windows_drive(path: Option<&str>) -> Option<String> {
    let path = path?;
    split_drive(path.trim()).map(|(d, _)| d.to_uppercase())
}

/// `collectWindowsDrives`: unique, sorted.
pub fn collect_windows_drives<'a>(paths: impl IntoIterator<Item = Option<&'a str>>) -> Vec<String> {
    let set: BTreeSet<String> = paths.into_iter().filter_map(extract_windows_drive).collect();
    set.into_iter().collect()
}

/// `remapWindowsPath`. `None` for an empty path or a drive without mapping.
pub fn remap_windows_path(path: Option<&str>, drive_map: &[DriveRemap]) -> Option<String> {
    let trimmed = path?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        return Some(trimmed.to_owned());
    }
    let Some((drive, rest)) = split_drive(trimmed) else {
        return Some(trimmed.replace('\\', "/"));
    };
    let drive = drive.to_uppercase();
    let rest = rest.replace('\\', "/");
    let mapping = drive_map
        .iter()
        .find(|item| item.windows_root.trim_end_matches('\\').to_uppercase() == drive)?;
    if mapping.linux_path.is_empty() {
        return None;
    }
    let base = mapping.linux_path.trim_end_matches('/');
    Some(format!("{base}{rest}"))
}

/// `expandPlayniteVariables`: replaces `{Key}` for every non-empty value, in
/// order.
pub fn expand_playnite_variables(value: Option<&str>, vars: &[(&str, Option<&str>)]) -> Option<String> {
    let value = value.filter(|v| !v.is_empty())?;
    let mut result = value.to_owned();
    for (key, replacement) in vars {
        let Some(replacement) = replacement.filter(|r| !r.is_empty()) else { continue };
        result = result.replace(&format!("{{{key}}}"), replacement);
    }
    Some(result)
}

/// `pathExists`.
pub fn path_exists(probe: &dyn Probe, path: Option<&str>) -> bool {
    path.is_some_and(|p| !p.is_empty() && probe.exists(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Vec<DriveRemap> {
        vec![
            DriveRemap { windows_root: "D:\\".into(), linux_path: "/mnt/games/".into() },
            DriveRemap { windows_root: "e:".into(), linux_path: "".into() },
        ]
    }

    #[test]
    fn remaps_drives() {
        let m = map();
        assert_eq!(remap_windows_path(Some("D:\\Games\\A\\a.exe"), &m).unwrap(), "/mnt/games/Games/A/a.exe");
        assert_eq!(remap_windows_path(Some(" d:\\x "), &m).unwrap(), "/mnt/games/x");
        assert_eq!(remap_windows_path(Some("E:\\x"), &m), None);
        assert_eq!(remap_windows_path(Some("F:\\x"), &m), None);
        assert_eq!(remap_windows_path(Some("/opt/x"), &m).unwrap(), "/opt/x");
        assert_eq!(remap_windows_path(Some("~/x"), &m).unwrap(), "~/x");
        assert_eq!(remap_windows_path(Some("rel\\path"), &m).unwrap(), "rel/path");
        assert_eq!(remap_windows_path(Some("  "), &m), None);
        assert_eq!(remap_windows_path(None, &m), None);
        assert_eq!(remap_windows_path(Some("D:"), &m).unwrap(), "/mnt/games");
    }

    #[test]
    fn detects_drives() {
        let d = collect_windows_drives([Some("d:\\a"), Some("C:/b"), None, Some("/x"), Some("D:\\z")]);
        assert_eq!(d, ["C:", "D:"]);
    }

    #[test]
    fn expands_variables_in_order() {
        let v = expand_playnite_variables(
            Some("{InstallDir}\\a.exe {Name}"),
            &[("InstallDir", Some("/g")), ("Name", Some("N")), ("Other", None)],
        );
        assert_eq!(v.unwrap(), "/g\\a.exe N");
        assert_eq!(expand_playnite_variables(Some(""), &[]), None);
    }
}
