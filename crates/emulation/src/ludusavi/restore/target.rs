//! Where a restore writes: the game's prefix as it is now.

use std::fs;
use std::path::{Path, PathBuf};

/// The places a backup is put back into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreTarget {
    /// The active prefix (the folder holding `drive_c`): Steam's
    /// `compatdata/<id>/pfx` or the game's Wine prefix.
    pub prefix: Option<PathBuf>,
    /// The Wine user inside it (`steamuser` in Proton prefixes).
    pub user: Option<String>,
    /// The game's install folder (saves kept next to the game).
    pub install_dir: Option<PathBuf>,
    /// Steam's `userdata` folder, for Steam games.
    pub steam_userdata: Option<PathBuf>,
    /// The game runs through Wine / Proton. Without a prefix yet (never
    /// started), nothing can be restored.
    pub needs_prefix: bool,
}

impl RestoreTarget {
    /// A Wine / Proton game whose prefix does not exist yet.
    pub fn missing_prefix(&self) -> bool {
        self.needs_prefix && self.prefix.is_none()
    }
}

/// The user folder Wine made in `prefix/drive_c/users` (`Public` is shared):
/// `steamuser` when there (Proton), the only one, the login name, then the
/// first by name.
pub fn wine_user(prefix: &Path, login: Option<&str>) -> Option<String> {
    let mut users: Vec<String> = fs::read_dir(prefix.join("drive_c/users"))
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "Public" && n != "Default")
        .collect();
    users.sort();
    if users.iter().any(|u| u == "steamuser") {
        return Some("steamuser".into());
    }
    if users.len() == 1 {
        return users.pop();
    }
    if let Some(login) = login.filter(|l| users.iter().any(|u| u == l)) {
        return Some(login.to_string());
    }
    users.into_iter().next()
}

/// Whether Steam is running the game: its launch command carries
/// `AppId=<id>` (`reaper SteamLaunch AppId=<id> -- ...`).
pub fn steam_game_running(app_id: &str) -> bool {
    steam_game_running_in(Path::new("/proc"), app_id)
}

fn steam_game_running_in(proc_dir: &Path, app_id: &str) -> bool {
    if app_id.is_empty() {
        return false;
    }
    let needle = format!("AppId={app_id}");
    let Ok(entries) = fs::read_dir(proc_dir) else { return false };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .any(|e| fs::read(e.path().join("cmdline")).is_ok_and(|raw| raw.split(|b| *b == 0).any(|arg| arg == needle.as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_wine_user() {
        let d = tempfile::tempdir().unwrap();
        let users = d.path().join("drive_c/users");
        fs::create_dir_all(users.join("Public")).unwrap();
        assert_eq!(wine_user(d.path(), None), None);
        fs::create_dir_all(users.join("rafa")).unwrap();
        assert_eq!(wine_user(d.path(), None).as_deref(), Some("rafa"));
        fs::create_dir_all(users.join("alice")).unwrap();
        assert_eq!(wine_user(d.path(), Some("rafa")).as_deref(), Some("rafa"));
        assert_eq!(wine_user(d.path(), None).as_deref(), Some("alice"));
        fs::create_dir_all(users.join("steamuser")).unwrap();
        assert_eq!(wine_user(d.path(), Some("rafa")).as_deref(), Some("steamuser"));
        assert_eq!(wine_user(Path::new("/nonexistent"), None), None);
    }

    #[test]
    fn finds_steam_launches() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("42")).unwrap();
        fs::write(d.path().join("42/cmdline"), b"reaper\0SteamLaunch\0AppId=1245620\0--\0game.exe\0").unwrap();
        fs::create_dir_all(d.path().join("self")).unwrap();
        fs::write(d.path().join("self/cmdline"), b"AppId=7\0").unwrap();
        assert!(steam_game_running_in(d.path(), "1245620"));
        assert!(!steam_game_running_in(d.path(), "124562"));
        assert!(!steam_game_running_in(d.path(), "7"));
        assert!(!steam_game_running_in(d.path(), ""));
    }

    #[test]
    fn missing_prefix_only_for_wine_games() {
        assert!(RestoreTarget { needs_prefix: true, ..Default::default() }.missing_prefix());
        assert!(!RestoreTarget::default().missing_prefix());
        assert!(!RestoreTarget { needs_prefix: true, prefix: Some("/p".into()), ..Default::default() }.missing_prefix());
    }
}
