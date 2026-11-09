//! Signing in: the user signs in to their Microsoft account on Microsoft's
//! own page (Heronite never sees the password), which hands back an
//! authorization code; the code becomes Microsoft account tokens
//! (`login.live.com`), whose access token becomes an Xbox network user token
//! (`user.auth.xboxlive.com`), then an XSTS token (`xsts.auth.xboxlive.com`)
//! that the other services take.
//!
//! The code is bound to this sign-in by PKCE (S256): only the program that
//! opened the page can use it. Heronite's own app registration is used
//! ([`crate::client_id`]); with none, signing in is not offered.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::live::http::{Http, Request};

pub const AUTHORIZE_URL: &str = "https://login.live.com/oauth20_authorize.srf";
pub const TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
/// Where Microsoft sends a desktop program's sign-in: a blank page whose
/// address carries the code.
pub const REDIRECT_URI: &str = "https://login.live.com/oauth20_desktop.srf";
pub const SCOPE: &str = "Xboxlive.signin Xboxlive.offline_access";
pub const USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
pub const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";

/// An XSTS token this close to its end is renewed first.
const MARGIN_MINUTES: i64 = 10;

/// One sign-in's secret (PKCE) and the page to open for it.
#[derive(Debug, Clone)]
pub struct SignIn {
    pub verifier: String,
    pub state: String,
    pub url: String,
}

impl SignIn {
    /// A new sign-in for `client_id`, with fresh random secrets.
    pub fn new(client_id: &str) -> Result<Self> {
        Ok(Self::with_secrets(client_id, random(32)?, random(16)?))
    }

    fn with_secrets(client_id: &str, verifier: String, state: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = url::Url::parse(AUTHORIZE_URL).expect("constant URL");
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPE)
            .append_pair("redirect_uri", REDIRECT_URI)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("prompt", "select_account");
        Self { verifier, state, url: url.into() }
    }

    /// The code in the address the sign-in ended on (the user may paste
    /// it), when it is this sign-in's. Microsoft's refusal (the user said
    /// no) is an error with its words.
    pub fn code_from(&self, address: &str) -> Result<String> {
        let url = url::Url::parse(address.trim()).context("not an address")?;
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let get = |k: &str| query.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
        if let Some(err) = get("error") {
            bail!("{}", get("error_description").unwrap_or(err));
        }
        if !(url.host_str() == Some("login.live.com") && url.path() == "/oauth20_desktop.srf") {
            bail!("not the address the sign-in ends on");
        }
        if get("state").as_deref() != Some(self.state.as_str()) {
            bail!("this address belongs to another sign-in");
        }
        get("code").ok_or_else(|| anyhow!("the address has no code"))
    }
}

/// `n` random bytes, URL-safe base64.
fn random(n: usize) -> Result<String> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|e| anyhow!("no random numbers: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The Microsoft account tokens. Only the refresh token is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub access_token: String,
    pub refresh_token: String,
}

/// The token Xbox network's services take, with who it is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Xsts {
    pub token: String,
    /// User hash: goes with the token in the header.
    pub uhs: String,
    pub xuid: String,
    #[serde(default)]
    pub gamertag: String,
    pub not_after: DateTime<Utc>,
}

impl Xsts {
    pub fn header(&self) -> String {
        format!("XBL3.0 x={};{}", self.uhs, self.token)
    }

    /// Still good for a while at `now`.
    pub fn fresh(&self, now: DateTime<Utc>) -> bool {
        self.not_after - Duration::minutes(MARGIN_MINUTES) > now
    }
}

