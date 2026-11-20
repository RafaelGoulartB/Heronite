//! A Steam sign-in saved by an earlier Heronite (the QR code of
//! [`auth`]), which signed in as the Steam mobile app. Heronite no longer
//! signs in to Steam; at start it ends a sign-in it finds
//! ([`sign_out_on_steam`]), so its tokens stop working and it leaves the
//! account's authorized devices, and deletes it.
//!
//! The sign-in was kept in `local_library/steam-account.json`, readable only
//! by the user (on Windows `steam-account.bin`, sealed for this Windows user
//! with `heronite_platform::secret`).

pub mod auth;
pub mod http;

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};

pub use auth::SignedOut;
use http::Http;

/// A saved sign-in.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// SteamID64.
    pub steam_id: String,
    /// The sign-in name (not the profile name), as Steam gave it.
    #[serde(default)]
    pub account_name: String,
    pub refresh_token: String,
    #[serde(default)]
    pub access_token: String,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account").field("steam_id", &self.steam_id).field("account_name", &self.account_name).finish_non_exhaustive()
    }
}

fn path(paths: &DataPaths) -> PathBuf {
    paths.local_meta().with_file_name(if cfg!(windows) { "steam-account.bin" } else { "steam-account.json" })
}

/// The sign-in saved here; `None` when none is, or the file cannot be read
/// (another Windows user's).
fn load(paths: &DataPaths) -> Option<Account> {
    let bytes = std::fs::read(path(paths)).ok()?;
    let plain = if cfg!(windows) {
        match heronite_platform::secret::unprotect(&bytes) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("steam: the saved sign-in cannot be opened ({e}); signed out");
                return None;
            }
        }
    } else {
        bytes
    };
    serde_json::from_slice(&plain).ok()
}

#[cfg(test)]
fn save(paths: &DataPaths, account: &Account) -> Result<()> {
    use anyhow::Context;
    let json = serde_json::to_vec_pretty(account)?;
    let bytes = if cfg!(windows) { heronite_platform::secret::protect(&json).context("sealing the Steam sign-in")? } else { json };
    let file = path(paths);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    heronite_data::temps::write_atomic(&file, &bytes, Some(0o600)).with_context(|| format!("writing {}", file.display()))
}

/// Forgets the sign-in here only.
fn sign_out(paths: &DataPaths) -> Result<()> {
    let file = path(paths);
    match std::fs::remove_file(&file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(anyhow!(e).context(format!("deleting {}", file.display()))),
        _ => Ok(()),
    }
}

/// Whether a sign-in is saved here (whether it can be opened or not).
pub fn saved(paths: &DataPaths) -> bool {
    path(paths).exists()
}

/// Signs out: ends the sign-in on Steam, so its tokens stop working and it
/// leaves the account's authorized devices, then forgets it here, whatever
/// Steam answered. `Ok(true)`: Steam no longer takes it; `Ok(false)`: only
/// forgotten here (Steam out of reach or refusing, or no saved sign-in this
/// user can open), the user removes it on Steam's site.
pub fn sign_out_on_steam(paths: &DataPaths, http: &dyn Http) -> Result<bool> {
    let ended = match load(paths) {
        Some(account) => match end_on_steam(http, &account) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("steam: the sign-in could not be ended on Steam ({e:#}); forgotten here only");
                false
            }
        },
        None => false,
    };
    sign_out(paths)?;
    Ok(ended)
}

fn end_on_steam(http: &dyn Http, account: &Account) -> Result<()> {
    if fresh(&account.access_token, chrono::Utc::now().timestamp()) {
        return auth::revoke(http, &account.access_token, &account.refresh_token);
    }
    // Not saved: the sign-in is about to be forgotten.
    match auth::renew_access(http, &account.refresh_token, &account.steam_id) {
        // Should Steam still replace the refresh token, the new one is the live one.
        Ok(r) => auth::revoke(http, &r.access_token, r.refresh_token.as_deref().unwrap_or(&account.refresh_token)),
        // Steam no longer takes it: nothing left to end.
        Err(e) if e.is::<SignedOut>() => Ok(()),
        Err(e) => Err(e),
    }
}

/// Whether the access token works for a while yet (a minute of margin).
fn fresh(token: &str, now: i64) -> bool {
    auth::claims(token).is_some_and(|c| c.expires > now + 60)
}

#[cfg(test)]
mod tests {
    use super::auth::tests::jwt;
    use super::http::Fake;
    use super::*;

    const ID: &str = "76561197960287930";

