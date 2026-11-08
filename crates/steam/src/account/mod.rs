//! The Steam account, signed in with a QR code ([`auth`]); optional. With it
//! the library is read through Steam's Web API (`IPlayerService/
//! GetOwnedGames`) when the client on this PC cannot be read: Steam closed,
//! its debugging interface kept shut, or its port taken by another program.
//!
//! The client stays the first choice: only it knows the games hidden in
//! Steam and the ones borrowed through Family Sharing. The Web API lists the
//! account's own games, with their play time, whatever the profile's privacy
//! (the token is the account's own).
//!
//! The sign-in is kept in `local_library/steam-account.json`, readable only
//! by the user (on Windows `steam-account.bin`, sealed for this Windows user
//! with `heronite_platform::secret`). It holds the tokens, never a password.
//! Signing out deletes it.

pub mod auth;
pub mod http;

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use heronite_data::steam_owned::SteamOwnedGame;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use auth::{Poll, QrSession, SignedOut};
use http::{Http, Request};

/// A signed-in Steam account.
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

impl Account {
    /// What the page shows for the account.
    pub fn label(&self) -> &str {
        if self.account_name.is_empty() {
            &self.steam_id
        } else {
            &self.account_name
        }
    }
}

fn path(paths: &DataPaths) -> PathBuf {
    paths.local_meta().with_file_name(if cfg!(windows) { "steam-account.bin" } else { "steam-account.json" })
}

