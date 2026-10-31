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

pub fn read(paths: &DataPaths) -> Accounts {
    heronite_data::json_store::read(&paths.achievement_accounts()).unwrap_or_default()
}

/// Replaces the file, readable by the user only.
pub fn write(paths: &DataPaths, accounts: &Accounts) -> Result<()> {
    let file = paths.achievement_accounts();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let bytes = heronite_data::json_store::to_electron_store_bytes(accounts)?;
    heronite_data::temps::write_atomic(&file, &bytes, Some(0o600))?;
    Ok(())
}

/// Read-modify-write of the file.
pub fn update(paths: &DataPaths, f: impl FnOnce(&mut Accounts)) -> Result<Accounts> {
    let mut accounts = read(paths);
    f(&mut accounts);
    write(paths, &accounts)?;
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(read(&paths), Accounts::default());
        update(&paths, |a| a.retro_achievements = Some(RetroAccount { username: "me".into(), token: "t".into(), ..Default::default() }))
            .unwrap();
        assert_eq!(read(&paths).retro_achievements.unwrap().username, "me");
        #[cfg(unix)]
        assert_eq!(heronite_platform::fs::mode(&std::fs::metadata(paths.achievement_accounts()).unwrap()).unwrap() & 0o777, 0o600);
    }
}
