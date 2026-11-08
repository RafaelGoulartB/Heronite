//! Signing in with a QR code: Steam's `IAuthenticationService`, the same
//! calls its own sign-in page and the Steam mobile app use. Heronite asks for
//! a sign-in session, shows its challenge URL as a QR code, and the user
//! scans and approves it in the Steam app on their phone; Heronite never sees
//! a password or a Steam Guard code.
//!
//! The session is the mobile app's kind (`platform_type` 3): its tokens are
//! the ones Steam's Web API takes, and its refresh token, which lasts months,
//! renews the access token (a day) through `GenerateAccessTokenForApp`.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::Value;

use super::http::{Http, Request, Response};

pub const API: &str = "https://api.steampowered.com";

/// `EAuthTokenPlatformType::MobileApp`.
const PLATFORM_MOBILE_APP: &str = "3";
/// `ETokenRenewalType::Allow`: a renewal may also give a new refresh token.
const RENEWAL_ALLOW: &str = "1";
/// `EAuthTokenRevokeAction::Permanent`: the refresh token stops working for
/// good, and the sign-in leaves the account's authorized devices.
const REVOKE_PERMANENT: &str = "1";
/// `EResult::OK`.
const OK: i32 = 1;
/// The `EResult`s that end a sign-in session: FileNotFound (gone),
/// AccessDenied (refused on the phone), Revoked, Expired.
const SESSION_GONE: [i32; 4] = [9, 15, 26, 27];
/// The `EResult`s that say a refresh token is dead: InvalidParam,
/// InvalidPassword, AccessDenied, Revoked, Expired. Any other one (Fail,
/// Busy, ServiceUnavailable, RateLimitExceeded...) is worth trying again
/// later, not a sign-out.
const TOKEN_DEAD: [i32; 5] = [8, 5, 15, 26, 27];

/// Steam no longer takes the saved refresh token (it expired, or the user
/// took the access away in the Steam app): sign in again.
#[derive(Debug)]
pub struct SignedOut;

impl std::fmt::Display for SignedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Steam no longer accepts this sign-in")
    }
}

impl std::error::Error for SignedOut {}

/// A sign-in waiting for the QR code to be scanned and approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrSession {
    pub client_id: String,
    /// Base64, as Steam gave it.
    pub request_id: String,
    /// What the QR code holds (`https://s.team/q/1/<client id>`). Steam
    /// changes it every little while; [`poll`] keeps it current.
    pub challenge_url: String,
    /// How often Steam wants to be asked.
    pub interval: Duration,
}

/// The tokens an approved sign-in gives.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub refresh_token: String,
    /// Not always given; [`renew`] gets one from the refresh token.
    pub access_token: Option<String>,
    pub account_name: String,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens").field("account_name", &self.account_name).finish_non_exhaustive()
    }
}

/// Where a sign-in stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// Not approved yet; `scanned`: the code was scanned and the phone asks
    /// the user to approve it.
    Waiting {
        scanned: bool,
    },
    Approved(Tokens),
    /// The session ended without an approval (it expired, or the user
    /// said no): a new code is needed.
    Ended,
}

/// A renewed access token, and the new refresh token when Steam gave one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renewed {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

fn url(method: &str) -> String {
    format!("{API}/IAuthenticationService/{method}/v1/")
}

