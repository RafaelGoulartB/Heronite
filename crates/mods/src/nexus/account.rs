//! The user's Nexus account: the personal API key and who it belongs to.

use heronite_data::temps::write_atomic;
use serde::{Deserialize, Serialize};

use super::api;
use crate::http;
use crate::ModsRoot;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub name: String,
    pub user_id: u64,
    /// Premium accounts get direct download links from the API.
    pub is_premium: bool,
    pub is_supporter: bool,
    /// Profile picture URL.
    pub avatar: Option<String>,
}

/// What `<mods root>/nexus.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredAccount {
    pub api_key: String,
    pub account: Account,
    /// RFC 3339 of the last successful `validate`.
    pub validated_at: String,
}

impl StoredAccount {
    /// An account just validated, stamped with the current time.
    pub fn new(api_key: &str, account: Account) -> Self {
        Self { api_key: api_key.trim().to_string(), account, validated_at: http::now_iso() }
    }
}

/// `None` when there is no file, it is unreadable, or it holds no key.
pub fn load_account(root: &ModsRoot) -> Option<StoredAccount> {
    let bytes = std::fs::read(root.nexus_account()).ok()?;
    serde_json::from_slice::<StoredAccount>(&bytes).ok().filter(|a| !a.api_key.is_empty())
}

/// Written with mode `0600`: the key grants the account's API access.
pub fn save_account(root: &ModsRoot, account: &StoredAccount) -> std::io::Result<()> {
    let path = root.nexus_account();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(account).map_err(std::io::Error::other)?;
    write_atomic(&path, &json, Some(0o600))
}

pub fn forget_account(root: &ModsRoot) {
    let _ = std::fs::remove_file(root.nexus_account());
}

/// Asks Nexus who owns `api_key`.
pub fn validate(api_key: &str) -> Result<Account, String> {
    let raw: api::RawUser = api::rest(api_key.trim(), "/users/validate.json")?;
    Ok(raw.into())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn validate_answers_in_both_spellings() {
        let new: api::RawUser = serde_json::from_str(
            r#"{"user_id":1234,"key":"k","name":"Rafa","is_premium":true,"is_supporter":false,"email":"a@b.c","profile_url":"https://forums.nexusmods.com/uploads/profile/photo-1234.png"}"#,
        )
        .unwrap();
        let a = Account::from(new);
        assert_eq!((a.name.as_str(), a.user_id, a.is_premium, a.is_supporter), ("Rafa", 1234, true, false));
        assert_eq!(a.avatar.as_deref(), Some("https://forums.nexusmods.com/uploads/profile/photo-1234.png"));
        let old: api::RawUser =
            serde_json::from_str(r#"{"user_id":7,"name":"Old","is_premium?":false,"is_supporter?":true,"profile_url":""}"#).unwrap();
        let a = Account::from(old);
        assert_eq!((a.is_premium, a.is_supporter, a.avatar), (false, true, None));
    }

    #[test]
    #[ignore = "network"]
    fn live_a_wrong_key_is_refused() {
        let err = validate("not-a-key").unwrap_err();
        assert!(err.contains("API key"), "{err}");
    }

    #[test]
    fn the_account_file_is_private_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(dir.path().join("mods"));
        assert_eq!(load_account(&root), None);
        let stored = StoredAccount::new(" abc ", Account { name: "N".into(), user_id: 3, ..Default::default() });
        save_account(&root, &stored).unwrap();
        let mode = std::fs::metadata(root.nexus_account()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let back = load_account(&root).unwrap();
        assert_eq!((back.api_key.as_str(), back.account.user_id), ("abc", 3));
        assert!(!back.validated_at.is_empty());
        forget_account(&root);
        assert_eq!(load_account(&root), None);
    }
}
