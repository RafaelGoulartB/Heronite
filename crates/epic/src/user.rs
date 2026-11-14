//! Epic account (port of `LegendaryUser`): signing in and out (legendary's
//! `user.json`), and the `userInfo` kept in `store/config.json` (Heroic's
//! `configStore` shape).

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache_store::CacheStore;
use crate::client::Epic;

/// `UserInfo` of Heroic's `configStore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    /// OS user name.
    pub user: String,
}

/// Epic's login for the login window (with [`LOGIN_USER_AGENT`]): Epic picks
/// the launcher's client and redirects to `https://localhost/launcher/authorized?code=...`
/// once signed in. See `EPIC_LOGIN_URL` in the `heronite-epic-login` helper.
pub const LOGIN_URL: &str = "https://www.epicgames.com/id/login?responseType=code";
/// Page that shows the authorization code in the browser (SID login).
pub const BROWSER_LOGIN_URL: &str = "https://legendary.gl/epiclogin";
/// User agent Heroic's webview uses for the Epic login.
pub const LOGIN_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) EpicGamesLauncher";

fn user_json(data_paths: &DataPaths) -> std::path::PathBuf {
    data_paths.legendary_config_dir().join("user.json")
}

/// `LegendaryUser.isLoggedIn`.
pub fn is_logged_in(data_paths: &DataPaths) -> bool {
    user_json(data_paths).exists()
}

fn os_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default()
}

/// Account name and id from legendary's `user.json` (tokens are never read
/// into anything that could be shown).
pub fn read_user_info(data_paths: &DataPaths) -> Option<UserInfo> {
    let doc = json_store::read_value(&user_json(data_paths)).ok()?;
    let text = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_string);
    Some(UserInfo { account_id: text("account_id")?, display_name: text("displayName").unwrap_or_default(), user: os_user() })
}

/// `getUserInfo`: reads the account and mirrors it into `configStore.userInfo`
/// (removing it when logged out).
pub fn sync_user_info(data_paths: &DataPaths) -> Result<Option<UserInfo>> {
    if !is_logged_in(data_paths) {
        remove_stored_user_info(data_paths)?;
        return Ok(None);
    }
    let Some(info) = read_user_info(data_paths) else { return Ok(None) };
    let value = serde_json::to_value(&info)?;
    // Do not touch the file (and its mtime) when it is already right.
    let stored = json_store::read_value(&data_paths.store_config()).ok().and_then(|d| d.get("userInfo").cloned());
    if stored.as_ref() != Some(&value) {
        json_store::update_value(&data_paths.store_config(), |doc| {
            let obj = doc.as_object_mut().expect("object");
            obj.insert("userInfo".into(), value);
            Ok(())
        })?;
    }
    Ok(Some(info))
}

fn remove_stored_user_info(data_paths: &DataPaths) -> Result<()> {
    if !data_paths.store_config().exists() {
        return Ok(());
    }
    json_store::update_value(&data_paths.store_config(), |doc| {
        if let Some(obj) = doc.as_object_mut() {
            obj.shift_remove("userInfo");
        }
        Ok(())
    })
}

/// Pulls the authorization code out of what the user pasted: the bare code,
/// the JSON page of `legendary.gl/epiclogin` / Epic's redirect, or a
/// `...?code=...` URL.
pub fn extract_code(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with('{') {
        let doc: Value = serde_json::from_str(text).ok()?;
        for key in ["authorizationCode", "code"] {
            if let Some(code) = doc.get(key).and_then(Value::as_str).filter(|c| !c.is_empty()) {
                return Some(code.to_string());
            }
        }
        return None;
    }
    if text.starts_with("http") {
        let query = text.split_once('?')?.1;
        return query
            .split('&')
            .find_map(|kv| kv.strip_prefix("code="))
            .map(|c| c.split('#').next().unwrap_or(c).to_string())
            .filter(|c| !c.is_empty());
    }
    let bare = text.trim_matches('"');
    (!bare.contains(char::is_whitespace)).then(|| bare.to_string())
}

/// Signs in with the authorization code; on success mirrors the account
/// into the config. Blocking, network.
pub fn login(data_paths: &DataPaths, epic: &Epic, input: &str) -> Result<UserInfo> {
    let Some(code) = extract_code(input) else { bail!(tr!("That does not look like an authorization code")) };
    if code.len() < 30 {
        bail!(tr!("That does not look like an authorization code"));
    }
    if let Err(err) = epic.client.login_with_code(&epic.config_dir, &code) {
        log::error!("epic: login failed: {err:#}");
        bail!(tr!("Failed to log in to Epic Games: {error}", error = format!("{err:#}")));
    }
    // The launcher identity legendary's API publishes, once a day.
    epic.refresh_launcher_config();
    match sync_user_info(data_paths)? {
        Some(info) => Ok(info),
        None => bail!(tr!("Logged in, but the account information could not be read")),
    }
}

/// Ends the session at Epic and forgets it, then the cached library but its
/// installed games (still on disk, still playable offline and
/// uninstallable).
pub fn logout(data_paths: &DataPaths, epic: &Epic) -> Result<()> {
    epic.client.logout(&epic.config_dir).map_err(|e| anyhow::anyhow!(tr!("Failed to logout: {error}", error = format!("{e:#}"))))?;
    remove_stored_user_info(data_paths)?;
    let library = CacheStore::library(data_paths);
    let kept = library.path().exists().then(|| library.get("library")).flatten();
    clear_caches(data_paths, epic);
    if let Some(list) = kept {
        let installed = crate::library::installed_games(data_paths);
        library.set("library", list)?;
        library.clear_keeping_games("library", |n| installed.contains_key(n))?;
    }
    Ok(())
}