/// The `response` object of a Web API answer with `EResult::OK`.
fn ok_response(r: Response, what: &str) -> Result<Value> {
    if r.status != 200 {
        bail!("{what}: HTTP {}", r.status);
    }
    if let Some(e) = r.eresult.filter(|e| *e != OK) {
        bail!("{what}: Steam answered with error {e}");
    }
    let doc: Value = serde_json::from_str(&r.body).with_context(|| format!("{what}: not JSON"))?;
    doc.get("response").cloned().ok_or_else(|| anyhow!("{what}: no response"))
}

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Starts a sign-in. `device` names it in the Steam app ("Heronite on
/// my-pc").
pub fn begin(http: &dyn Http, device: &str) -> Result<QrSession> {
    let form = vec![("device_friendly_name", device.to_string()), ("platform_type", PLATFORM_MOBILE_APP.to_string())];
    let r = ok_response(http.send(Request::post(url("BeginAuthSessionViaQR"), form))?, "starting the Steam sign-in")?;
    let field = |k: &str| text(&r, k).ok_or_else(|| anyhow!("starting the Steam sign-in: no {k}"));
    let interval = r.get("interval").and_then(Value::as_f64).filter(|s| *s >= 1.0).unwrap_or(5.0).min(30.0);
    Ok(QrSession {
        client_id: field("client_id")?,
        request_id: field("request_id")?,
        challenge_url: field("challenge_url")?,
        interval: Duration::from_secs_f64(interval),
    })
}

/// Asks whether the sign-in was approved. When Steam changed the code, the
/// session takes the new one (the QR code shown must follow).
pub fn poll(http: &dyn Http, session: &mut QrSession) -> Result<Poll> {
    let form = vec![("client_id", session.client_id.clone()), ("request_id", session.request_id.clone())];
    let r = http.send(Request::post(url("PollAuthSessionStatus"), form))?;
    if r.status != 200 {
        bail!("checking the Steam sign-in: HTTP {}", r.status);
    }
    match r.eresult {
        Some(e) if SESSION_GONE.contains(&e) => return Ok(Poll::Ended),
        // Busy, rate limited...: the caller asks again and counts it.
        Some(e) if e != OK => bail!("checking the Steam sign-in: Steam answered with error {e}"),
        _ => {}
    }
    let doc: Value = serde_json::from_str(&r.body).context("checking the Steam sign-in: not JSON")?;
    let resp = doc.get("response").cloned().unwrap_or(Value::Null);
    if let (Some(id), Some(url)) = (text(&resp, "new_client_id"), text(&resp, "new_challenge_url")) {
        session.client_id = id;
        session.challenge_url = url;
    }
    if let Some(refresh_token) = text(&resp, "refresh_token") {
        return Ok(Poll::Approved(Tokens {
            refresh_token,
            access_token: text(&resp, "access_token"),
            account_name: text(&resp, "account_name").unwrap_or_default(),
        }));
    }
    let scanned = resp.get("had_remote_interaction").and_then(Value::as_bool).unwrap_or(false);
    Ok(Poll::Waiting { scanned })
}

/// A new access token from the refresh token. [`SignedOut`] when Steam no
/// longer takes the refresh token.
pub fn renew(http: &dyn Http, refresh_token: &str, steam_id: &str) -> Result<Renewed> {
    let form =
        vec![("refresh_token", refresh_token.to_string()), ("steamid", steam_id.to_string()), ("renewal_type", RENEWAL_ALLOW.to_string())];
    let r = http.send(Request::post(url("GenerateAccessTokenForApp"), form))?;
    if matches!(r.status, 401 | 403) || r.eresult.is_some_and(|e| TOKEN_DEAD.contains(&e)) {
        return Err(SignedOut.into());
    }
    let resp = ok_response(r, "renewing the Steam sign-in")?;
    let access_token = text(&resp, "access_token").ok_or_else(|| anyhow!("renewing the Steam sign-in: no access token"))?;
    Ok(Renewed { access_token, refresh_token: text(&resp, "refresh_token") })
}

/// Ends the sign-in on Steam: `refresh_token` stops working and the sign-in
/// leaves the account's authorized devices. `access_token`: a working one of
/// the same sign-in (it goes in the address, the refresh token in the form).
pub fn revoke(http: &dyn Http, access_token: &str, refresh_token: &str) -> Result<()> {
    let form = vec![("token", refresh_token.to_string()), ("revoke_action", REVOKE_PERMANENT.to_string())];
    let r = http.send(Request::post(format!("{}?access_token={access_token}", url("RevokeToken")), form))?;
    if r.status != 200 {
        bail!("ending the Steam sign-in: HTTP {}", r.status);
    }
    if let Some(e) = r.eresult.filter(|e| *e != OK) {
        bail!("ending the Steam sign-in: Steam answered with error {e}");
    }
    Ok(())
}

/// What a Steam token (a JWT) says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    /// The account's SteamID64.
    pub steam_id: String,
    /// When it stops working (Unix seconds).
    pub expires: i64,
}

