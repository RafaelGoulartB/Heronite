//! The signed-in account: `local_library/xbox-account.bin`, sealed for this
//! Windows user (`heronite_platform::secret`). It holds the Microsoft
//! account refresh token and the last XSTS token (with its end), never a
//! password. Signing out deletes it.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};

use crate::live::auth::{self, Live, SignIn, Xsts};
use crate::live::http::Http;

const FILE: &str = "xbox-account.bin";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub refresh_token: String,
    pub xsts: Xsts,
}

impl Account {
    pub fn gamertag(&self) -> &str {
        &self.xsts.gamertag
    }
}

fn path(paths: &DataPaths) -> PathBuf {
    paths.local_meta().with_file_name(FILE)
}

/// The account signed in here; `None` when none is, or the file cannot be
/// opened (another user's, another PC's: the same as signed out).
pub fn load(paths: &DataPaths) -> Option<Account> {
    let sealed = std::fs::read(path(paths)).ok()?;
    let plain = match heronite_platform::secret::unprotect(&sealed) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("xbox: the saved sign-in cannot be opened ({e}); signed out");
            return None;
        }
    };
    serde_json::from_slice(&plain).ok()
}

pub fn save(paths: &DataPaths, account: &Account) -> Result<()> {
    let sealed = heronite_platform::secret::protect(&serde_json::to_vec(account)?).context("sealing the Xbox sign-in")?;
    let file = path(paths);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("bin.tmp");
    std::fs::write(&tmp, sealed)?;
    std::fs::rename(&tmp, &file).with_context(|| format!("writing {}", file.display()))
}

/// Forgets the account. (The user can also take Heronite's access away at
/// account.live.com/consent/Manage.)
pub fn sign_out(paths: &DataPaths) -> Result<()> {
    match std::fs::remove_file(path(paths)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Finishes a sign-in: the code becomes tokens, which are saved.
pub fn finish(paths: &DataPaths, http: &dyn Http, client_id: &str, sign_in: &SignIn, code: &str) -> Result<Account> {
    let live = auth::redeem(http, client_id, sign_in, code)?;
    let account = Account { xsts: auth::xsts(http, &live.access_token)?, refresh_token: live.refresh_token };
    save(paths, &account)?;
    Ok(account)
}

/// An XSTS token good for a while: the saved one, else a new one from the
/// refresh token (saved with the new refresh token). [`crate::live::SignedOut`]
/// when Microsoft no longer takes the refresh token: the file is deleted.
pub fn xsts(paths: &DataPaths, http: &dyn Http, client_id: &str) -> Result<Xsts> {
    let Some(mut account) = load(paths) else { return Err(crate::live::SignedOut.into()) };
    if account.xsts.fresh(Utc::now()) {
        return Ok(account.xsts);
    }
    let renewed = auth::refresh(http, client_id, &account.refresh_token)
        .and_then(|Live { access_token, refresh_token }| Ok(Account { xsts: auth::xsts(http, &access_token)?, refresh_token }));
    match renewed {
        Ok(fresh) => {
            account = fresh;
            save(paths, &account)?;
            Ok(account.xsts)
        }
        Err(e) if e.is::<crate::live::SignedOut>() => {
            sign_out(paths)?;
            Err(e)
        }
        Err(e) => Err(e),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::live::auth::tests::{CLIENT, LIVE_OK, USER_OK, XSTS_OK};
    use crate::live::http::Fake;
    use chrono::{Duration, TimeZone};

    fn account(not_after: chrono::DateTime<Utc>) -> Account {
        Account {
            refresh_token: "refresh-1".into(),
            xsts: Xsts { token: "old-xsts".into(), uhs: "u".into(), xuid: "1".into(), gamertag: "Lamplighter".into(), not_after },
        }
    }

    #[test]
    fn the_sign_in_is_sealed_and_signing_out_forgets_it() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert_eq!(load(&paths), None);
        let a = account(Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap());
        save(&paths, &a).unwrap();
        let raw = std::fs::read(path(&paths)).unwrap();
        assert!(!raw.windows(9).any(|w| w == b"refresh-1"), "not in the clear");
        assert_eq!(load(&paths), Some(a));
        std::fs::write(path(&paths), b"garbage").unwrap();
        assert_eq!(load(&paths), None, "unreadable is signed out");
        sign_out(&paths).unwrap();
        sign_out(&paths).unwrap();
        assert!(!path(&paths).exists());
    }

    #[test]
    fn a_fresh_token_is_used_as_it_is_and_an_old_one_renewed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(xsts(&paths, &Fake::new(&[]), CLIENT).unwrap_err().is::<crate::live::SignedOut>());

        save(&paths, &account(Utc::now() + Duration::hours(5))).unwrap();
        let none = Fake::new(&[]);
        assert_eq!(xsts(&paths, &none, CLIENT).unwrap().token, "old-xsts");
        assert!(none.sent().is_empty(), "nothing asked");

        save(&paths, &account(Utc::now() + Duration::minutes(3))).unwrap();
        let fake = Fake::new(&[(200, LIVE_OK), (200, USER_OK), (200, XSTS_OK)]);
        assert_eq!(xsts(&paths, &fake, CLIENT).unwrap().token, "xsts-token");
        assert_eq!(fake.sent().len(), 3);
        assert_eq!(load(&paths).unwrap().refresh_token, "live-refresh-2", "the new refresh token is kept");
    }

    #[test]
    fn a_revoked_sign_in_is_forgotten_and_a_network_error_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        save(&paths, &account(Utc::now() - Duration::hours(1))).unwrap();
        let down = Fake::new(&[(503, "")]);
        assert!(!xsts(&paths, &down, CLIENT).unwrap_err().is::<crate::live::SignedOut>());
        assert!(load(&paths).is_some(), "kept for later");
        let revoked = Fake::new(&[(400, r#"{"error":"invalid_grant"}"#)]);
        assert!(xsts(&paths, &revoked, CLIENT).unwrap_err().is::<crate::live::SignedOut>());
        assert!(load(&paths).is_none());
    }

    #[test]
    fn finishing_a_sign_in_saves_it() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let s = SignIn::new(CLIENT).unwrap();
        let fake = Fake::new(&[(200, LIVE_OK), (200, USER_OK), (200, XSTS_OK)]);
        let a = finish(&paths, &fake, CLIENT, &s, "code").unwrap();
        assert_eq!(a.gamertag(), "Lamplighter");
        assert_eq!(load(&paths), Some(a));
    }
}
