//! What is left of the Steam sign-in Heronite once had: ending a sign-in
//! saved by an earlier version on Steam (`IAuthenticationService`), so its
//! tokens stop working and it leaves the account's authorized devices.
//!
//! Heronite no longer signs in to Steam: the sign-in with a QR code made a
//! session of the Steam mobile app's kind from a PC, which Steam's security
//! took for a phishing attack and locked accounts over.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::Value;

use super::http::{Http, Request, Response};

const API: &str = "https://api.steampowered.com";

/// `ETokenRenewalType::None`: the refresh token stays as it is.
const RENEWAL_NONE: &str = "0";
/// `EAuthTokenRevokeAction::Permanent`: the refresh token stops working for
/// good, and the sign-in leaves the account's authorized devices.
const REVOKE_PERMANENT: &str = "1";
/// `EResult::OK`.
const OK: i32 = 1;
/// The `EResult`s that say a refresh token is dead: InvalidParam,
/// InvalidPassword, AccessDenied, Revoked, Expired. Any other one (Fail,
/// Busy, ServiceUnavailable, RateLimitExceeded...) is worth trying again
/// later, not a sign-out.
const TOKEN_DEAD: [i32; 5] = [8, 5, 15, 26, 27];
/// The `EResult`s that say a token was already ended: Revoked, Expired.
const TOKEN_ENDED: [i32; 2] = [26, 27];

/// Steam no longer takes the saved refresh token (it expired, or the user
/// removed the device on Steam's site): nothing left to end.
#[derive(Debug)]
pub struct SignedOut;

impl std::fmt::Display for SignedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Steam no longer accepts this sign-in")
    }
}

impl std::error::Error for SignedOut {}

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

/// A new access token only, the refresh token kept: to end the sign-in
/// ([`revoke`]) without Steam handing out a new refresh token first.
/// [`SignedOut`] when Steam no longer takes the refresh token.
pub fn renew_access(http: &dyn Http, refresh_token: &str, steam_id: &str) -> Result<Renewed> {
    let form =
        vec![("refresh_token", refresh_token.to_string()), ("steamid", steam_id.to_string()), ("renewal_type", RENEWAL_NONE.to_string())];
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
/// `Ok` too when Steam says the token was already ended (the user removed the
/// device on Steam's site).
pub fn revoke(http: &dyn Http, access_token: &str, refresh_token: &str) -> Result<()> {
    let form = vec![("token", refresh_token.to_string()), ("revoke_action", REVOKE_PERMANENT.to_string())];
    let r = http.send(Request::post(format!("{}?access_token={access_token}", url("RevokeToken")), form))?;
    if r.eresult.is_some_and(|e| TOKEN_ENDED.contains(&e)) {
        return Ok(());
    }
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
    fn a_refused_refresh_token_is_a_sign_out() {
        let fake = Fake::new(&[(200, 1, r#"{"response":{"access_token":"A2"}}"#), (200, 15, r#"{"response":{}}"#), (401, 1, "")]);
        assert_eq!(renew_access(&fake, "R", "7656").unwrap(), Renewed { access_token: "A2".into(), refresh_token: None });
        assert!(fake.sent()[0].form.contains(&("steamid", "7656".into())));
        assert!(fake.sent()[0].form.contains(&("renewal_type", "0".into())));
        assert!(renew_access(&fake, "R", "7656").unwrap_err().is::<SignedOut>());
        assert!(renew_access(&fake, "R", "7656").unwrap_err().is::<SignedOut>());
        // A hiccup (ServiceUnavailable, RateLimitExceeded) is not a sign-out.
        for eresult in [20, 84, 2] {
            let fake = Fake::new(&[(503, eresult, "")]);
            let err = renew_access(&fake, "R", "7656").unwrap_err();
            assert!(!err.is::<SignedOut>(), "{eresult}: {err:#}");
        }
    }

    #[test]
    fn revoking_sends_the_refresh_token_in_the_form() {
        let fake = Fake::new(&[(200, 1, r#"{"response":{}}"#)]);
        revoke(&fake, "A", "R").unwrap();
        let sent = &fake.sent()[0];
        assert_eq!(sent.method, "POST");
        assert_eq!(sent.url, "https://api.steampowered.com/IAuthenticationService/RevokeToken/v1/?access_token=A");
        assert_eq!(sent.form, [("token", "R".to_string()), ("revoke_action", "1".to_string())]);
        // Already ended (removed on Steam's site): nothing left to do.
        revoke(&Fake::new(&[(200, 26, "")]), "A", "R").unwrap();
        revoke(&Fake::new(&[(200, 27, "")]), "A", "R").unwrap();
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