/// `clearCache('legendary')`, and what `legendary cleanup` removes: the
/// stored manifests of builds no longer installed and the temporary files.
pub fn clear_caches(data_paths: &DataPaths, epic: &Epic) {
    for store in [
        CacheStore::install_info(data_paths),
        CacheStore::library(data_paths),
        CacheStore::new(data_paths, "legendary_gameinfo", Some(60 * 6)),
    ] {
        if store.path().exists() {
            if let Err(err) = store.clear() {
                log::warn!("clearing {}: {err:#}", store.path().display());
            }
        }
    }
    let removed = heronite_epic_api::installed::clean_manifests(&epic.config_dir);
    if removed > 0 {
        log::info!("epic: {removed} stored manifest(s) of builds no longer installed removed");
    }
    heronite_epic_api::installed::clean_tmp(&epic.config_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::testing::Server;
    use serde_json::json;

    const CODE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn codes_are_extracted_from_what_users_paste() {
        assert_eq!(extract_code(CODE).as_deref(), Some(CODE));
        assert_eq!(extract_code(&format!("  \"{CODE}\"\n")).as_deref(), Some(CODE));
        let blob =
            format!(r#"{{"redirectUrl":"https://localhost/launcher/authorized?code={CODE}","sid":null,"authorizationCode":"{CODE}"}}"#);
        assert_eq!(extract_code(&blob).as_deref(), Some(CODE));
        assert_eq!(extract_code(&format!("https://localhost/launcher/authorized?code={CODE}")).as_deref(), Some(CODE));
        assert_eq!(extract_code("{}"), None);
        assert_eq!(extract_code("two words"), None);
        assert_eq!(extract_code(""), None);
    }

    fn setup_user(data_paths: &DataPaths) {
        std::fs::create_dir_all(data_paths.legendary_config_dir()).unwrap();
        std::fs::write(
            user_json(data_paths),
            json!({"displayName": "Player One", "account_id": "abc123", "access_token": "SECRET", "refresh_token": "SECRET2"}).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn user_info_is_mirrored_into_the_config_store() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store")).unwrap();
        std::fs::write(data_paths.store_config(), "{\n\t\"language\": \"en\"\n}").unwrap();
        assert!(sync_user_info(&data_paths).unwrap().is_none());
        setup_user(&data_paths);
        let info = sync_user_info(&data_paths).unwrap().unwrap();
        assert_eq!((info.account_id.as_str(), info.display_name.as_str()), ("abc123", "Player One"));
        let stored = json_store::read_value(&data_paths.store_config()).unwrap();
        assert_eq!(stored["language"], "en");
        assert_eq!(stored["userInfo"]["displayName"], "Player One");
        assert_eq!(stored["userInfo"]["account_id"], "abc123");
        assert!(stored["userInfo"].get("access_token").is_none());
        // Logging out of legendary removes it again.
        std::fs::remove_file(user_json(&data_paths)).unwrap();
        assert!(sync_user_info(&data_paths).unwrap().is_none());
        assert!(json_store::read_value(&data_paths.store_config()).unwrap().get("userInfo").is_none());
    }

    #[test]
    fn login_signs_in_with_the_code() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let server = Server::start();
        server.route(
            "/account/api/oauth/token",
            200,
            json!({"access_token": "tok", "refresh_token": "ref", "expires_at": "2099-01-01T00:00:00.000Z", "account_id": "id1", "displayName": "P"})
                .to_string(),
        );
        let epic = server.epic(&data_paths.legendary_config_dir());
        let info = login(&data_paths, &epic, CODE).unwrap();
        assert_eq!((info.display_name.as_str(), info.account_id.as_str()), ("P", "id1"));
        assert!(is_logged_in(&data_paths));
        assert!(login(&data_paths, &epic, "short").is_err());
    }

    #[test]
    fn logout_keeps_installed_games_listed() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        setup_user(&data_paths);
        std::fs::write(data_paths.legendary_config_dir().join("installed.json"), r#"{"Inst": {"app_name": "Inst"}}"#).unwrap();
        let library = CacheStore::library(&data_paths);
        library.set("library", json!([{"app_name": "Inst"}, {"app_name": "Owned"}])).unwrap();
        CacheStore::install_info(&data_paths).set("Owned", json!({})).unwrap();
        let server = Server::start();
        logout(&data_paths, &server.epic(&data_paths.legendary_config_dir())).unwrap();
        assert!(!is_logged_in(&data_paths));
        assert_eq!(library.get("library").unwrap(), json!([{"app_name": "Inst"}]));
        assert!(CacheStore::install_info(&data_paths).get("Owned").is_none());
    }

    #[test]
    fn failed_logins_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let server = Server::start();
        server.route(
            "/account/api/oauth/token",
            400,
            r#"{"errorCode": "errors.com.epicgames.account.oauth.authorization_code_not_found"}"#,
        );
        let err = login(&data_paths, &server.epic(&data_paths.legendary_config_dir()), CODE).unwrap_err().to_string();
        assert!(err.contains("Failed to log in"), "{err}");
        assert!(!is_logged_in(&data_paths));
    }
}
