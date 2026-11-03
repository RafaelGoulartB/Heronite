//! The Epic sign-in: legendary's `user.json` (the token answer of Epic's
//! OAuth service, kept as it came), signing in with the code the login page
//! shows, renewing the token, and the one-time codes a game signs in with.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value};

use crate::client::Client;
use crate::files;
use crate::http::{ApiError, Auth, Body, Request};

/// The Epic Games Launcher's OAuth client, which legendary signs in as.
pub const CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";
pub(crate) const CLIENT_SECRET: &str = "daafbccc737745039dffe53d94fc76cf";
/// A token with less left than this is renewed first (legendary's 10 minutes).
const REFRESH_MARGIN_MINUTES: i64 = 10;

/// `user.json` below legendary's config folder.
pub fn user_file(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("user.json")
}

/// Epic refused the saved sign-in (its refresh token expired or was revoked):
/// only signing in again helps. legendary deletes `user.json` then, and so
/// does [`Client::ensure_session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInRefused {
    /// Epic's `errorCode`.
    pub code: String,
    /// A page to open when Epic wants the user to act first (accept new
    /// terms): `continuationUrl`.
    pub continuation_url: Option<String>,
}

impl std::fmt::Display for SignInRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Epic refused the saved sign-in ({})", self.code)
    }
}

impl std::error::Error for SignInRefused {}

/// Whether `err`, or what caused it, is a [`SignInRefused`].
pub fn is_sign_in_refused(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.is::<SignInRefused>())
}

/// The saved sign-in. Every key Epic sent is kept; the token is never logged.
#[derive(Clone, PartialEq)]
pub struct Session {
    doc: Map<String, Value>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("account_id", &self.account_id()).field("expires_at", &self.text("expires_at")).finish()
    }
}

impl Session {
    pub fn from_value(doc: Value) -> Result<Self> {
        let Value::Object(doc) = doc else { bail!("the sign-in is not a JSON object") };
        let s = Self { doc };
        if s.access_token().is_empty() || s.account_id().is_empty() {
            bail!("the sign-in has no token or account");
        }
        Ok(s)
    }

    /// `user.json` of `config_dir`; `None` when signed out.
    pub fn load(config_dir: &Path) -> Result<Option<Self>> {
        let path = user_file(config_dir);
        let Some(doc) = files::read_json(&path) else { return Ok(None) };
        Self::from_value(doc).map(Some).with_context(|| format!("reading {}", path.display()))
    }

    /// Writes `user.json` (readable by the user only).
    pub fn save(&self, config_dir: &Path) -> Result<()> {
        files::write_json(&user_file(config_dir), &Value::Object(self.doc.clone()), true)
    }

    fn text(&self, key: &str) -> &str {
        self.doc.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    pub fn account_id(&self) -> &str {
        self.text("account_id")
    }

    pub fn display_name(&self) -> &str {
        self.text("displayName")
    }

    pub fn access_token(&self) -> &str {
        self.text("access_token")
    }

    fn time(&self, key: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(self.text(key)).ok().map(|t| t.with_timezone(&Utc))
    }

    /// When the access token stops working.
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.time("expires_at")
    }

    /// When the refresh token stops working (a new sign-in is needed then).
    pub fn refresh_expires_at(&self) -> Option<DateTime<Utc>> {
        self.time("refresh_expires_at")
    }

    /// The access token has less than 10 minutes left (or no known end).
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        self.expires_at().is_none_or(|at| at - now <= Duration::minutes(REFRESH_MARGIN_MINUTES))
    }

    /// Every key, for code that needs one this type does not name.
    pub fn raw(&self) -> &Map<String, Value> {
        &self.doc
    }

    /// The answer of a token request over the old sign-in: new keys win,
    /// keys only the old one had stay.
    fn merged(&self, answer: Map<String, Value>) -> Self {
        let mut doc = self.doc.clone();
        doc.extend(answer);
        Self { doc }
    }
}

