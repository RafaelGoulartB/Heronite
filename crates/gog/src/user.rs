//! The GOG account: `gog_store/auth.json` holds the tokens (gogdl's file), `gog_store/config.json`
//! the name shown in the app.

use std::collections::HashSet;

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use heronite_epic::session::SessionExpired;
use heronite_gog_api::{auth, SignInExpired, CLIENT_ID};
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::api;

pub const LOGIN_URL: &str = "https://auth.gog.com/auth?client_id=46899977096215655&redirect_uri=https%3A%2F%2Fembed.gog.com%2Fon_login_success%3Forigin%3Dclient&response_type=code&layout=galaxy";
/// The login page navigates here on success; the `code` parameter is the authorization code.
pub const REDIRECT_PREFIX: &str = "https://embed.gog.com/on_login_success";

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub user_id: String,
    pub username: String,
}

#[derive(Clone)]
pub struct Credentials {
    pub access_token: String,
    pub user_id: String,
    /// Comet needs it, see `heronite_launch::galaxy`.
    pub refresh_token: String,
}

/// Without the tokens: they must not reach a log.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("user_id", &self.user_id).finish_non_exhaustive()
    }
}

/// Names of the caches that depend on the account.
const ACCOUNT_CACHES: [&str; 3] = [LIBRARY_CACHE, "gog_api_info", "gog_install_info"];
const LIBRARY_CACHE: &str = "gog_library";

/// Tokens present and the app's own config says logged in. No network, no process.
pub fn is_logged_in(paths: &DataPaths) -> bool {
    let has_tokens = json_store::read_value(&paths.gog_auth()).is_ok_and(|auth| auth.get(CLIENT_ID).is_some_and(Value::is_object));
    let flagged =
        json_store::read_value(&paths.gog_store("config")).is_ok_and(|c| c.get("isLoggedIn").and_then(Value::as_bool) == Some(true));
    has_tokens && flagged
}

/// The account saved at login (from `config.json`).
pub fn account(paths: &DataPaths) -> Option<Account> {
    let config = json_store::read_value(&paths.gog_store("config")).ok()?;
    let user = config.get("userData")?;
    let text = |k: &str| user.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Account { user_id: text("galaxyUserId")?, username: text("username").unwrap_or_default() })
}

/// The authorization code out of what the user pasted: the bare code or the
/// `on_login_success?...&code=...` address.
pub fn extract_code(input: &str) -> Option<String> {
    let text = input.trim().trim_matches('"');
    if text.is_empty() {
        return None;
    }
    if text.starts_with("http") {
        let query = text.split_once('?')?.1;
        let query = query.split('#').next().unwrap_or(query);
        return query.split('&').find_map(|kv| kv.strip_prefix("code=")).filter(|c| !c.is_empty()).map(str::to_string);
    }
    (!text.contains(char::is_whitespace)).then(|| text.to_string())
}

/// Fresh tokens of the account, refreshed first when they ran out. Tokens
/// GOG no longer refreshes are a [`SessionExpired`]. Blocking, network when
/// refreshing.
pub fn credentials(paths: &DataPaths) -> Result<Credentials> {
    match auth::credentials(&paths.gog_auth()) {
        Ok(c) => Ok(Credentials { access_token: c.access_token, user_id: c.user_id, refresh_token: c.refresh_token }),
        Err(err) if err.downcast_ref::<SignInExpired>().is_some() => {
            log::warn!("gog: the sign-in was refused: {err:#}");
            Err(anyhow!(SessionExpired).context(tr!("Your GOG sign-in expired: log in again")))
        }
        Err(err) => Err(err),
    }
}

/// [`credentials`] that gives up after `limit` (a refresh may hang on a bad
/// network). The refresh then goes on in the background.
pub fn credentials_within(paths: &DataPaths, limit: std::time::Duration) -> Result<Credentials> {
    let (tx, rx) = std::sync::mpsc::channel();
    let paths = paths.clone();
    std::thread::spawn(move || {
        let _ = tx.send(credentials(&paths));
    });
    rx.recv_timeout(limit).map_err(|_| anyhow!(tr!("GOG did not answer in time")))?
}

