//! EA's services, as Lutris's EA library asks them: a token from
//! `accounts.ea.com/connect/auth` for the web session, then the games the
//! account owns from the GraphQL service (`ownedGameProducts`, paged), and
//! each one's content id and title (`legacyOffers`, `gameProducts`, a
//! hundred offers a query). Only base games for the PC, bought on EA's
//! store, the way Lutris filters them.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::http::{Http, Request, Response};
use super::{Error, Session};
use crate::owned::OwnedGame;

/// A token for the web session's account, without a page (`prompt=none`):
/// the client of EA's own web pages, which answers with JSON.
pub const AUTH_URL: &str =
    "https://accounts.ea.com/connect/auth?client_id=ORIGIN_JS_SDK&response_type=token&redirect_uri=nucleus:rest&prompt=none";
const GRAPHQL_URL: &str = "https://service-aggregation-layer.juno.ea.com/graphql";
/// Games a page of the owned list holds, and offers a details query asks for.
const PAGE: usize = 100;
/// Pages read at most (10 000 games): a `next` that never ends stops here.
const MAX_PAGES: usize = 100;
/// Redirects of `connect/auth` followed, as a browser (Lutris's `requests`,
/// Playnite's web view) would while EA renews the session.
const MAX_HOPS: usize = 5;

const OWNED_QUERY: &str = "query getEntitlements($limit: Int, $next: String) { me { ownedGameProducts(locale: \"DEFAULT\", entitlementEnabled: true, storefronts: [EA], type: [DIGITAL_FULL_GAME, PACKAGED_FULL_GAME], platforms: [PC], paging: {limit: $limit, next: $next}) { next items { originOfferId product { baseItem { gameType } } } } } }";
const OFFERS_QUERY: &str = "query getOffers($offerIds: [String!]!) { legacyOffers(offerIds: $offerIds, locale: \"DEFAULT\") { offerId: id contentId } gameProducts(offerIds: $offerIds, locale: \"DEFAULT\") { items { id originOfferId baseItem { title } } } }";
const NAME_QUERY: &str = "query { me { player { displayName } } }";

/// Starts the account from what the login window printed: the session must
/// give a token. The name EA shows is added when it says it.
pub fn sign_in(http: &dyn Http, login_window: &str) -> Result<Session, Error> {
    let mut session = Session::from_login_window(login_window).ok_or_else(|| Error::Changed("no EA cookies after the sign-in".into()))?;
    let token = token(http, &mut session)?;
    match display_name(http, &token) {
        Ok(name) => session.name = name,
        Err(e) => log::info!("EA account: no name ({e})"),
    }
    Ok(session)
}

/// The games `session`'s account owns. EA may renew the session's cookies
/// meanwhile: the caller keeps `session` when it changed.
pub fn read(http: &dyn Http, session: &mut Session) -> Result<Vec<OwnedGame>, Error> {
    let token = token(http, session)?;
    let offers = owned_offers(http, &token)?;
    let mut games = Vec::new();
    for chunk in offers.chunks(PAGE) {
        games.extend(details(http, &token, chunk)?);
    }
    Ok(games)
}

/// A token for the session (EA's answer is JSON: `access_token`, or
/// `error` `login_required` once the session is over). Redirects that stay
/// on `accounts.ea.com` are followed, keeping the cookies each one sets; one
/// to EA's sign-in page means the session is over.
pub fn token(http: &dyn Http, session: &mut Session) -> Result<String, Error> {
    if session.cookies.is_empty() {
        return Err(Error::SignedOut);
    }
    let mut url = url::Url::parse(AUTH_URL).expect("a valid address");
    for _ in 0..=MAX_HOPS {
        let headers = vec![("Cookie", session.header()), ("Accept", "application/json".into())];
        let resp = http.send(Request::get(url.as_str(), headers)).map_err(Error::Failed)?;
        for line in &resp.set_cookies {
            session.apply(line);
        }
        if !(300..400).contains(&resp.status) {
            return token_from(&resp);
        }
        let next = resp.location.as_deref().and_then(|l| url.join(l).ok());
        match next {
            Some(next)
                if next.scheme() == "https"
                    && next.host_str() == Some("accounts.ea.com")
                    && next.path().starts_with("/connect/auth")
                    && !has_error(&next) =>
            {
                url = next
            }
            Some(next) => return token_in_redirect(&next),
            None => return Err(Error::Changed(format!("connect/auth redirected ({}) without an address", resp.status))),
        }
    }
    Err(Error::Failed(anyhow::anyhow!("accounts.ea.com redirected more than {MAX_HOPS} times")))
}