fn token_request(client_id: &str, grant: &[(&'static str, &str)]) -> Request {
    let mut form = vec![("client_id", client_id.to_string()), ("redirect_uri", REDIRECT_URI.to_string()), ("scope", SCOPE.to_string())];
    form.extend(grant.iter().map(|(k, v)| (*k, v.to_string())));
    Request::post_form(TOKEN_URL, form)
}

fn parse_live(status: u16, body: &str) -> Result<Live> {
    let doc: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        // invalid_grant: the refresh token was revoked or expired.
        if doc.get("error").and_then(Value::as_str) == Some("invalid_grant") {
            return Err(crate::live::SignedOut.into());
        }
        let why = doc.get("error_description").or(doc.get("error")).and_then(Value::as_str).unwrap_or("no reason given");
        bail!("Microsoft account: HTTP {status}: {why}");
    }
    let text = |k: &str| doc.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    Ok(Live {
        access_token: text("access_token").context("Microsoft account: no access token")?,
        refresh_token: text("refresh_token").context("Microsoft account: no refresh token")?,
    })
}

/// The tokens for a sign-in's code.
pub fn redeem(http: &dyn Http, client_id: &str, sign_in: &SignIn, code: &str) -> Result<Live> {
    let req = token_request(client_id, &[("grant_type", "authorization_code"), ("code", code), ("code_verifier", &sign_in.verifier)]);
    let (status, body) = http.send(req)?;
    parse_live(status, &body)
}

/// New tokens from the refresh token (Microsoft hands a new refresh token too).
pub fn refresh(http: &dyn Http, client_id: &str, refresh_token: &str) -> Result<Live> {
    let (status, body) = http.send(token_request(client_id, &[("grant_type", "refresh_token"), ("refresh_token", refresh_token)]))?;
    parse_live(status, &body)
}

fn xbl_error(status: u16, doc: &Value, what: &str) -> anyhow::Error {
    // XErr: 2148916233 no Xbox profile yet, 2148916238 a child account
    // whose family settings say no, 2148916235 a country without Xbox.
    match doc.get("XErr").and_then(Value::as_u64) {
        Some(2148916233) => anyhow!("This Microsoft account has no Xbox profile yet. Sign in once at xbox.com, then try again."),
        Some(2148916238) => anyhow!("This account's family settings do not let it sign in here."),
        Some(2148916235) => anyhow!("Xbox network is not available in this account's country."),
        _ if status == 401 => crate::live::SignedOut.into(),
        _ => anyhow!("{what}: HTTP {status}"),
    }
}

/// The XSTS token for a Microsoft account access token.
pub fn xsts(http: &dyn Http, access_token: &str) -> Result<Xsts> {
    let user = json!({
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
        "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={access_token}")},
    });
    let contract = vec![("x-xbl-contract-version", "1".to_string())];
    let (status, body) = http.send(Request::post_json(USER_AUTH_URL, contract.clone(), user))?;
    let doc: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(xbl_error(status, &doc, "Xbox network sign-in"));
    }
    let user_token = doc.get("Token").and_then(Value::as_str).filter(|t| !t.is_empty()).context("Xbox network sign-in: no token")?;

    let authorize = json!({
        "RelyingParty": "http://xboxlive.com",
        "TokenType": "JWT",
        "Properties": {"SandboxId": "RETAIL", "UserTokens": [user_token]},
    });
    let (status, body) = http.send(Request::post_json(XSTS_URL, contract, authorize))?;
    let doc: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(xbl_error(status, &doc, "Xbox network authorization"));
    }
    parse_xsts(&doc)
}