/// The account signed in here; `None` when none is, or the file cannot be
/// read (another Windows user's: the same as signed out).
pub fn load(paths: &DataPaths) -> Option<Account> {
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

pub fn save(paths: &DataPaths, account: &Account) -> Result<()> {
    let json = serde_json::to_vec_pretty(account)?;
    let bytes = if cfg!(windows) { heronite_platform::secret::protect(&json).context("sealing the Steam sign-in")? } else { json };
    let file = path(paths);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    heronite_data::temps::write_atomic(&file, &bytes, Some(0o600)).with_context(|| format!("writing {}", file.display()))
}

/// Forgets the account. (Steam keeps listing the sign-in under the
/// account's authorized devices until the user removes it there.)
pub fn sign_out(paths: &DataPaths) -> Result<()> {
    match std::fs::remove_file(path(paths)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Saves the account an approved sign-in gives.
pub fn finish(paths: &DataPaths, http: &dyn Http, tokens: auth::Tokens) -> Result<Account> {
    let steam_id = auth::claims(&tokens.refresh_token)
        .or_else(|| tokens.access_token.as_deref().and_then(auth::claims))
        .map(|c| c.steam_id)
        .ok_or_else(|| anyhow!("Steam's sign-in did not say which account it is"))?;
    let mut account =
        Account { steam_id, account_name: tokens.account_name, refresh_token: tokens.refresh_token, access_token: String::new() };
    match tokens.access_token {
        Some(token) => account.access_token = token,
        None => renew(paths, http, &mut account)?,
    }
    save(paths, &account)?;
    Ok(account)
}

/// Gets a new access token (and, when Steam gives one, refresh token) and
/// saves them. [`SignedOut`] deletes the saved sign-in.
fn renew(paths: &DataPaths, http: &dyn Http, account: &mut Account) -> Result<()> {
    match auth::renew(http, &account.refresh_token, &account.steam_id) {
        Ok(r) => {
            account.access_token = r.access_token;
            if let Some(refresh) = r.refresh_token {
                account.refresh_token = refresh;
            }
            save(paths, account)
        }
        Err(e) if e.is::<SignedOut>() => {
            sign_out(paths)?;
            Err(e)
        }
        Err(e) => Err(e),
    }
}

/// Whether the access token works for a while yet (a minute of margin).
fn fresh(token: &str, now: i64) -> bool {
    auth::claims(token).is_some_and(|c| c.expires > now + 60)
}

/// The account's games, as the Web API lists them (only games, as
/// [`crate::library::read_owned`] keeps them). `installed`: the app ids
/// installed on this PC. Renews the access token when it ran out.
pub fn read_owned(paths: &DataPaths, http: &dyn Http, installed: &HashSet<String>) -> Result<Vec<SteamOwnedGame>> {
    let mut account = load(paths).ok_or(SignedOut)?;
    if !fresh(&account.access_token, chrono::Utc::now().timestamp()) {
        renew(paths, http, &mut account)?;
    }
    let mut r = http.send(owned_request(&account))?;
    // Taken away before it ran out (a new password, a sign-out everywhere).
    if matches!(r.status, 401 | 403) {
        renew(paths, http, &mut account)?;
        r = http.send(owned_request(&account))?;
    }
    if r.status != 200 {
        anyhow::bail!("reading the Steam library: HTTP {}", r.status);
    }
    parse_owned(&r.body, installed)
}

fn owned_request(account: &Account) -> Request {
    Request::get(format!(
        "{}/IPlayerService/GetOwnedGames/v1/?access_token={}&steamid={}&include_appinfo=true&include_played_free_games=true&skip_unvetted_apps=false",
        auth::API,
        account.access_token,
        account.steam_id
    ))
}

fn parse_owned(body: &str, installed: &HashSet<String>) -> Result<Vec<SteamOwnedGame>> {
    let doc: Value = serde_json::from_str(body).context("reading the Steam library: not JSON")?;
    let Some(games) = doc.pointer("/response/games").and_then(Value::as_array) else {
        // An account with no games has `game_count: 0`; nothing at all is an error.
        return match doc.pointer("/response/game_count").and_then(Value::as_u64) {
            Some(0) => Ok(Vec::new()),
            _ => Err(anyhow!(tr!("Steam did not return your library. Try again in a moment."))),
        };
    };
    Ok(games
        .iter()
        .filter_map(|g| {
            let app_id = g.get("appid").and_then(Value::as_u64)?.to_string();
            let name = g.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            if name.is_empty() || crate::library::not_a_game(&name, &app_id) {
                return None;
            }
            Some(SteamOwnedGame {
                installed: installed.contains(&app_id),
                playtime_minutes: g.get("playtime_forever").and_then(Value::as_u64).unwrap_or(0),
                last_played: g.get("rtime_last_played").and_then(Value::as_i64).filter(|t| *t > 0),
                app_id,
                name,
                extra: Default::default(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::auth::tests::jwt;
    use super::auth::Tokens;
    use super::http::Fake;
    use super::*;

    const ID: &str = "76561197960287930";
    const OWNED: &str = r#"{"response":{"game_count":3,"games":[
        {"appid":620,"name":"Portal 2","playtime_forever":125,"rtime_last_played":1499373907},
        {"appid":1493710,"name":"Proton Experimental","playtime_forever":0,"rtime_last_played":0},
        {"appid":400,"name":"Portal","playtime_forever":0,"rtime_last_played":0}
    ]}}"#;

    fn later() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    #[test]
    fn a_sign_in_is_saved_for_this_user_only_and_forgotten_on_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let tokens =
            Tokens { refresh_token: jwt(ID, later() + 86_400), access_token: Some(jwt(ID, later())), account_name: "gaben".into() };
        let account = finish(&paths, &Fake::new(&[]), tokens).unwrap();
        assert_eq!((account.steam_id.as_str(), account.label()), (ID, "gaben"));
        assert_eq!(load(&paths), Some(account));
        #[cfg(unix)]
        assert_eq!(heronite_platform::fs::mode(&std::fs::metadata(path(&paths)).unwrap()).unwrap() & 0o777, 0o600);
        // The tokens never show in a log line.
        assert!(!format!("{:?}", load(&paths).unwrap()).contains("refresh"));
        sign_out(&paths).unwrap();
        assert_eq!(load(&paths), None);
        sign_out(&paths).unwrap();
    }

    #[test]
    fn a_sign_in_without_an_access_token_gets_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let access = jwt(ID, later());
        let fake = Fake::new(&[(200, 1, &format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
        let tokens = Tokens { refresh_token: jwt(ID, later()), access_token: None, account_name: String::new() };
        let account = finish(&paths, &fake, tokens).unwrap();
        assert_eq!(account.access_token, access);
        assert_eq!(account.label(), ID);
    }

    #[test]
    fn the_library_comes_with_play_time_and_the_installed_ones() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let account =
            Account { steam_id: ID.into(), account_name: "gaben".into(), refresh_token: "R".into(), access_token: jwt(ID, later()) };
        save(&paths, &account).unwrap();
        let fake = Fake::new(&[(200, 1, OWNED)]);
        let installed: HashSet<String> = ["400".to_string()].into();
        let games = read_owned(&paths, &fake, &installed).unwrap();
        let summary: Vec<_> = games.iter().map(|g| (g.app_id.as_str(), g.installed, g.playtime_minutes, g.last_played)).collect();
        assert_eq!(summary, [("620", false, 125, Some(1_499_373_907)), ("400", true, 0, None)]);
        let url = &fake.sent()[0].url;
        assert!(url.starts_with("https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?access_token="), "{url}");
        assert!(url.contains(&format!("steamid={ID}")) && url.contains("include_appinfo=true"), "{url}");
    }

    #[test]
    fn an_old_access_token_is_renewed_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let account = Account { steam_id: ID.into(), account_name: String::new(), refresh_token: "R".into(), access_token: jwt(ID, 1) };
        save(&paths, &account).unwrap();
        let access = jwt(ID, later());
        let fake =
            Fake::new(&[(200, 1, &format!(r#"{{"response":{{"access_token":"{access}","refresh_token":"R2"}}}}"#)), (200, 1, OWNED)]);
        assert_eq!(read_owned(&paths, &fake, &HashSet::new()).unwrap().len(), 2);
        let saved = load(&paths).unwrap();
        assert_eq!((saved.access_token, saved.refresh_token.as_str()), (access, "R2"));
    }

    #[test]
    fn a_refused_sign_in_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let account =
            Account { steam_id: ID.into(), account_name: String::new(), refresh_token: "R".into(), access_token: jwt(ID, later()) };
        save(&paths, &account).unwrap();
        // The access token was taken away, and the refresh token with it.
        let fake = Fake::new(&[(401, 1, ""), (200, 88, r#"{"response":{}}"#)]);
        assert!(read_owned(&paths, &fake, &HashSet::new()).unwrap_err().is::<SignedOut>());
        assert_eq!(load(&paths), None);
        assert!(read_owned(&paths, &Fake::new(&[]), &HashSet::new()).unwrap_err().is::<SignedOut>());
    }

    #[test]
    fn an_empty_answer_is_an_error_but_no_games_is_not() {
        assert!(parse_owned(r#"{"response":{}}"#, &HashSet::new()).is_err());
        assert!(parse_owned(r#"{"response":{"game_count":0}}"#, &HashSet::new()).unwrap().is_empty());
    }
}
