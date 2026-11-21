//! The account's tokens in `gog_store/auth.json`, the file gogdl kept:
//! `{"<client id>": {access_token, refresh_token, user_id, expires_in,
//! loginTime, ...}}`. The Galaxy client id holds the account's sign-in; a
//! game with a client of its own (for cloud saves) gets an entry under its
//! id, made from the account's refresh token. Keys this code does not use
//! are kept on every rewrite.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use heronite_i18n::tr;
use parking_lot::Mutex;
use serde_json::{Map, Value};

use crate::api::{self, Client};
use crate::{CLIENT_ID, CLIENT_SECRET};

/// GOG refused the saved sign-in (or a token it handed out): the account
/// must log in again. Found with `err.downcast_ref::<SignInExpired>()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignInExpired;

impl std::fmt::Display for SignInExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the GOG sign-in expired")
    }
}

impl std::error::Error for SignInExpired {}

/// Where the login page sends the browser; GOG checks it against the client.
pub const REDIRECT_URI: &str = "https://embed.gog.com/on_login_success?origin=client";
/// Tokens are refreshed this long before they run out, so a request started
/// now does not meet an expired one.
const SKEW_SECS: f64 = 60.;

/// Only one refresh runs at a time: two would each write the file.
static WRITE: Mutex<()> = Mutex::new(());

/// One client's tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    /// Seconds the access token lives from `login_time`.
    pub expires_in: f64,
    /// Seconds since the epoch.
    pub login_time: f64,
}

impl Credentials {
    fn from_entry(entry: &Value) -> Option<Self> {
        let text = |k: &str| entry.get(k).and_then(api::id_text);
        let num = |k: &str| entry.get(k).and_then(Value::as_f64).unwrap_or(0.);
        Some(Self {
            access_token: text("access_token")?,
            refresh_token: text("refresh_token").unwrap_or_default(),
            user_id: text("user_id").unwrap_or_default(),
            expires_in: num("expires_in"),
            login_time: num("loginTime"),
        })
    }

    /// Whether the access token is past (or near) its end at `now`.
    pub fn expired_at(&self, now: f64) -> bool {
        now + SKEW_SECS >= self.login_time + self.expires_in
    }
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.)
}

fn read(path: &Path) -> Map<String, Value> {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => map,
            _ => {
                log::warn!("{} is not a JSON object; starting over", path.display());
                Map::new()
            }
        },
        Err(_) => Map::new(),
    }
}

fn write(path: &Path, doc: &Map<String, Value>) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let bytes = serde_json::to_vec(doc)?;
    heronite_data::temps::write_atomic(path, &bytes, Some(0o600)).with_context(|| format!("writing {}", path.display()))
}

/// Stores a token answer under `client_id`, stamped with the time.
fn store(path: &Path, client_id: &str, mut answer: Map<String, Value>) -> Result<Credentials> {
    answer.insert("loginTime".into(), Value::from(now()));
    let mut doc = read(path);
    let entry = Value::Object(answer);
    let creds = Credentials::from_entry(&entry).ok_or_else(|| anyhow!("GOG sent no access token"))?;
    doc.insert(client_id.to_string(), entry);
    write(path, &doc)?;
    Ok(creds)
}

/// The tokens saved for `client_id`, as they are (maybe expired).
pub fn saved(path: &Path, client_id: &str) -> Option<Credentials> {
    read(path).get(client_id).and_then(Credentials::from_entry)
}

/// Whether the file has the account's tokens. No network.
pub fn has_tokens(path: &Path) -> bool {
    read(path).get(CLIENT_ID).is_some_and(Value::is_object)
}

fn token_url(client_id: &str, client_secret: &str, grant: &str) -> String {
    format!("{}/token?client_id={client_id}&client_secret={client_secret}&{grant}", api::AUTH)
}

/// The JSON object of a token answer; a refusal (4xx) is a [`SignInExpired`].
fn token_answer(url: &str) -> Result<Map<String, Value>> {
    let resp = Client::anonymous().get(url, &[])?;
    if (400..500).contains(&resp.status) {
        return Err(anyhow!(SignInExpired).context(format!("GOG refused the sign-in (HTTP {})", resp.status)));
    }
    api::check(&resp, url)?;
    match resp.json()? {
        Value::Object(map) if map.get("access_token").is_some() => Ok(map),
        _ => bail!("GOG sent no access token"),
    }
}