    fn later() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    #[test]
    fn a_saved_sign_in_is_this_user_s_only_and_its_tokens_never_logged() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(!saved(&paths));
        let account = Account { steam_id: ID.into(), account_name: "gaben".into(), refresh_token: "R".into(), access_token: "A".into() };
        save(&paths, &account).unwrap();
        assert!(saved(&paths));
        assert_eq!(load(&paths), Some(account));
        #[cfg(unix)]
        assert_eq!(heronite_platform::fs::mode(&std::fs::metadata(path(&paths)).unwrap()).unwrap() & 0o777, 0o600);
        assert!(!format!("{:?}", load(&paths).unwrap()).contains("refresh"));
        sign_out(&paths).unwrap();
        assert_eq!(load(&paths), None);
        sign_out(&paths).unwrap();
    }

    #[test]
    fn signing_out_ends_the_sign_in_on_steam() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let access = jwt(ID, later());
        let account = Account { steam_id: ID.into(), account_name: String::new(), refresh_token: "R".into(), access_token: access.clone() };
        save(&paths, &account).unwrap();
        let fake = Fake::new(&[(200, 1, r#"{"response":{}}"#)]);
        assert!(sign_out_on_steam(&paths, &fake).unwrap());
        assert_eq!(load(&paths), None);
        let sent = fake.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].url.ends_with(&format!("/RevokeToken/v1/?access_token={access}")), "{}", sent[0].url);
        assert!(sent[0].form.contains(&("token", "R".into())));
    }

    #[test]
    fn an_old_access_token_is_renewed_to_sign_out_and_the_new_refresh_token_ended() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let account = Account { steam_id: ID.into(), account_name: String::new(), refresh_token: "R".into(), access_token: jwt(ID, 1) };
        save(&paths, &account).unwrap();
        let fake = Fake::new(&[(200, 1, r#"{"response":{"access_token":"A2","refresh_token":"R2"}}"#), (200, 1, r#"{"response":{}}"#)]);
        assert!(sign_out_on_steam(&paths, &fake).unwrap());
        let sent = fake.sent();
        // No new refresh token is asked for.
        assert!(sent[0].form.contains(&("renewal_type", "0".into())));
        assert!(sent[1].url.ends_with("?access_token=A2"), "{}", sent[1].url);
        assert!(sent[1].form.contains(&("token", "R2".into())));
        // The renewal is not saved: the sign-in is gone.
        assert_eq!(load(&paths), None);

        // Steam kept the refresh token: that one is ended.
        save(&paths, &account).unwrap();
        let fake = Fake::new(&[(200, 1, r#"{"response":{"access_token":"A2"}}"#), (200, 1, r#"{"response":{}}"#)]);
        assert!(sign_out_on_steam(&paths, &fake).unwrap());
        assert!(fake.sent()[1].form.contains(&("token", "R".into())));
    }

    #[test]
    fn a_sign_out_steam_does_not_take_is_still_a_sign_out_here() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let fresh = Account { steam_id: ID.into(), account_name: String::new(), refresh_token: "R".into(), access_token: jwt(ID, later()) };
        // Steam out of reach, or refusing: forgotten here, not on Steam.
        for answers in [vec![], vec![(503, 20, "")], vec![(200, 15, "")]] {
            save(&paths, &fresh).unwrap();
            assert!(!sign_out_on_steam(&paths, &Fake::new(&answers)).unwrap());
            assert_eq!(load(&paths), None);
        }
        // The renewal fails for a while (Steam busy): forgotten here only.
        let old = Account { access_token: jwt(ID, 1), ..fresh.clone() };
        save(&paths, &old).unwrap();
        assert!(!sign_out_on_steam(&paths, &Fake::new(&[(503, 20, "")])).unwrap());
        assert_eq!(load(&paths), None);
        // A refresh token Steam already dropped, found renewing or ending it:
        // nothing left to end there.
        save(&paths, &old).unwrap();
        assert!(sign_out_on_steam(&paths, &Fake::new(&[(200, 26, "")])).unwrap());
        assert_eq!(load(&paths), None);
        save(&paths, &fresh).unwrap();
        assert!(sign_out_on_steam(&paths, &Fake::new(&[(200, 26, "")])).unwrap());
        assert_eq!(load(&paths), None);
        // Nothing saved.
        assert!(!sign_out_on_steam(&paths, &Fake::new(&[])).unwrap());
    }

    #[test]
    fn a_sign_in_that_cannot_be_deleted_is_an_error_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        // A folder where the file goes: neither read nor deleted.
        std::fs::create_dir_all(path(&paths)).unwrap();
        let err = sign_out_on_steam(&paths, &Fake::new(&[])).unwrap_err();
        assert!(format!("{err:#}").contains("deleting"), "{err:#}");
    }
}