fn parse_xsts(doc: &Value) -> Result<Xsts> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let xui = doc.pointer("/DisplayClaims/xui/0").context("Xbox network authorization: no user")?;
    let not_after = text(doc, "NotAfter")
        .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&Utc))
        .context("Xbox network authorization: no end date")?;
    Ok(Xsts {
        token: text(doc, "Token").context("Xbox network authorization: no token")?,
        uhs: text(xui, "uhs").context("Xbox network authorization: no user hash")?,
        xuid: text(xui, "xid").context("Xbox network authorization: no user id")?,
        gamertag: text(xui, "gtg").unwrap_or_default(),
        not_after,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::live::http::{Body, Fake};

    pub(crate) const CLIENT: &str = "0b9f6e5a-1c2d-4e3f-8a9b-0c1d2e3f4a5b";
    pub(crate) const LIVE_OK: &str = r#"{"token_type":"bearer","expires_in":3600,"scope":"Xboxlive.signin Xboxlive.offline_access","access_token":"live-access","refresh_token":"live-refresh-2","user_id":"abc"}"#;
    pub(crate) const USER_OK: &str = r#"{"IssueInstant":"2026-11-07T10:00:00Z","NotAfter":"2026-11-21T10:00:00Z","Token":"user-token","DisplayClaims":{"xui":[{"uhs":"uhs1"}]}}"#;
    pub(crate) const XSTS_OK: &str = r#"{"IssueInstant":"2026-11-07T10:00:00.0000000Z","NotAfter":"2026-11-08T02:00:00.0000000Z","Token":"xsts-token","DisplayClaims":{"xui":[{"gtg":"Lamplighter","xid":"2533274800000001","uhs":"uhs1","agg":"Adult","usr":"234","utr":"190","prv":"185"}]}}"#;

    fn form(body: &Body) -> Vec<(String, String)> {
        let Body::Form(f) = body else { panic!("not a form: {body:?}") };
        f.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn the_sign_in_page_carries_the_pkce_challenge() {
        // RFC 7636, appendix B.
        let s = SignIn::with_secrets(CLIENT, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(), "st4te".into());
        let url = url::Url::parse(&s.url).unwrap();
        assert_eq!((url.host_str(), url.path()), (Some("login.live.com"), "/oauth20_authorize.srf"));
        let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], CLIENT);
        assert_eq!(q["code_challenge"], "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["redirect_uri"], REDIRECT_URI);
        assert_eq!(q["scope"], SCOPE);
        assert_eq!(q["state"], "st4te");
        assert_eq!(q["response_type"], "code");

        let a = SignIn::new(CLIENT).unwrap();
        let b = SignIn::new(CLIENT).unwrap();
        assert_ne!(a.verifier, b.verifier);
        assert!(a.verifier.len() >= 43, "RFC 7636: 43 characters at least");
    }

    #[test]
    fn the_code_comes_from_the_end_of_the_sign_in() {
        let s = SignIn::with_secrets(CLIENT, "v".repeat(43), "st4te".into());
        assert_eq!(
            s.code_from(" https://login.live.com/oauth20_desktop.srf?code=M.C5_BAY.2.U.abc&state=st4te ").unwrap(),
            "M.C5_BAY.2.U.abc"
        );
        let other = s.code_from("https://login.live.com/oauth20_desktop.srf?code=x&state=other").unwrap_err();
        assert!(other.to_string().contains("another sign-in"));
        let denied = s
            .code_from(
                "https://login.live.com/oauth20_desktop.srf?error=access_denied&error_description=The+user+has+denied+access&state=st4te",
            )
            .unwrap_err();
        assert_eq!(denied.to_string(), "The user has denied access");
        assert!(s.code_from("https://evil.example/oauth20_desktop.srf?code=x&state=st4te").is_err());
        assert!(s.code_from("https://login.live.com/oauth20_desktop.srf?state=st4te").is_err());
        assert!(s.code_from("not a url").is_err());
    }

    #[test]
    fn redeems_the_code_with_the_verifier() {
        let s = SignIn::with_secrets(CLIENT, "verifier-1234567890123456789012345678901234".into(), "st".into());
        let fake = Fake::new(&[(200, LIVE_OK)]);
        let live = redeem(&fake, CLIENT, &s, "the-code").unwrap();
        assert_eq!(live, Live { access_token: "live-access".into(), refresh_token: "live-refresh-2".into() });
        let sent = fake.sent();
        assert_eq!(sent[0].url, TOKEN_URL);
        let f = form(&sent[0].body);
        for (k, v) in [
            ("grant_type", "authorization_code"),
            ("code", "the-code"),
            ("code_verifier", "verifier-1234567890123456789012345678901234"),
            ("client_id", CLIENT),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPE),
        ] {
            assert!(f.contains(&(k.to_string(), v.to_string())), "{k} missing in {f:?}");
        }
        assert!(!f.iter().any(|(k, _)| k == "client_secret"), "a public client has no secret");
    }

    #[test]
    fn refreshing_a_revoked_sign_in_signs_out() {
        let fake = Fake::new(&[(200, LIVE_OK)]);
        assert_eq!(refresh(&fake, CLIENT, "old").unwrap().refresh_token, "live-refresh-2");
        assert!(form(&fake.sent()[0].body).contains(&("grant_type".into(), "refresh_token".into())));

        let revoked = Fake::new(&[(400, r#"{"error":"invalid_grant","error_description":"The token was revoked."}"#)]);
        assert!(refresh(&revoked, CLIENT, "old").unwrap_err().is::<crate::live::SignedOut>());
        let other = Fake::new(&[(400, r#"{"error":"invalid_client","error_description":"The client does not exist."}"#)]);
        let e = refresh(&other, CLIENT, "old").unwrap_err();
        assert!(!e.is::<crate::live::SignedOut>());
        assert!(e.to_string().contains("The client does not exist."));
        assert!(refresh(&Fake::new(&[(200, r#"{"access_token":"a"}"#)]), CLIENT, "old").is_err(), "no refresh token");
    }

    #[test]
    fn exchanges_the_access_token_for_xsts() {
        let fake = Fake::new(&[(200, USER_OK), (200, XSTS_OK)]);
        let x = xsts(&fake, "live-access").unwrap();
        assert_eq!(
            (x.token.as_str(), x.uhs.as_str(), x.xuid.as_str(), x.gamertag.as_str()),
            ("xsts-token", "uhs1", "2533274800000001", "Lamplighter")
        );
        assert_eq!(x.header(), "XBL3.0 x=uhs1;xsts-token");
        assert_eq!(x.not_after.to_rfc3339(), "2026-11-08T02:00:00+00:00");
        let sent = fake.sent();
        assert_eq!(sent[0].url, USER_AUTH_URL);
        let Body::Json(user) = &sent[0].body else { panic!() };
        assert_eq!(user["Properties"]["RpsTicket"], "d=live-access");
        assert_eq!(user["RelyingParty"], "http://auth.xboxlive.com");
        assert_eq!(sent[1].url, XSTS_URL);
        let Body::Json(auth) = &sent[1].body else { panic!() };
        assert_eq!(auth["Properties"]["UserTokens"], json!(["user-token"]));
        assert_eq!(auth["Properties"]["SandboxId"], "RETAIL");
        assert_eq!(auth["RelyingParty"], "http://xboxlive.com");

        let now = DateTime::parse_from_rfc3339("2026-11-08T01:40:00Z").unwrap().with_timezone(&Utc);
        assert!(x.fresh(now));
        assert!(!x.fresh(now + Duration::minutes(15)), "renewed before its last ten minutes");
    }

    #[test]
    fn xbox_refusals_say_why() {
        let child = Fake::new(&[
            (200, USER_OK),
            (401, r#"{"Identity":"0","XErr":2148916238,"Message":"","Redirect":"https://start.ui.xboxlive.com/AddChildToFamily"}"#),
        ]);
        assert!(xsts(&child, "a").unwrap_err().to_string().contains("family settings"));
        let no_profile = Fake::new(&[(200, USER_OK), (401, r#"{"XErr":2148916233}"#)]);
        assert!(xsts(&no_profile, "a").unwrap_err().to_string().contains("no Xbox profile"));
        let expired = Fake::new(&[(401, "")]);
        assert!(xsts(&expired, "a").unwrap_err().is::<crate::live::SignedOut>());
        let broken =
            Fake::new(&[(200, USER_OK), (200, r#"{"Token":"t","NotAfter":"2026-11-08T02:00:00Z","DisplayClaims":{"xui":[{"uhs":"u"}]}}"#)]);
        assert!(xsts(&broken, "a").unwrap_err().to_string().contains("no user id"));
    }
}