/// The OAuth error a redirect carries, in its query or fragment.
fn redirect_error(url: &url::Url) -> Option<String> {
    let fragment = url.fragment().map(|f| url::form_urlencoded::parse(f.as_bytes()).into_owned().collect::<Vec<_>>()).unwrap_or_default();
    url.query_pairs().into_owned().chain(fragment).find(|(k, _)| k == "error").map(|(_, v)| v)
}

fn has_error(url: &url::Url) -> bool {
    redirect_error(url).is_some()
}

/// A redirect that carries an OAuth error saying the user must sign in.
fn asks_to_sign_in(url: &url::Url) -> bool {
    redirect_error(url).is_some_and(|e| SIGN_IN_ERRORS.contains(&e.as_str()))
}

/// The errors of `connect/auth` that mean the session is over.
const SIGN_IN_ERRORS: [&str; 4] = ["login_required", "invalid_grant", "consent_required", "interaction_required"];

/// Where a redirect off `accounts.ea.com` leads: the token itself (sent to
/// `nucleus:rest#access_token=…`), EA's sign-in, or somewhere unknown, which
/// keeps the session (the next read tries again).
fn token_in_redirect(next: &url::Url) -> Result<String, Error> {
    if asks_to_sign_in(next) || next.host_str() == Some("signin.ea.com") {
        return Err(Error::SignedOut);
    }
    if let Some(e) = redirect_error(next).filter(|e| matches!(e.as_str(), "server_error" | "temporarily_unavailable")) {
        return Err(Error::Failed(anyhow::anyhow!("accounts.ea.com: {e}")));
    }
    if next.scheme() == "nucleus" {
        let params = next.fragment().or(next.query()).unwrap_or("");
        if let Some((_, token)) = url::form_urlencoded::parse(params.as_bytes()).find(|(k, v)| k == "access_token" && !v.is_empty()) {
            return Ok(token.into_owned());
        }
    }
    Err(Error::Changed(format!("connect/auth redirected to {}", next.host_str().unwrap_or(next.scheme()))))
}

fn token_from(resp: &Response) -> Result<String, Error> {
    let doc: Option<Value> = serde_json::from_str(&resp.body).ok();
    let field = |k: &str| doc.as_ref().and_then(|d| d.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty());
    if let Some(token) = field("access_token").filter(|_| resp.status == 200) {
        return Ok(token.to_string());
    }
    let error = field("error").or_else(|| field("error_code"));
    match (resp.status, error) {
        (_, Some(e)) if SIGN_IN_ERRORS.contains(&e) => Err(Error::SignedOut),
        // EA's service failing or busy, or refusing how it is asked.
        (429 | 500.., Some(e)) => Err(Error::Failed(anyhow::anyhow!("accounts.ea.com: {e}"))),
        (_, Some(e)) => Err(Error::Changed(format!("connect/auth: {e}"))),
        (200, None) => Err(Error::Changed("no token in connect/auth's answer".into())),
        (status, None) => Err(Error::Failed(anyhow::anyhow!("accounts.ea.com answered {status}"))),
    }
}

/// Runs a GraphQL query; its `data`. A query EA refuses (its `errors`) is
/// one this version asks wrong: [`Error::Changed`].
fn graphql(http: &dyn Http, token: &str, query: &str, variables: Value) -> Result<Value, Error> {
    let headers = vec![("Authorization", format!("Bearer {token}")), ("Accept", "application/json".into())];
    let resp =
        http.send(Request::post_json(GRAPHQL_URL, headers, json!({"query": query, "variables": variables}))).map_err(Error::Failed)?;
    let doc: Option<Value> = serde_json::from_str(&resp.body).ok();
    let first_error = doc
        .as_ref()
        .and_then(|d| d.get("errors"))
        .and_then(Value::as_array)
        .and_then(|e| e.first())
        .map(|e| e.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
    // Busy, or a token EA took back meanwhile: the next read tries again.
    if resp.status >= 500 || matches!(resp.status, 401 | 403 | 429) {
        return Err(Error::Failed(anyhow::anyhow!("EA's library service answered {}", resp.status)));
    }
    if let Some(message) = first_error {
        return Err(Error::Changed(message));
    }
    if resp.status != 200 {
        return Err(Error::Changed(format!("EA's library service answered {}", resp.status)));
    }
    doc.and_then(|mut d| d.get_mut("data").map(Value::take))
        .filter(|d| !d.is_null())
        .ok_or_else(|| Error::Changed("no data in the answer".into()))
}

fn display_name(http: &dyn Http, token: &str) -> Result<Option<String>, Error> {
    let data = graphql(http, token, NAME_QUERY, json!({}))?;
    Ok(data.pointer("/me/player/displayName").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string))
}

