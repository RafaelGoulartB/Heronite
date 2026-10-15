//! When a game's saves were last backed up by Ludusavi, read straight from
//! its backup folder. No process is spawned: the UI asks every time a game is
//! selected.
//!
//! Ludusavi keeps one folder per game under the backup root, with a
//! `mapping.yaml` (`name: ...`, `backups:` list, and the original save paths)
//! that it rewrites on every backup.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use heronite_data::DataPaths;

use super::detect::{parse_config, yaml_get};

/// The newest backup of a game.
#[derive(Debug, Clone, PartialEq)]
pub struct LastBackup {
    /// Ludusavi's name for the game (`name:` of `mapping.yaml`, else the folder name).
    pub game_name: String,
    /// The game's folder under the backup root.
    pub dir: PathBuf,
    /// Modified time of `mapping.yaml` (the folder when it is missing).
    pub at: SystemTime,
}

/// Looks up the backup folder of a game by title, then by Steam app id.
/// `None` when Ludusavi is off, has no backup root, or has no such game.
pub fn last_backup(paths: &DataPaths, title: &str, steam_app_id: Option<&str>) -> Option<LastBackup> {
    let settings = heronite_data::settings::load(paths).ok()?.ludusavi;
    if !settings.enabled {
        return None;
    }
    let home = std::env::var_os("HOME");
    let config = ludusavi_config_path(std::env::var_os("XDG_CONFIG_HOME"), home.as_deref());
    let root = backup_root(
        settings.use_installed_config,
        &settings.backup_path,
        config.as_deref(),
        &home.map(|h| h.to_string_lossy().into_owned()).unwrap_or_default(),
    )?;
    find_in(&root, title, steam_app_id)
}

/// `$XDG_CONFIG_HOME/ludusavi/config.yaml`, else `~/.config/ludusavi/config.yaml`.
pub(super) fn ludusavi_config_path(xdg_config_home: Option<OsString>, home: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|v| !v.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home?).join(".config"),
    };
    Some(base.join("ludusavi/config.yaml"))
}

/// The folder holding the per-game backup folders.
pub(super) fn backup_root(use_installed_config: bool, configured: &str, config_file: Option<&Path>, home: &str) -> Option<PathBuf> {
    let from_config = || {
        let yaml = fs::read_to_string(config_file?).ok()?;
        parse_config(&yaml, home).0.map(PathBuf::from)
    };
    let configured = || {
        let path = configured.trim();
        (!path.is_empty()).then(|| match path.strip_prefix("~/") {
            Some(rest) => Path::new(home).join(rest),
            None => PathBuf::from(path),
        })
    };
    if use_installed_config {
        from_config().or_else(configured)
    } else {
        configured().or_else(from_config)
    }
}

/// Lowercase alphanumerics only: "ELDEN RING" and "Elden Ring" match, and so
/// do "Onimusha: Way of the Sword" and the folder "Onimusha_ Way of the Sword".
fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn mapping_name(yaml: &str) -> Option<String> {
    yaml_get(yaml, &["name"]).map(|n| n.trim_matches('\'').to_owned()).filter(|n| !n.is_empty())
}

fn found(dir: PathBuf, name: Option<String>) -> LastBackup {
    let game_name = name.unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let at = heronite_data::json_store::mtime(&dir.join("mapping.yaml"))
        .or_else(|| heronite_data::json_store::mtime(&dir))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    LastBackup { game_name, dir, at }
}

/// The game folders of a backup root. Each `mapping.yaml` is read at most
/// once, however many titles are looked up (they can be large).
pub(super) struct RootFolders {
    dirs: Vec<(String, PathBuf)>,
    /// `compatdata/<id>/`, the Proton prefix the game's paths mention.
    proton: Option<String>,
    /// Per folder, read on the first lookup the folder names miss:
    /// `(mapping name, mentions the Proton prefix)`; `None` without mapping.
    mapped: Option<Vec<Option<(Option<String>, bool)>>>,
}

