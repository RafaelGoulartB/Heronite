//! The packages of the Microsoft Store (MSIX / AppX) registered for this
//! user: the Store's apps and the games the Xbox app installs.
//!
//! They are read from the registry the deployment service keeps per user
//! (`AppModel\Repository\Packages` of `HKCU`, one key per package with its
//! folder, its display name and a key per application), so nothing is
//! started and no WinRT runtime is needed. Other systems have none.

use std::path::PathBuf;

/// A package registered for this user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// `Microsoft.Name_1.2.3.0_x64__8wekyb3d8bbwe`: name, version,
    /// architecture, resource id (often empty) and the publisher's id.
    pub full_name: String,
    /// Where its files are (`PackageRootFolder`): a folder of
    /// `C:\Program Files\WindowsApps`, or `<drive>\XboxGames\<game>\Content`
    /// for the games the Xbox app installs.
    pub root: PathBuf,
    /// `DisplayName`: plain text, or an `ms-resource:` reference that
    /// [`resolve`] turns into text.
    pub display_name: String,
    /// The ids of its applications (`App`, `Game`): what follows the `!` of
    /// an application's AUMID.
    pub apps: Vec<String>,
}

impl Package {
    /// `Microsoft.Name` (package names have no `_`).
    pub fn name(&self) -> &str {
        name_of(&self.full_name)
    }

    /// `Microsoft.Name_8wekyb3d8bbwe`: the name and the publisher's id, the
    /// same for every version of the package.
    pub fn family_name(&self) -> String {
        family_of(&self.full_name)
    }
}

/// `Microsoft.Name` of a package's full name.
pub fn name_of(full_name: &str) -> &str {
    full_name.split('_').next().unwrap_or(full_name)
}

/// The family name of a package's full name: its first and last parts.
pub fn family_of(full_name: &str) -> String {
    let mut parts = full_name.split('_');
    let name = parts.next().unwrap_or_default();
    match parts.next_back() {
        Some(publisher) if !publisher.is_empty() => format!("{name}_{publisher}"),
        _ => name.to_string(),
    }
}

/// The packages registered for this user whose folder is there, by full
/// name. Empty on other systems and when the registry cannot be read.
pub fn installed() -> Vec<Package> {
    imp::installed()
}

/// The text of a package's display name: plain text as it is, an
/// `ms-resource:` reference looked up in the package's resources (in the
/// language of the user). `None` when it cannot be found.
pub fn resolve(full_name: &str, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if !text.to_ascii_lowercase().starts_with("ms-resource:") {
        return Some(text.to_string());
    }
    resource_uris(name_of(full_name), text)
        .into_iter()
        .find_map(|uri| imp::indirect_string(&format!("@{{{full_name}? {uri}}}")))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.to_ascii_lowercase().starts_with("ms-resource:"))
}

/// Where an `ms-resource:` reference of package `name` may point, the
/// likeliest first: a bare key (`ms-resource:AppName`) is in the package's
/// `Resources` map, a path (`ms-resource:/Strings/AppName`) starts at the
/// package, a full URI is kept.
fn resource_uris(name: &str, text: &str) -> Vec<String> {
    let key = &text["ms-resource:".len()..];
    if key.starts_with("//") {
        return vec![format!("ms-resource:{key}")];
    }
    let key = key.trim_start_matches('/');
    if key.contains('/') {
        let mut out = vec![format!("ms-resource://{name}/{key}")];
        if !key.to_ascii_lowercase().starts_with("resources/") {
            out.push(format!("ms-resource://{name}/Resources/{key}"));
        }
        out
    } else {
        vec![format!("ms-resource://{name}/Resources/{key}"), format!("ms-resource://{name}/{key}")]
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    use windows_sys::Win32::UI::Shell::SHLoadIndirectString;

    use super::Package;
    use crate::registry::{self, Hive};

    const PACKAGES: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

    pub fn installed() -> Vec<Package> {
        registry::subkeys(Hive::CurrentUser, PACKAGES)
            .into_iter()
            .filter_map(|full_name| {
                let key = format!(r"{PACKAGES}\{full_name}");
                let root = registry::string(Hive::CurrentUser, &key, "PackageRootFolder").map(PathBuf::from)?;
                if !root.is_dir() {
                    return None;
                }
                let display_name = registry::string(Hive::CurrentUser, &key, "DisplayName").unwrap_or_default();
                let apps = registry::subkeys(Hive::CurrentUser, &key);
                Some(Package { full_name, root, display_name, apps })
            })
            .collect()
    }

    /// `SHLoadIndirectString` of `source` (`@{<full name>? ms-resource://...}`).
    pub fn indirect_string(source: &str) -> Option<String> {
        let source: Vec<u16> = OsStr::new(source).encode_wide().chain([0]).collect();
        let mut buf = vec![0u16; 1024];
        // SAFETY: `source` is NUL-terminated; `buf` holds `buf.len()` characters.
        let hr = unsafe { SHLoadIndirectString(source.as_ptr(), buf.as_mut_ptr(), buf.len() as u32, std::ptr::null()) };
        if hr != 0 {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Package;

    pub fn installed() -> Vec<Package> {
        Vec::new()
    }

    pub fn indirect_string(_source: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_full_name() {
        let full = "Microsoft.624F8B84B80_1.2.3.0_x64__8wekyb3d8bbwe";
        assert_eq!(name_of(full), "Microsoft.624F8B84B80");
        assert_eq!(family_of(full), "Microsoft.624F8B84B80_8wekyb3d8bbwe");
        assert_eq!(family_of("Publisher.Game_1.0.0.0_neutral_split.scale-100_abcdefghjkmnp"), "Publisher.Game_abcdefghjkmnp");
        assert_eq!(family_of("Odd"), "Odd");
    }

    #[test]
    fn resource_references_point_into_the_package() {
        assert_eq!(
            resource_uris("Microsoft.WindowsCalculator", "ms-resource:AppStoreName"),
            ["ms-resource://Microsoft.WindowsCalculator/Resources/AppStoreName", "ms-resource://Microsoft.WindowsCalculator/AppStoreName"]
        );
        assert_eq!(
            resource_uris("Game", "ms-resource:/Strings/Title"),
            ["ms-resource://Game/Strings/Title", "ms-resource://Game/Resources/Strings/Title"]
        );
        assert_eq!(resource_uris("Game", "ms-resource:Resources/Title"), ["ms-resource://Game/Resources/Title"]);
        assert_eq!(resource_uris("Game", "ms-resource://Other/Resources/Title"), ["ms-resource://Other/Resources/Title"]);
    }

    #[test]
    fn plain_names_need_no_lookup() {
        assert_eq!(resolve("Game_1.0.0.0_x64__abc", "  Forza Horizon 5 "), Some("Forza Horizon 5".into()));
        assert_eq!(resolve("Game_1.0.0.0_x64__abc", ""), None);
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_packages_of_this_user() {
        // Every Windows 10 / 11 has packages; their folders are there.
        let packages = installed();
        assert!(!packages.is_empty());
        assert!(packages.iter().all(|p| p.root.is_dir() && !p.full_name.is_empty()));
        // An unknown package has no resources.
        assert_eq!(resolve("Heronite.Missing_1.0.0.0_x64__abc", "ms-resource:AppName"), None);
    }
}