/// Exchanges the authorization code of the login page for the account's
/// tokens and saves them.
pub fn login(path: &Path, code: &str) -> Result<Credentials> {
    let grant =
        format!("grant_type=authorization_code&code={}&redirect_uri={}", api::encode(code, false), api::encode(REDIRECT_URI, false));
    let answer = token_answer(&token_url(CLIENT_ID, CLIENT_SECRET, &grant)).map_err(|err| {
        if err.downcast_ref::<SignInExpired>().is_some() {
            anyhow!(tr!("GOG did not accept that code"))
        } else {
            err
        }
    })?;
    let _guard = WRITE.lock();
    store(path, CLIENT_ID, answer)
}

/// The account's tokens, refreshed first when they ran out. No tokens is an
/// error; tokens GOG no longer refreshes are a [`SignInExpired`].
pub fn credentials(path: &Path) -> Result<Credentials> {
    game_credentials(path, CLIENT_ID, CLIENT_SECRET)
}

/// Tokens for `client_id` (a game's own client, for its cloud saves), made
/// from the account's refresh token when missing or expired. With the
/// Galaxy client this is [`credentials`].
pub fn game_credentials(path: &Path, client_id: &str, client_secret: &str) -> Result<Credentials> {
    if let Some(c) = saved(path, client_id).filter(|c| !c.expired_at(now())) {
        return Ok(c);
    }
    let _guard = WRITE.lock();
    // Another thread may have refreshed it while this one waited.
    if let Some(c) = saved(path, client_id).filter(|c| !c.expired_at(now())) {
        return Ok(c);
    }
    let Some(account) = saved(path, CLIENT_ID) else { bail!(tr!("You are not logged in to GOG")) };
    if account.refresh_token.is_empty() {
        return Err(anyhow!(SignInExpired).context(tr!("Your GOG sign-in expired: log in again")));
    }
    let mut grant = format!("grant_type=refresh_token&refresh_token={}", api::encode(&account.refresh_token, false));
    // A game's client gets tokens of the same session, not a new one.
    if client_id != CLIENT_ID {
        grant.push_str("&without_new_session=1");
    }
    let answer = token_answer(&token_url(client_id, client_secret, &grant)).map_err(|err| {
        if err.downcast_ref::<SignInExpired>().is_some() {
            err.context(tr!("Your GOG sign-in expired: log in again"))
        } else {
            err
        }
    })?;
    store(path, client_id, answer)
}

/// Deletes the tokens (the file); nothing is sent to GOG.
pub fn logout(path: &Path) -> Result<()> {
    let _guard = WRITE.lock();
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entries_are_read_with_numbers_or_text() {
        let c = Credentials::from_entry(
            &json!({"access_token": "a", "refresh_token": "r", "user_id": 42, "expires_in": 3600, "loginTime": 100.5}),
        )
        .unwrap();
        assert_eq!((c.user_id.as_str(), c.expires_in, c.login_time), ("42", 3600., 100.5));
        assert!(!c.expired_at(100.5 + 3000.));
        assert!(c.expired_at(100.5 + 3550.));
        assert!(Credentials::from_entry(&json!({"user_id": "1"})).is_none());
    }

    #[test]
    fn storing_keeps_other_clients_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gog_store/auth.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"46899977096215655": {"access_token": "old"}, "999": {"access_token": "g", "extra": 1}}"#).unwrap();
        let answer = json!({"access_token": "new", "refresh_token": "r2", "user_id": "7", "expires_in": 3600, "session_id": "s"});
        let c = store(&path, CLIENT_ID, answer.as_object().unwrap().clone()).unwrap();
        assert_eq!(c.access_token, "new");
        let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(doc["999"]["extra"], 1);
        assert_eq!(doc[CLIENT_ID]["session_id"], "s");
        assert!(doc[CLIENT_ID]["loginTime"].as_f64().unwrap() > 1.6e9);
        assert!(has_tokens(&path));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn fresh_tokens_need_no_network_and_missing_ones_are_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        assert!(credentials(&path).is_err());
        let entry = json!({CLIENT_ID: {"access_token": "t", "refresh_token": "r", "user_id": "1", "expires_in": 3600, "loginTime": now()}});
        std::fs::write(&path, entry.to_string()).unwrap();
        assert_eq!(credentials(&path).unwrap().access_token, "t");
        logout(&path).unwrap();
        assert!(!path.exists());
        logout(&path).unwrap();
    }

    #[test]
    fn an_expired_sign_in_without_a_refresh_token_must_log_in_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, json!({CLIENT_ID: {"access_token": "t", "expires_in": 1, "loginTime": 1}}).to_string()).unwrap();
        let err = credentials(&path).unwrap_err();
        assert!(err.downcast_ref::<SignInExpired>().is_some(), "{err:#}");
    }
}
