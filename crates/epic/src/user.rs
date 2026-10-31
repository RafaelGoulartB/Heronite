//! Epic account (port of `LegendaryUser`): `legendary auth`, and the
//! `userInfo` kept in `store/config.json` (Heroic's `configStore` shape).

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache_store::CacheStore;
use crate::cmd::Legendary;

/// `UserInfo` of Heroic's `configStore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    /// OS user name.
    pub user: String,
}

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

/// `legendary auth --code`; on success mirrors the account into the config.
pub fn login(data_paths: &DataPaths, legendary: &Legendary, input: &str) -> Result<UserInfo> {
    let Some(code) = extract_code(input) else { bail!(tr!("That does not look like an authorization code")) };
    if code.len() < 30 {
        bail!(tr!("That does not look like an authorization code"));
    }
    let out = legendary.run(&["auth", "--code", &code], &[])?;
    if out.stderr.contains("ERROR: Logging in ") || !out.success() {
        let why = out.error_lines();
        if why.is_empty() {
            bail!(tr!("Failed to login with Legendary"));
        }
        bail!(tr!("Failed to login with Legendary: {error}", error = why));
    }
    match sync_user_info(data_paths)? {
        Some(info) => Ok(info),
        None => bail!(tr!("Logged in, but the account information could not be read")),
    }
}

/// `legendary auth --delete`, then forget the account and the cached library
/// but its installed games (still on disk, still playable offline and
/// uninstallable).
pub fn logout(data_paths: &DataPaths, legendary: &Legendary) -> Result<()> {
    let out = legendary.run(&["auth", "--delete"], &[])?;
    if !out.success() {
        bail!(tr!("Failed to logout: {error}", error = out.error_lines()));
    }
    remove_stored_user_info(data_paths)?;
    let library = CacheStore::library(data_paths);
    let kept = library.path().exists().then(|| library.get("library")).flatten();
    clear_caches(data_paths, legendary);
    if let Some(list) = kept {
        let installed = crate::library::installed_games(data_paths);
        library.set("library", list)?;
        library.clear_keeping_games("library", |n| installed.contains_key(n))?;
    }
    Ok(())
}

/// `clearCache('legendary')`.
pub fn clear_caches(data_paths: &DataPaths, legendary: &Legendary) {
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
    if let Err(err) = legendary.run(&["cleanup"], &[]) {
        log::warn!("legendary cleanup: {err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::*;
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
    fn login_runs_auth_with_the_code() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        // The fake "logs in" by writing user.json.
        let tool = fake_tool(dir.path())
            .otherwise(Reply::new().write_file("${LEGENDARY_CONFIG_PATH}/user.json", r#"{"displayName":"P","account_id":"id1"}"#));
        let lg = Legendary::with_bin(tool.bin(), data_paths.legendary_config_dir());
        let info = login(&data_paths, &lg, CODE).unwrap();
        assert_eq!(info.display_name, "P");
        assert_eq!(tool.calls(), [format!("auth --code {CODE}")]);
        assert!(login(&data_paths, &lg, "short").is_err());
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
        let lg = Legendary::with_bin(fake_tool(dir.path()).bin(), data_paths.legendary_config_dir());
        logout(&data_paths, &lg).unwrap();
        assert_eq!(library.get("library").unwrap(), json!([{"app_name": "Inst"}]));
        assert!(CacheStore::install_info(&data_paths).get("Owned").is_none());
    }

    #[test]
    fn failed_logins_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path().join("h"));
        let tool = fake_tool(dir.path()).otherwise(Reply::new().err("[cli] ERROR: Logging in with code failed").exit(1));
        let err = login(&data_paths, &legendary_of(&tool, dir.path()), CODE).unwrap_err().to_string();
        assert!(err.contains("Failed to login"), "{err}");
    }
}