/// The account saved in `config.json` for a GOG user document.
fn account_from_user(user_id: &str, doc: &Value) -> (Account, Option<String>) {
    let username = doc.get("username").and_then(Value::as_str).unwrap_or_default().to_string();
    let avatar = doc.get("avatar");
    let avatar_url =
        ["medium_2x", "medium"].iter().find_map(|k| avatar.and_then(|a| a.get(*k)).and_then(Value::as_str)).map(str::to_string);
    (Account { user_id: user_id.to_string(), username }, avatar_url)
}

/// Exchanges the code of the login page for tokens, reads the user's name
/// and saves the account. Blocking, network.
pub fn login(paths: &DataPaths, input: &str) -> Result<Account> {
    let Some(code) = extract_code(input) else { bail!(tr!("That does not look like an authorization code")) };
    let creds = auth::login(&paths.gog_auth(), &code)?;
    let url = format!("https://users.gog.com/users/{}", creds.user_id);
    let doc = api::get_json(&url, &creds.access_token).with_context(|| tr!("Logged in, but the account information could not be read"))?;
    let (account, avatar_url) = account_from_user(&creds.user_id, &doc);
    save_account(paths, &account, avatar_url.as_deref())?;
    Ok(account)
}

fn save_account(paths: &DataPaths, account: &Account, avatar_url: Option<&str>) -> Result<()> {
    json_store::update_value(&paths.gog_store("config"), |doc| {
        let obj = doc.as_object_mut().expect("object");
        obj.insert("isLoggedIn".into(), json!(true));
        obj.insert(
            "userData".into(),
            json!({"username": account.username, "galaxyUserId": account.user_id, "avatarUrl": avatar_url.unwrap_or("")}),
        );
        Ok(())
    })
}