/// The claims of a token. Nothing is verified: Steam checks its tokens, this
/// only reads which account and until when.
pub fn claims(token: &str) -> Option<Claims> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let steam_id = text(&v, "sub").filter(|s| s.bytes().all(|b| b.is_ascii_digit()))?;
    Some(Claims { steam_id, expires: v.get("exp").and_then(Value::as_i64).unwrap_or(0) })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::account::http::Fake;

    /// A token for `steam_id` that ends at `exp` (the signature is not read).
    pub fn jwt(steam_id: &str, exp: i64) -> String {
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.sig", enc(r#"{"typ":"JWT","alg":"EdDSA"}"#), enc(&format!(r#"{{"iss":"steam","sub":"{steam_id}","exp":{exp}}}"#)))
    }

    #[test]
    fn a_sign_in_starts_with_a_challenge_url() {
        let fake = Fake::new(&[(
            200,
            1,
            r#"{"response":{"client_id":"8406597215213412810","challenge_url":"https://s.team/q/1/8406597215213412810","request_id":"FsK8UsPe1cQTf2nh72Lwbg==","interval":5,"allowed_confirmations":[{"confirmation_type":4}],"version":1}}"#,
        )]);
        let s = begin(&fake, "Heronite").unwrap();
        assert_eq!(s.client_id, "8406597215213412810");
        assert_eq!(s.request_id, "FsK8UsPe1cQTf2nh72Lwbg==");
        assert_eq!(s.challenge_url, "https://s.team/q/1/8406597215213412810");
        assert_eq!(s.interval, Duration::from_secs(5));
        let sent = fake.sent();
        assert_eq!(sent[0].url, "https://api.steampowered.com/IAuthenticationService/BeginAuthSessionViaQR/v1/");
        assert!(sent[0].form.contains(&("platform_type", "3".into())));
        assert!(sent[0].form.contains(&("device_friendly_name", "Heronite".into())));
    }

    fn session() -> QrSession {
        QrSession {
            client_id: "1".into(),
            request_id: "cmVx".into(),
            challenge_url: "https://s.team/q/1/1".into(),
            interval: Duration::from_secs(5),
        }
    }

    #[test]
    fn polling_follows_the_scan_the_new_codes_and_the_approval() {
        let fake = Fake::new(&[
            (200, 1, r#"{"response":{"had_remote_interaction":false}}"#),
            (200, 1, r#"{"response":{"new_client_id":"2","new_challenge_url":"https://s.team/q/1/2","had_remote_interaction":false}}"#),
            (200, 1, r#"{"response":{"had_remote_interaction":true}}"#),
            (200, 1, r#"{"response":{"refresh_token":"R","access_token":"A","had_remote_interaction":true,"account_name":"gaben"}}"#),
        ]);
        let mut s = session();
        assert_eq!(poll(&fake, &mut s).unwrap(), Poll::Waiting { scanned: false });
        assert_eq!(poll(&fake, &mut s).unwrap(), Poll::Waiting { scanned: false });
        assert_eq!((s.client_id.as_str(), s.challenge_url.as_str()), ("2", "https://s.team/q/1/2"));
        assert_eq!(poll(&fake, &mut s).unwrap(), Poll::Waiting { scanned: true });
        let tokens = Tokens { refresh_token: "R".into(), access_token: Some("A".into()), account_name: "gaben".into() };
        assert_eq!(poll(&fake, &mut s).unwrap(), Poll::Approved(tokens));
        // The request id goes as Steam gave it, with the current client id.
        assert!(fake.sent()[2].form.contains(&("client_id", "2".into())));
        assert!(fake.sent()[2].form.contains(&("request_id", "cmVx".into())));
    }

    #[test]
    fn an_expired_session_ends() {
        let fake = Fake::new(&[(200, 9, r#"{"response":{}}"#)]);
        assert_eq!(poll(&fake, &mut session()).unwrap(), Poll::Ended);
        let fake = Fake::new(&[(500, 2, "")]);
        assert!(poll(&fake, &mut session()).is_err());
        // Refused on the phone.
        assert_eq!(poll(&Fake::new(&[(200, 15, r#"{"response":{}}"#)]), &mut session()).unwrap(), Poll::Ended);
        // Steam busy: an error to count, the code stays.
        assert!(poll(&Fake::new(&[(200, 20, r#"{"response":{}}"#)]), &mut session()).is_err());
    }

    #[test]
    fn a_refused_refresh_token_is_a_sign_out() {
        let fake = Fake::new(&[
            (200, 1, r#"{"response":{"access_token":"A2","refresh_token":"R2"}}"#),
            (200, 1, r#"{"response":{"access_token":"A3"}}"#),
            (200, 15, r#"{"response":{}}"#),
            (401, 1, ""),
        ]);
        assert_eq!(renew(&fake, "R", "7656").unwrap(), Renewed { access_token: "A2".into(), refresh_token: Some("R2".into()) });
        assert_eq!(renew(&fake, "R", "7656").unwrap().refresh_token, None);
        assert!(renew(&fake, "R", "7656").unwrap_err().is::<SignedOut>());
        assert!(renew(&fake, "R", "7656").unwrap_err().is::<SignedOut>());
        assert!(fake.sent()[0].form.contains(&("steamid", "7656".into())));
        // A hiccup (ServiceUnavailable, RateLimitExceeded) keeps the sign-in.
        for eresult in [20, 84, 2] {
            let fake = Fake::new(&[(503, eresult, "")]);
            let err = renew(&fake, "R", "7656").unwrap_err();
            assert!(!err.is::<SignedOut>(), "{eresult}: {err:#}");
        }
        assert!(
            !format!("{:?}", Tokens { refresh_token: "secret".into(), access_token: None, account_name: "a".into() }).contains("secret")
        );
    }

    #[test]
    fn revoking_sends_the_refresh_token_in_the_form() {
        let fake = Fake::new(&[(200, 1, r#"{"response":{}}"#)]);
        revoke(&fake, "A", "R").unwrap();
        let sent = &fake.sent()[0];
        assert_eq!(sent.method, "POST");
        assert_eq!(sent.url, "https://api.steampowered.com/IAuthenticationService/RevokeToken/v1/?access_token=A");
        assert_eq!(sent.form, [("token", "R".to_string()), ("revoke_action", "1".to_string())]);
        // Refused or out of reach: an error, the caller says so.
        assert!(revoke(&Fake::new(&[(200, 15, "")]), "A", "R").is_err());
        assert!(revoke(&Fake::new(&[(401, 1, "")]), "A", "R").is_err());
        assert!(revoke(&Fake::new(&[]), "A", "R").is_err());
    }

    #[test]
    fn a_token_says_whose_it_is_and_until_when() {
        assert_eq!(
            claims(&jwt("76561197960287930", 1_800_000_000)),
            Some(Claims { steam_id: "76561197960287930".into(), expires: 1_800_000_000 })
        );
        assert_eq!(claims("not a token"), None);
        assert_eq!(claims(&jwt("me", 1)), None);
    }
}