/// Which grant a token request uses.
enum Grant<'a> {
    AuthorizationCode(&'a str),
    ExchangeCode(&'a str),
    RefreshToken(&'a str),
    ClientCredentials,
}

impl Client {
    /// `POST /account/api/oauth/token` with the launcher's client.
    fn token(&self, grant: Grant) -> Result<Map<String, Value>> {
        let url = format!("{}/account/api/oauth/token", self.endpoints.oauth);
        let fields: Vec<(&str, &str)> = match grant {
            Grant::AuthorizationCode(code) => vec![("grant_type", "authorization_code"), ("code", code)],
            Grant::ExchangeCode(code) => vec![("grant_type", "exchange_code"), ("exchange_code", code)],
            Grant::RefreshToken(token) => vec![("grant_type", "refresh_token"), ("refresh_token", token)],
            Grant::ClientCredentials => vec![("grant_type", "client_credentials")],
        };
        let mut fields = fields;
        fields.push(("token_type", "eg1"));
        let resp = self.http.send(&self.post(&url, Body::Form(&fields)).auth(Auth::Basic(&self.client_id, &self.client_secret)))?;
        if resp.status >= 500 {
            bail!("Epic's sign-in service is unavailable (HTTP {})", resp.status);
        }
        let doc = resp.json().unwrap_or(Value::Null);
        if let Some(code) = doc.get("errorCode").and_then(Value::as_str) {
            return Err(SignInRefused {
                code: code.to_string(),
                continuation_url: doc.get("continuationUrl").and_then(Value::as_str).map(str::to_string),
            }
            .into());
        }
        if !resp.ok() {
            return Err(ApiError::from_response(&resp).into());
        }
        match doc {
            Value::Object(map) if map.get("access_token").and_then(Value::as_str).is_some_and(|t| !t.is_empty()) => Ok(map),
            _ => bail!("Epic's sign-in service gave no token"),
        }
    }

    /// Signs in with the authorization code of the login page and saves the
    /// session to `config_dir` (`legendary auth --code`).
    pub fn login_with_code(&self, config_dir: &Path, code: &str) -> Result<Session> {
        let session = Session::from_value(Value::Object(self.token(Grant::AuthorizationCode(code))?))?;
        session.save(config_dir)?;
        Ok(session)
    }

    /// Signs in with an exchange code (another launcher's sign-in) and saves
    /// the session (`legendary auth --token`).
    pub fn login_with_exchange_code(&self, config_dir: &Path, code: &str) -> Result<Session> {
        let session = Session::from_value(Value::Object(self.token(Grant::ExchangeCode(code))?))?;
        session.save(config_dir)?;
        Ok(session)
    }

    /// Renews `session` with its refresh token and saves the answer at once
    /// (Epic hands out a new refresh token and may refuse the old one).
    pub fn refresh(&self, config_dir: &Path, session: &Session) -> Result<Session> {
        let token = session.text("refresh_token");
        if token.is_empty() {
            return Err(SignInRefused { code: "no_refresh_token".into(), continuation_url: None }.into());
        }
        let renewed = session.merged(self.token(Grant::RefreshToken(token))?);
        renewed.save(config_dir)?;
        Ok(renewed)
    }

    /// The saved session with a token good for at least 10 more minutes,
    /// renewed first when needed. Signed out gives `Ok(None)`. When Epic
    /// refuses the refresh token, `user.json` is deleted (as legendary does)
    /// and the error is a [`SignInRefused`].
    pub fn ensure_session(&self, config_dir: &Path) -> Result<Option<Session>> {
        let Some(session) = Session::load(config_dir)? else { return Ok(None) };
        if !session.needs_refresh(Utc::now()) {
            return Ok(Some(session));
        }
        log::info!("epic: renewing the sign-in");
        match self.refresh(config_dir, &session) {
            Ok(s) => Ok(Some(s)),
            Err(err) if is_sign_in_refused(&err) => {
                log::warn!("epic: the saved sign-in was refused: {err}");
                let _ = std::fs::remove_file(user_file(config_dir));
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    /// Ends the session at Epic (best effort) and deletes `user.json`.
    pub fn logout(&self, config_dir: &Path) -> Result<()> {
        if let Ok(Some(session)) = Session::load(config_dir) {
            let url = format!("{}/account/api/oauth/sessions/kill/{}", self.endpoints.oauth, session.access_token());
            let req = Request { method: "DELETE", ..self.get(&url).bearer(session.access_token()) };
            if let Err(err) = self.http.send(&req) {
                log::debug!("epic: ending the session: {err:#}");
            }
        }
        match std::fs::remove_file(user_file(config_dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).context("removing the Epic sign-in"),
            _ => Ok(()),
        }
    }

    /// A one-time code a game signs in with (`-AUTH_PASSWORD`).
    pub fn exchange_code(&self, session: &Session) -> Result<String> {
        let url = format!("{}/account/api/oauth/exchange", self.endpoints.oauth);
        let doc = self.http.get_json(&self.get(&url).bearer(session.access_token()))?;
        doc.get("code")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Epic gave no sign-in code"))
    }

    /// The ownership token of a game with DRM that checks it (`-epicovt`).
    pub fn ownership_token(&self, session: &Session, namespace: &str, catalog_item_id: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/ecommerceintegration/api/public/platforms/EPIC/identities/{}/ownershipToken",
            self.endpoints.ecommerce,
            session.account_id()
        );
        let item = format!("{namespace}:{catalog_item_id}");
        let resp = self.http.send(&self.post(&url, Body::Form(&[("nsCatalogItemId", &item)])).bearer(session.access_token()))?;
        if !resp.ok() {
            return Err(ApiError::from_response(&resp).into());
        }
        Ok(resp.body)
    }

    /// A token of no account, enough to read public builds (the EOS overlay).
    pub fn anonymous_token(&self) -> Result<String> {
        let doc = self.token(Grant::ClientCredentials)?;
        Ok(doc.get("access_token").and_then(Value::as_str).unwrap_or_default().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Endpoints;
    use crate::http::tests::serve;
    use serde_json::json;

    fn token_answer(token: &str, expires_at: &str) -> String {
        json!({
            "access_token": token, "refresh_token": format!("r-{token}"), "expires_at": expires_at,
            "refresh_expires_at": "2099-01-01T00:00:00.000Z", "account_id": "acc1", "displayName": "Player", "token_type": "bearer"
        })
        .to_string()
    }

    #[test]
    fn sessions_know_when_to_renew() {
        let s = Session::from_value(json!({"access_token": "t", "account_id": "a", "expires_at": "2026-11-05T03:02:01.328Z"})).unwrap();
        let at = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        assert!(!s.needs_refresh(at("2026-11-05T02:00:00Z")));
        assert!(s.needs_refresh(at("2026-11-05T02:55:00Z")));
        assert!(Session::from_value(json!({"account_id": "a"})).is_err());
        assert!(!format!("{s:?}").contains("\"t\""));
    }

    #[test]
    fn a_code_signs_in_and_is_saved() {
        let answer = Box::leak(token_answer("tok", "2099-01-01T00:00:00.000Z").into_boxed_str());
        let (url, server) = serve(vec![(200, answer)]);
        let dir = tempfile::tempdir().unwrap();
        let client = Client::new(Endpoints::all_at(&url));
        let s = client.login_with_code(dir.path(), "CODE").unwrap();
        assert_eq!((s.account_id(), s.display_name()), ("acc1", "Player"));
        assert_eq!(Session::load(dir.path()).unwrap().unwrap(), s);
        let seen = server.join().unwrap();
        assert!(seen[0].contains("grant_type=authorization_code&code=CODE&token_type=eg1"), "{seen:?}");
    }

    #[test]
    fn an_expiring_token_is_renewed_and_saved_with_the_old_keys() {
        let answer = Box::leak(token_answer("new", "2099-01-01T00:00:00.000Z").into_boxed_str());
        let (url, server) = serve(vec![(200, answer)]);
        let dir = tempfile::tempdir().unwrap();
        files::write_json(
            &user_file(dir.path()),
            &json!({"access_token": "old", "refresh_token": "r-old", "account_id": "acc1", "expires_at": "2000-01-01T00:00:00Z", "extra": 1}),
            true,
        )
        .unwrap();
        let client = Client::new(Endpoints::all_at(&url));
        let s = client.ensure_session(dir.path()).unwrap().unwrap();
        assert_eq!(s.access_token(), "new");
        let saved = files::read_json(&user_file(dir.path())).unwrap();
        assert_eq!(saved["refresh_token"], "r-new");
        assert_eq!(saved["extra"], 1);
        assert!(server.join().unwrap()[0].contains("grant_type=refresh_token&refresh_token=r-old"));
        // Still fresh: no request at all.
        assert_eq!(client.ensure_session(dir.path()).unwrap().unwrap().access_token(), "new");
    }

    #[test]
    fn a_refused_refresh_signs_out() {
        let (url, server) =
            serve(vec![(400, r#"{"errorCode":"errors.com.epicgames.account.auth_token.invalid_refresh_token","errorMessage":"no"}"#)]);
        let dir = tempfile::tempdir().unwrap();
        files::write_json(&user_file(dir.path()), &json!({"access_token": "old", "refresh_token": "r", "account_id": "a"}), true).unwrap();
        let err = Client::new(Endpoints::all_at(&url)).ensure_session(dir.path()).unwrap_err();
        assert!(is_sign_in_refused(&err));
        assert!(!user_file(dir.path()).exists());
        server.join().unwrap();
    }

    #[test]
    fn server_errors_keep_the_sign_in() {
        let (url, server) = serve(vec![(503, "")]);
        let dir = tempfile::tempdir().unwrap();
        files::write_json(&user_file(dir.path()), &json!({"access_token": "old", "refresh_token": "r", "account_id": "a"}), true).unwrap();
        let err = Client::new(Endpoints::all_at(&url)).ensure_session(dir.path()).unwrap_err();
        assert!(!is_sign_in_refused(&err));
        assert!(user_file(dir.path()).exists());
        server.join().unwrap();
    }

    #[test]
    fn logout_removes_the_file_even_offline() {
        let dir = tempfile::tempdir().unwrap();
        files::write_json(&user_file(dir.path()), &json!({"access_token": "t", "account_id": "a"}), true).unwrap();
        let client = Client::new(Endpoints::all_at("http://127.0.0.1:9"));
        client.logout(dir.path()).unwrap();
        assert!(!user_file(dir.path()).exists());
        client.logout(dir.path()).unwrap();
    }
}