/// Forgets the account and the data fetched with it. Installed games stay
/// (`installed.json` and gogdl's manifests are kept), and so does their entry
/// in the library, so they can still be played and uninstalled.
pub fn logout(paths: &DataPaths) -> Result<()> {
    match std::fs::remove_file(paths.gog_auth()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("removing the GOG tokens"),
    }
    if paths.gog_store("config").exists() {
        json_store::update_value(&paths.gog_store("config"), |doc| {
            *doc = json!({});
            Ok(())
        })?;
    }
    let installed: HashSet<String> = crate::installed::load(paths).into_iter().map(|i| i.app_name).collect();
    for name in ACCOUNT_CACHES {
        let store = CacheStore::new(paths, name, None);
        if store.path().exists() {
            let cleared = if name == LIBRARY_CACHE { store.clear_keeping_games("games", |n| installed.contains(n)) } else { store.clear() };
            if let Err(err) = cleared {
                log::warn!("clearing {}: {err:#}", store.path().display());
            }
        }
    }
    crate::updates::clear(paths);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::setup;

    const CODE: &str = "AbC123-xyz_Code0123456789";

    #[test]
    fn codes_are_extracted_from_what_users_paste() {
        assert_eq!(extract_code(CODE).as_deref(), Some(CODE));
        assert_eq!(extract_code(&format!("  \"{CODE}\"\n")).as_deref(), Some(CODE));
        assert_eq!(extract_code(&format!("{REDIRECT_PREFIX}?origin=client&code={CODE}")).as_deref(), Some(CODE));
        assert_eq!(extract_code(&format!("{REDIRECT_PREFIX}?code={CODE}&x=1#frag")).as_deref(), Some(CODE));
        assert_eq!(extract_code(REDIRECT_PREFIX), None);
        assert_eq!(extract_code("https://x/?code="), None);
        assert_eq!(extract_code("two words"), None);
        assert_eq!(extract_code(""), None);
    }

    #[test]
    fn login_url_points_at_the_redirect() {
        assert!(LOGIN_URL.starts_with("https://auth.gog.com/auth?client_id=46899977096215655"));
        assert!(LOGIN_URL.contains("embed.gog.com%2Fon_login_success"));
    }

    #[test]
    fn user_documents_become_accounts() {
        let doc = json!({"id": "4242", "username": "player", "avatar": {"medium": "https://a/m.jpg", "medium_2x": "https://a/m2.jpg"}});
        let (account, avatar) = account_from_user("4242", &doc);
        assert_eq!(account, Account { user_id: "4242".into(), username: "player".into() });
        assert_eq!(avatar.as_deref(), Some("https://a/m2.jpg"));
        let (_, avatar) = account_from_user("1", &json!({"username": "x", "avatar": {"medium": "https://a/m.jpg"}}));
        assert_eq!(avatar.as_deref(), Some("https://a/m.jpg"));
    }

    #[test]
    fn login_state_needs_tokens_and_the_flag() {
        let (_dir, paths) = setup();
        assert!(!is_logged_in(&paths));
        std::fs::create_dir_all(paths.gog_auth().parent().unwrap()).unwrap();
        std::fs::write(paths.gog_auth(), r#"{"46899977096215655":{"access_token":"t","user_id":"4242"}}"#).unwrap();
        assert!(!is_logged_in(&paths));
        save_account(&paths, &Account { user_id: "4242".into(), username: "player".into() }, Some("https://a/m.jpg")).unwrap();
        assert!(is_logged_in(&paths));
        assert_eq!(account(&paths), Some(Account { user_id: "4242".into(), username: "player".into() }));
        let config = json_store::read_value(&paths.gog_store("config")).unwrap();
        assert_eq!(config["userData"]["avatarUrl"], "https://a/m.jpg");
    }

    #[test]
    fn what_is_not_a_code_is_refused_before_asking_gog() {
        let (_dir, paths) = setup();
        let err = login(&paths, "two words").unwrap_err().to_string();
        assert!(err.contains("does not look like"), "{err}");
        assert!(!paths.gog_auth().exists());
    }

    #[test]
    fn credentials_come_from_the_auth_file() {
        let (_dir, paths) = setup();
        assert!(credentials(&paths).is_err());
        std::fs::create_dir_all(paths.gog_auth().parent().unwrap()).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        std::fs::write(
            paths.gog_auth(),
            format!(
                r#"{{"46899977096215655":{{"access_token":"tok","refresh_token":"r","user_id":7,"expires_in":3600,"loginTime":{now}}}}}"#
            ),
        )
        .unwrap();
        let c = credentials(&paths).unwrap();
        assert_eq!((c.access_token.as_str(), c.user_id.as_str(), c.refresh_token.as_str()), ("tok", "7", "r"));
        let c = credentials_within(&paths, std::time::Duration::from_secs(30)).unwrap();
        assert_eq!(c.access_token, "tok");
    }

    #[test]
    fn tokens_that_cannot_be_refreshed_are_an_expired_session() {
        let (_dir, paths) = setup();
        std::fs::create_dir_all(paths.gog_auth().parent().unwrap()).unwrap();
        // Expired, with no refresh token to renew it.
        std::fs::write(paths.gog_auth(), r#"{"46899977096215655":{"access_token":"t","user_id":"4242","expires_in":1,"loginTime":0}}"#)
            .unwrap();
        let err = credentials(&paths).unwrap_err();
        assert!(heronite_epic::session::is_expired(&err), "{err:#}");
        assert!(err.to_string().contains("expired"), "{err}");
    }

    #[test]
    fn logout_clears_the_account_but_keeps_installed_games() {
        let (_dir, paths) = setup();
        std::fs::create_dir_all(paths.gog_auth().parent().unwrap()).unwrap();
        std::fs::write(paths.gog_auth(), r#"{"46899977096215655":{}}"#).unwrap();
        std::fs::write(paths.gog_installed(), r#"{"installed":[{"appName":"1"}]}"#).unwrap();
        save_account(&paths, &Account { user_id: "1".into(), username: "p".into() }, None).unwrap();
        CacheStore::new(&paths, "gog_library", None).set("games", json!([{"app_name": "1"}, {"app_name": "2"}])).unwrap();
        CacheStore::new(&paths, "gog_api_info", None).set("gog_1", json!({})).unwrap();
        crate::updates::save(&paths, &["1".to_string()]).unwrap();

        logout(&paths).unwrap();
        assert!(!paths.gog_auth().exists());
        assert!(!is_logged_in(&paths));
        assert_eq!(json_store::read_value(&paths.gog_store("config")).unwrap(), json!({}));
        // The installed game stays listed; the rest of the library goes.
        assert_eq!(CacheStore::new(&paths, "gog_library", None).get("games").unwrap(), json!([{"app_name": "1"}]));
        assert!(CacheStore::new(&paths, "gog_api_info", None).get("gog_1").is_none());
        assert!(crate::updates::cached_updates(&paths).is_empty());
        assert!(paths.gog_installed().exists());
        logout(&paths).unwrap();
    }
}
