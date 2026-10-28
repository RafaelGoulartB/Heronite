//! What the sources without a store sign-in need: the RetroAchievements
//! account (the token its login gives, never the password) and an optional
//! Steam Web API key. Kept in
//! `local_library/achievement-accounts.json`, readable only by the user
//! (backups count it with the store sign-ins). Settings never shows the keys
//! back.

use anyhow::Result;
use heronite_data::paths::DataPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accounts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retro_achievements: Option<RetroAccount>,
    /// Lets Steam games never played on this PC be read through the Web API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_web_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetroAccount {
    pub username: String,
    /// The token of the login (`dorequest.php?r=login2`), the one emulators
    /// keep too: it reads the user's progress and signs the emulators in.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// Optional: the read-only Web API key of retroachievements.org →
    /// Settings → Keys, which adds the unlock times of past unlocks.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
}

impl RetroAccount {
    /// It can read the user's progress.
    pub fn usable(&self) -> bool {
        !self.username.is_empty() && (!self.token.is_empty() || !self.api_key.is_empty())
    }
}

/// The accounts; none when the file is missing. An error when it exists but
/// cannot be read or parsed.
pub fn try_read(paths: &DataPaths) -> Result<Accounts> {
    heronite_data::json_store::read(&paths.achievement_accounts())
}

/// [`try_read`] for display: no accounts when the file cannot be read.
/// [`write`] keeps such a file instead of replacing it.
pub fn read(paths: &DataPaths) -> Accounts {
    try_read(paths).unwrap_or_else(|e| {
        log::warn!("achievement accounts: {e:#}");
        Accounts::default()
    })
}

/// Replaces the file, readable by the user only. A file that cannot be parsed
/// is first renamed to `achievement-accounts.json.corrupt-<timestamp>`, and
/// one that cannot be read fails the write: the tokens in it are not lost.
pub fn write(paths: &DataPaths, accounts: &Accounts) -> Result<()> {
    let file = paths.achievement_accounts();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::store::keep_unreadable::<Accounts>(&file)?;
    let bytes = heronite_data::json_store::to_electron_store_bytes(accounts)?;
    heronite_data::temps::write_atomic(&file, &bytes, Some(0o600))?;
    Ok(())
}

/// Read-modify-write of the file. Fails, writing nothing, when the file exists
/// but cannot be read: the change would be applied to empty accounts.
pub fn update(paths: &DataPaths, f: impl FnOnce(&mut Accounts)) -> Result<Accounts> {
    let mut accounts = try_read(paths)?;
    f(&mut accounts);
    write(paths, &accounts)?;
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(read(&paths), Accounts::default());
        update(&paths, |a| a.retro_achievements = Some(RetroAccount { username: "me".into(), token: "t".into(), ..Default::default() }))
            .unwrap();
        assert_eq!(read(&paths).retro_achievements.unwrap().username, "me");
        let mode = std::fs::metadata(paths.achievement_accounts()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    fn corrupt_copies(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains(".corrupt-")).collect();
        found.sort();
        found
    }

    #[test]
    fn an_unparseable_file_is_not_updated_over() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = paths.achievement_accounts();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"retroAchievements\": {\"username\": \"me\", \"tok").unwrap();
        assert!(try_read(&paths).is_err());
        assert_eq!(read(&paths), Accounts::default());
        assert!(update(&paths, |a| a.steam_web_api_key = Some("k".into())).is_err());
        assert!(std::fs::read_to_string(&file).unwrap().contains("\"tok"), "the file is untouched");
    }

    #[test]
    fn writing_over_a_corrupt_file_keeps_it_aside() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = paths.achievement_accounts();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "not json {").unwrap();
        let accounts = Accounts { steam_web_api_key: Some("k".into()), ..Default::default() };
        write(&paths, &accounts).unwrap();
        assert_eq!(read(&paths), accounts);
        let kept = corrupt_copies(file.parent().unwrap());
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "not json {");
        // A valid file is rewritten in place.
        write(&paths, &Accounts::default()).unwrap();
        assert_eq!(corrupt_copies(file.parent().unwrap()).len(), 1);
    }

    #[test]
    fn an_unreadable_file_fails_the_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let file = paths.achievement_accounts();
        // A directory in place of the file: it exists, but reading it fails.
        std::fs::create_dir_all(&file).unwrap();
        assert!(try_read(&paths).is_err());
        assert!(write(&paths, &Accounts::default()).is_err());
        assert!(file.is_dir());
    }
}