impl RootFolders {
    pub(super) fn read(root: &Path, steam_app_id: Option<&str>) -> Option<Self> {
        let mut dirs: Vec<(String, PathBuf)> = fs::read_dir(root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        dirs.sort();
        let proton = steam_app_id
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            .map(|id| format!("compatdata/{id}/"));
        Some(Self { dirs, proton, mapped: None })
    }

    /// The folder of `title`: by folder name, then by mapping name or by the
    /// game's Proton prefix.
    pub(super) fn find(&mut self, title: &str) -> Option<LastBackup> {
        let wanted = normalize(title);
        if !wanted.is_empty() {
            if let Some((_, dir)) = self.dirs.iter().find(|(name, _)| normalize(name) == wanted) {
                let name = fs::read_to_string(dir.join("mapping.yaml")).ok().as_deref().and_then(mapping_name);
                return Some(found(dir.clone(), name));
            }
        }
        let (dirs, proton) = (&self.dirs, &self.proton);
        let mapped = self.mapped.get_or_insert_with(|| {
            dirs.iter()
                .map(|(_, dir)| {
                    let yaml = fs::read_to_string(dir.join("mapping.yaml")).ok()?;
                    Some((mapping_name(&yaml), proton.as_ref().is_some_and(|p| yaml.contains(p.as_str()))))
                })
                .collect()
        });
        dirs.iter().zip(mapped.iter()).find_map(|((_, dir), m)| {
            let (name, in_prefix) = m.as_ref()?;
            let by_name = !wanted.is_empty() && name.as_ref().is_some_and(|n| normalize(n) == wanted);
            (by_name || *in_prefix).then(|| found(dir.clone(), name.clone()))
        })
    }
}

pub(super) fn find_in(root: &Path, title: &str, steam_app_id: Option<&str>) -> Option<LastBackup> {
    RootFolders::read(root, steam_app_id)?.find(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn game(root: &Path, folder: &str, yaml: Option<&str>) -> PathBuf {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        if let Some(yaml) = yaml {
            fs::write(dir.join("mapping.yaml"), yaml).unwrap();
        }
        dir
    }

    #[test]
    fn matches_folder_name_ignoring_case_and_punctuation() {
        let tmp = tempfile::tempdir().unwrap();
        game(tmp.path(), "Elden Ring", Some("---\nname: Elden Ring\nbackups:\n  - name: x\n"));
        game(tmp.path(), "Onimusha_ Way of the Sword", None);
        let b = find_in(tmp.path(), "ELDEN RING", None).unwrap();
        assert_eq!(b.game_name, "Elden Ring");
        assert_eq!(b.dir, tmp.path().join("Elden Ring"));
        let b = find_in(tmp.path(), "Onimusha: Way of the Sword", None).unwrap();
        assert_eq!(b.game_name, "Onimusha_ Way of the Sword");
        assert!(find_in(tmp.path(), "Control", None).is_none());
    }

    #[test]
    fn time_is_the_mapping_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = game(tmp.path(), "Game", Some("name: Game\n"));
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::File::options().write(true).open(dir.join("mapping.yaml")).unwrap().set_modified(when).unwrap();
        assert_eq!(find_in(tmp.path(), "Game", None).unwrap().at, when);
        fs::remove_file(dir.join("mapping.yaml")).unwrap();
        assert!(find_in(tmp.path(), "Game", None).unwrap().at > when);
    }

    #[test]
    fn falls_back_to_mapping_name_and_proton_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        game(tmp.path(), "Odd Folder", Some("name: Real Title\n"));
        game(tmp.path(), "Other", Some("name: Other\nfiles:\n  /s/steamapps/compatdata/12345/pfx/save.dat:\n    hash: a\n"));
        assert_eq!(find_in(tmp.path(), "real title", None).unwrap().dir, tmp.path().join("Odd Folder"));
        assert_eq!(find_in(tmp.path(), "Unrelated", Some("12345")).unwrap().game_name, "Other");
        assert!(find_in(tmp.path(), "Unrelated", Some("1234")).is_none());
        assert!(find_in(tmp.path(), "Unrelated", Some("../")).is_none());
        assert!(find_in(tmp.path(), "Unrelated", None).is_none());
    }

    #[test]
    fn missing_root_is_none() {
        assert!(find_in(Path::new("/nonexistent/ludusavi-root"), "Game", None).is_none());
    }

    #[test]
    fn config_path_follows_xdg_then_home() {
        let xdg = Some(OsString::from("/x"));
        let home = OsString::from("/h");
        assert_eq!(ludusavi_config_path(xdg, Some(&home)), Some(PathBuf::from("/x/ludusavi/config.yaml")));
        assert_eq!(ludusavi_config_path(None, Some(&home)), Some(PathBuf::from("/h/.config/ludusavi/config.yaml")));
        assert_eq!(ludusavi_config_path(Some(OsString::new()), Some(&home)), Some(PathBuf::from("/h/.config/ludusavi/config.yaml")));
        assert_eq!(ludusavi_config_path(None, None), None);
    }

    #[test]
    fn root_prefers_installed_config_when_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config.yaml");
        fs::write(&cfg, "backup:\n  path: ~/Saves\n").unwrap();
        let root = |installed, configured: &str, file: Option<&Path>| backup_root(installed, configured, file, "/home/u");
        assert_eq!(root(true, "/mine", Some(&cfg)), Some(PathBuf::from("/home/u/Saves")));
        assert_eq!(root(true, "/mine", None), Some(PathBuf::from("/mine")));
        assert_eq!(root(false, "/mine", Some(&cfg)), Some(PathBuf::from("/mine")));
        assert_eq!(root(false, "  ", Some(&cfg)), Some(PathBuf::from("/home/u/Saves")));
        assert_eq!(root(false, "~/B", None), Some(PathBuf::from("/home/u/B")));
        assert_eq!(root(true, "", None), None);
    }
}