/// The offer ids of the base games the account owns, in EA's order.
fn owned_offers(http: &dyn Http, token: &str) -> Result<Vec<String>, Error> {
    let mut offers: Vec<String> = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let data = graphql(http, token, OWNED_QUERY, json!({"limit": PAGE, "next": next}))?;
        let page = data.pointer("/me/ownedGameProducts").ok_or_else(|| Error::Changed("no ownedGameProducts".into()))?;
        let items = page.get("items").and_then(Value::as_array).ok_or_else(|| Error::Changed("no items in ownedGameProducts".into()))?;
        for item in items {
            let base = item.pointer("/product/baseItem/gameType").and_then(Value::as_str) == Some("BASE_GAME");
            if let Some(offer) = item.get("originOfferId").and_then(Value::as_str).filter(|o| base && !o.is_empty()) {
                if !offers.iter().any(|o| o == offer) {
                    offers.push(offer.to_string());
                }
            }
        }
        let after = page.get("next").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
        if after.is_none() || after == next {
            return Ok(offers);
        }
        next = after;
    }
    log::warn!("EA account: the owned list did not end after {MAX_PAGES} pages");
    Ok(offers)
}

/// The content id (what the installed game's `installerdata.xml` names
/// first, so both are one Collection entry) and title of each offer. An
/// offer EA gives no product for is left out, as Lutris does.
fn details(http: &dyn Http, token: &str, offers: &[String]) -> Result<Vec<OwnedGame>, Error> {
    let data = graphql(http, token, OFFERS_QUERY, json!({"offerIds": offers}))?;
    let legacy = data.get("legacyOffers").and_then(Value::as_array).ok_or_else(|| Error::Changed("no legacyOffers".into()))?;
    let products: Vec<&Value> =
        data.pointer("/gameProducts/items").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let by = |key: &str| -> HashMap<&str, &Value> {
        products.iter().filter_map(|p| Some((p.get(key)?.as_str().filter(|s| !s.is_empty())?, *p))).collect()
    };
    let (by_offer, by_id) = (by("originOfferId"), by("id"));
    let mut out: Vec<OwnedGame> = Vec::new();
    for offer in legacy {
        let text = |k: &str| offer.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let (Some(offer_id), Some(content_id)) = (text("offerId"), text("contentId")) else { continue };
        let Some(product) = by_offer.get(offer_id).or_else(|| by_id.get(content_id)).or_else(|| by_id.get(offer_id)) else { continue };
        let Some(title) = product.pointer("/baseItem/title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else {
            continue;
        };
        if crate::ea::usable_content_id(content_id) && !out.iter().any(|g| g.id == content_id) {
            out.push(OwnedGame { id: content_id.to_string(), title: title.to_string() });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::http::{answer, Fake};
    use super::super::Cookie;
    use super::*;

    fn session() -> Session {
        Session {
            cookies: vec![Cookie { name: "sid".into(), value: "S1".into() }, Cookie { name: "remid".into(), value: "R1".into() }],
            name: None,
        }
    }

    fn token_ok() -> Response {
        Response {
            status: 200,
            body: r#"{"access_token":"TOKEN","token_type":"Bearer","expires_in":"3599"}"#.into(),
            set_cookies: vec!["sid=S2; Path=/; Secure; HttpOnly".into()],
            location: None,
        }
    }

    fn owned_page(items: &[(&str, &str)], next: Option<&str>) -> Response {
        let items: Vec<Value> =
            items.iter().map(|(offer, kind)| json!({"originOfferId": offer, "product": {"baseItem": {"gameType": kind}}})).collect();
        answer(200, &json!({"data": {"me": {"ownedGameProducts": {"next": next, "items": items}}}}).to_string())
    }

    #[test]
    fn a_token_comes_from_the_session_and_renews_its_cookies() {
        let http = Fake::new(vec![token_ok()]);
        let mut s = session();
        assert_eq!(token(&http, &mut s).unwrap(), "TOKEN");
        let sent = http.sent();
        assert_eq!(sent[0].url, AUTH_URL);
        assert!(sent[0].headers.contains(&("Cookie", "sid=S1; remid=R1".into())));
        assert_eq!(s.cookies[0].value, "S2", "EA renewed the session");
    }

    #[test]
    fn an_ended_session_is_signed_out_and_other_answers_are_not() {
        let cases: Vec<(Response, &str)> = vec![
            (answer(200, r#"{"error_code":"login_required","error":"login_required","error_number":"102100"}"#), "out"),
            (answer(200, "<html>a page</html>"), "changed"),
            (answer(503, "busy"), "failed"),
            (answer(400, r#"{"error":"invalid_request"}"#), "changed"),
            (answer(503, r#"{"error":"temporarily_unavailable"}"#), "failed"),
            (answer(429, r#"{"error":"rate_limited"}"#), "failed"),
        ];
        for (resp, want) in cases {
            let got = match token_from(&resp) {
                Err(Error::SignedOut) => "out",
                Err(Error::Changed(_)) => "changed",
                Err(Error::Failed(_)) => "failed",
                Ok(_) => "ok",
            };
            assert_eq!(got, want, "{resp:?}");
        }
        assert!(matches!(token(&Fake::new(vec![]), &mut Session::default()), Err(Error::SignedOut)), "no cookies: nothing is sent");
    }

    fn redirect(to: &str, cookie: Option<&str>) -> Response {
        Response {
            status: 302,
            location: Some(to.into()),
            set_cookies: cookie.into_iter().map(str::to_string).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn redirects_on_accounts_ea_com_are_followed_with_the_cookies_they_set() {
        let http = Fake::new(vec![redirect("/connect/auth?client_id=ORIGIN_JS_SDK&fid=abc", Some("sid=S9; Path=/")), token_ok()]);
        let mut s = session();
        assert_eq!(token(&http, &mut s).unwrap(), "TOKEN");
        let sent = http.sent();
        assert_eq!(sent[1].url, "https://accounts.ea.com/connect/auth?client_id=ORIGIN_JS_SDK&fid=abc");
        assert!(sent[1].headers.contains(&("Cookie", "sid=S9; remid=R1".into())), "the renewed cookie goes along");

        let ends = |to: &str| {
            let http = Fake::new(vec![redirect(to, None)]);
            match token(&http, &mut session()) {
                Ok(t) => t,
                Err(Error::SignedOut) => "out".into(),
                Err(Error::Changed(_)) => "changed".into(),
                Err(Error::Failed(_)) => "failed".into(),
            }
        };
        assert_eq!(ends("https://signin.ea.com/p/juno/login?fid=x"), "out");
        assert_eq!(ends("nucleus:rest#access_token=T2&token_type=Bearer"), "T2");
        assert_eq!(ends("nucleus:rest#error=login_required"), "out");
        assert_eq!(ends("https://accounts.ea.com/connect/auth?error=interaction_required"), "out");
        assert_eq!(ends("https://www.example.com/"), "changed", "the session is kept");
        assert_eq!(ends("https://accounts.ea.com/connect/logout"), "changed", "only connect/auth is followed");
        assert_eq!(ends("https://accounts.ea.com/connect/auth?error=server_error"), "failed", "an error ends the chain");
        assert_eq!(ends("https://accounts.ea.com/connect/auth?error=unsupported_response_type"), "changed");

        let loops: Vec<Response> = (0..=MAX_HOPS).map(|_| redirect("/connect/auth?again", None)).collect();
        assert!(matches!(token(&Fake::new(loops), &mut session()), Err(Error::Failed(_))));
    }

    #[test]
    fn signing_in_checks_the_session_and_takes_the_name() {
        let http = Fake::new(vec![token_ok(), answer(200, r#"{"data":{"me":{"player":{"displayName":" Ana "}}}}"#)]);
        let s = sign_in(&http, r#"{"cookies":[{"name":"sid","value":"S1"}]}"#).unwrap();
        assert_eq!(s.name.as_deref(), Some("Ana"));
        assert_eq!(s.cookies[0].value, "S2");
        assert_eq!(http.sent()[1].headers[0], ("Authorization", "Bearer TOKEN".into()));

        // No name: still signed in.
        let http = Fake::new(vec![token_ok(), answer(500, "")]);
        assert_eq!(sign_in(&http, r#"{"cookies":[{"name":"sid","value":"S1"}]}"#).unwrap().name, None);
        // A session EA does not take is no sign-in.
        let http = Fake::new(vec![answer(200, r#"{"error":"login_required"}"#)]);
        assert!(matches!(sign_in(&http, r#"{"cookies":[{"name":"sid","value":"S1"}]}"#), Err(Error::SignedOut)));
    }

    #[test]
    fn reads_every_page_and_keeps_the_base_games_with_a_product() {
        let offers = answer(
            200,
            &json!({"data": {
                "legacyOffers": [
                    {"offerId": "Origin.OFR.50.0001", "contentId": "1001"},
                    {"offerId": "Origin.OFR.50.0002", "contentId": "1002"},
                    {"offerId": "Origin.OFR.50.0003", "contentId": "1003"},
                    {"offerId": "Origin.OFR.50.0004", "contentId": "bad id"},
                    {"offerId": "Origin.OFR.50.0005", "contentId": null}
                ],
                "gameProducts": {"items": [
                    {"id": "p1", "originOfferId": "Origin.OFR.50.0001", "baseItem": {"title": "Mass Effect\u{2122} Legendary Edition"}},
                    {"id": "1002", "originOfferId": null, "baseItem": {"title": "Unravel Two"}},
                    {"id": "p4", "originOfferId": "Origin.OFR.50.0004", "baseItem": {"title": "Bad"}}
                ]}
            }})
            .to_string(),
        );
        let http = Fake::new(vec![
            token_ok(),
            owned_page(&[("Origin.OFR.50.0001", "BASE_GAME"), ("Origin.OFR.50.0009", "EXTRA_CONTENT")], Some("p2")),
            owned_page(
                &[("Origin.OFR.50.0002", "BASE_GAME"), ("Origin.OFR.50.0003", "BASE_GAME"), ("Origin.OFR.50.0001", "BASE_GAME")],
                None,
            ),
            offers,
        ]);
        let mut s = session();
        let games = read(&http, &mut s).unwrap();
        assert_eq!(
            games,
            [
                OwnedGame { id: "1001".into(), title: "Mass Effect\u{2122} Legendary Edition".into() },
                OwnedGame { id: "1002".into(), title: "Unravel Two".into() },
            ],
            "0003 has no product, 0004 an id that cannot be launched, 0005 none"
        );
        let sent = http.sent();
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[1].json.as_ref().unwrap()["variables"], json!({"limit": 100, "next": null}));
        assert_eq!(sent[2].json.as_ref().unwrap()["variables"], json!({"limit": 100, "next": "p2"}));
        assert_eq!(
            sent[3].json.as_ref().unwrap()["variables"]["offerIds"],
            json!(["Origin.OFR.50.0001", "Origin.OFR.50.0002", "Origin.OFR.50.0003"])
        );
    }

    #[test]
    fn a_next_page_that_repeats_ends_the_list() {
        let http = Fake::new(vec![owned_page(&[("A", "BASE_GAME")], Some("same")), owned_page(&[("B", "BASE_GAME")], Some("same"))]);
        assert_eq!(owned_offers(&http, "T").unwrap(), ["A", "B"]);
        assert_eq!(http.sent().len(), 2);
    }

    #[test]
    fn answers_of_another_shape_are_a_changed_service() {
        let cases = [
            answer(200, r#"{"errors":[{"message":"Cannot query field \"ownedGameProducts\""}],"data":null}"#),
            answer(200, r#"{"data":{"me":{}}}"#),
            answer(200, r#"{"data":{"me":{"ownedGameProducts":{"next":null}}}}"#),
            answer(200, "not json"),
        ];
        for resp in cases {
            let http = Fake::new(vec![resp.clone()]);
            assert!(matches!(owned_offers(&http, "T"), Err(Error::Changed(_))), "{resp:?}");
        }
        for resp in [answer(503, ""), answer(401, r#"{"errors":[{"message":"Not authenticated."}],"data":null}"#)] {
            let http = Fake::new(vec![resp.clone()]);
            assert!(matches!(owned_offers(&http, "T"), Err(Error::Failed(_))), "{resp:?}");
        }
    }

    #[test]
    fn nothing_owned_is_an_empty_list() {
        let http = Fake::new(vec![token_ok(), owned_page(&[], None)]);
        assert!(read(&http, &mut session()).unwrap().is_empty());
        assert_eq!(http.sent().len(), 2, "no details query for no offers");
    }
}
