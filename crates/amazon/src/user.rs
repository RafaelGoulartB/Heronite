//! The Amazon account: signing in through nile's two-step flow and the
//! account mirrored into `nile_store/config.json`.
//!
//! 1. `nile auth --login --non-interactive` prints the sign-in page's URL and
//!    the values `register` needs ([`start_login`]).
//! 2. The app shows that page. A successful sign-in redirects to a URL with the
//!    query parameter [`CODE_PARAM`]; its value is the authorization code
//!    ([`extract_code`]).
//! 3. `nile register --code ...` finishes the device registration and syncs the
//!    library ([`register`]).
//!
//! nile keeps the signed-in user in `current_user.json` (`{name, user_id}`) and
//! the tokens encrypted next to it; the tokens are never read here.

use std::path::PathBuf;

use anyhow::{bail, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_epic::cache_store::CacheStore;
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::cmd::Nile;
use crate::{library, updates};

/// Query parameter of the redirect that carries the authorization code.
pub const CODE_PARAM: &str = "openid.oa2.authorization_code";

/// What the sign-in page and `nile register` need.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginRequest {
    /// Amazon's sign-in page.
    pub url: String,
    pub client_id: String,
    pub code_verifier: String,
    pub serial: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub user_id: String,
    /// Given name.
    pub name: String,
}

fn current_user(nile_dir: PathBuf) -> Option<Account> {
    let doc = json_store::read_value(&nile_dir.join("current_user.json")).ok()?;
    let user_id = doc.get("user_id").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_string();
    Some(Account { user_id, name: doc.get("name").and_then(Value::as_str).unwrap_or_default().to_string() })
}

fn nile_dir(paths: &DataPaths) -> PathBuf {
    paths.nile_config_dir().join("nile")
}

/// Whether nile has a signed-in user. Reads a file, runs nothing.
pub fn is_logged_in(paths: &DataPaths) -> bool {
    account(paths).is_some()
}

/// The signed-in user, from `current_user.json`.
pub fn account(paths: &DataPaths) -> Option<Account> {
    current_user(nile_dir(paths))
}

/// Starts a sign-in. `Ok(None)` means nile is already signed in.
pub fn start_login(nile: &Nile) -> Result<Option<LoginRequest>> {
    let out = nile.run(&["auth", "--login", "--non-interactive"])?;
    if let Some(request) = parse_login_request(&out.stdout) {
        return Ok(Some(request));
    }
    if out.stderr.to_lowercase().contains("already logged in") || current_user(nile.data_dir()).is_some() {
        return Ok(None);
    }
    let why = out.error_lines();
    if why.is_empty() {
        bail!(tr!("Could not start the Amazon Games sign-in"));
    }
    bail!(tr!("Could not start the Amazon Games sign-in: {error}", error = why));
}

/// The JSON object nile prints: `{client_id, code_verifier, serial, url}`.
fn parse_login_request(stdout: &str) -> Option<LoginRequest> {
    stdout.lines().map(str::trim).filter(|l| l.starts_with('{')).find_map(|line| {
        let doc: Value = serde_json::from_str(line).ok()?;
        let field = |k: &str| doc.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        Some(LoginRequest {
            url: field("url")?,
            client_id: field("client_id")?,
            code_verifier: field("code_verifier")?,
            serial: field("serial")?,
        })
    })
}

/// The authorization code from what the app or the user has: the bare code,
/// or the URL of the redirect after the sign-in.
pub fn extract_code(input: &str) -> Option<String> {
    let text = input.trim().trim_matches(|c| c == '"' || c == '\'');
    let marker = format!("{CODE_PARAM}=");
    if let Some(start) = text.find(&marker) {
        let rest = &text[start + marker.len()..];
        let value = rest.split(['&', '#']).next().unwrap_or_default();
        return Some(percent_decode(value)).filter(|c| !c.is_empty());
    }
    // A URL without the parameter is not a code.
    let bare = !text.is_empty() && !text.contains(|c: char| c.is_whitespace() || "/?=&:".contains(c));
    bare.then(|| text.to_string())
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `nile register`: finishes the sign-in with the code (or the redirect URL)
/// and mirrors the account. nile also syncs the library, so this can take a
/// while.
pub fn register(paths: &DataPaths, nile: &Nile, request: &LoginRequest, input: &str) -> Result<Account> {
    let Some(code) = extract_code(input) else { bail!(tr!("That does not look like an authorization code")) };
    let out = nile.run(&[
        "register",
        "--code",
        &code,
        "--code-verifier",
        &request.code_verifier,
        "--serial",
        &request.serial,
        "--client-id",
        &request.client_id,
    ])?;
    let text = format!("{}\n{}", out.stderr, out.stdout);
    // nile's own spelling is "Succesfully".
    let registered = text.contains("Succesfully registered a device") || text.contains("Successfully registered a device");
    // A second submit of the same page finds the device already registered.
    let already = text.contains("already logged in") && is_logged_in(paths);
    if !registered && !already {
        let why = out.error_lines();
        if why.is_empty() {
            bail!(tr!("Failed to sign in to Amazon Games"));
        }
        bail!(tr!("Failed to sign in to Amazon Games: {error}", error = why));
    }
    let Some(account) = sync_account(paths)? else { bail!(tr!("Signed in, but the account information could not be read")) };
    if let Err(err) = library::write_library(paths) {
        log::warn!("writing the Amazon library: {err:#}");
    }
    Ok(account)
}

/// Mirrors the account into `nile_store/config.json` (`userData`), or removes
/// it when nile is not signed in.
pub fn sync_account(paths: &DataPaths) -> Result<Option<Account>> {
    let Some(account) = account(paths) else {
        remove_stored_account(paths)?;
        return Ok(None);
    };
    let value = json!({ "name": account.name, "user_id": account.user_id });
    let stored = json_store::read_value(&paths.nile_store("config")).ok().and_then(|d| d.get("userData").cloned());
    // Leave the file (and its mtime) alone when it is already right.
    if stored.as_ref() != Some(&value) {
        json_store::update_value(&paths.nile_store("config"), |doc| {
            doc.as_object_mut().expect("object").insert("userData".into(), value);
            Ok(())
        })?;
    }
    Ok(Some(account))
}

fn remove_stored_account(paths: &DataPaths) -> Result<()> {
    if !paths.nile_store("config").exists() {
        return Ok(());
    }
    json_store::update_value(&paths.nile_store("config"), |doc| {
        if let Some(obj) = doc.as_object_mut() {
            obj.shift_remove("userData");
        }
        Ok(())
    })
}

/// `nile auth --logout`, then forget the account, the library list (but its
/// installed games, still on disk) and the saved updates.
pub fn logout(paths: &DataPaths, nile: &Nile) -> Result<()> {
    // nile deregisters the device online; whatever it says, what we keep is
    // cleaned up below.
    let result = nile.run(&["auth", "--logout"]);
    remove_stored_account(paths)?;
    let installed: Vec<String> = crate::installed::load(paths).into_iter().map(|i| i.id).collect();
    for name in ["nile_library", "nile_install_info"] {
        let store = CacheStore::new(paths, name, None);
        if store.path().exists() {
            let cleared = if name == "nile_library" {
                store.clear_keeping_games("library", |n| installed.iter().any(|i| i == n))
            } else {
                store.clear()
            };
            if let Err(err) = cleared {
                log::warn!("clearing {}: {err:#}", store.path().display());
            }
        }
    }
    updates::clear(paths);
    let out = result?;
    // nile keeps the user when the deregistration failed: still signed in.
    if is_logged_in(paths) {
        let why = out.error_lines();
        if why.is_empty() {
            bail!(tr!("Failed to sign out of Amazon Games"));
        }
        bail!(tr!("Failed to sign out of Amazon Games: {error}", error = why));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::cmd::testing::Env;

    #[cfg(unix)]
    const LOGIN_JSON: &str =
        r#"{"client_id": "cid", "code_verifier": "ver", "serial": "SER", "url": "https://www.amazon.com/ap/signin?x=1&y=2"}"#;
    #[cfg(unix)]
    const USER: &str = r#"{"name": "Ana", "user_id": "amzn1.account.AAA"}"#;

    #[cfg(unix)]
    fn request() -> LoginRequest {
        LoginRequest { url: "u".into(), client_id: "cid".into(), code_verifier: "ver".into(), serial: "SER".into() }
    }

    #[cfg(unix)]
    #[test]
    fn login_request_is_parsed_from_stdout() {
        let env = Env::new();
        let nile = env.nile(&format!("echo '{LOGIN_JSON}'"));
        let req = start_login(&nile).unwrap().unwrap();
        assert_eq!(req.client_id, "cid");
        assert_eq!(req.code_verifier, "ver");
        assert_eq!(req.serial, "SER");
        assert_eq!(req.url, "https://www.amazon.com/ap/signin?x=1&y=2");
        assert_eq!(env.calls(), ["auth --login --non-interactive"]);
    }

    #[cfg(unix)]
    #[test]
    fn already_signed_in_gives_none() {
        let env = Env::new();
        env.write_nile_file("current_user.json", USER);
        let nile = env.nile("echo 'ERROR [AUTH]:\t You are already logged in' >&2");
        assert_eq!(start_login(&nile).unwrap(), None);
        // Nothing printed at all, but a user is stored.
        let nile = env.nile("true");
        assert_eq!(start_login(&nile).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_start_is_an_error() {
        let env = Env::new();
        let nile = env.nile("echo 'ERROR [AUTH]:\t boom' >&2; exit 1");
        let err = start_login(&nile).unwrap_err().to_string();
        assert!(err.contains("boom"), "{err}");
        let nile = env.nile("true");
        assert!(start_login(&nile).is_err());
    }

    #[test]
    fn codes_are_extracted_from_what_users_paste() {
        let url = format!("https://www.amazon.com/ap/maplanding?openid.assoc_handle=amzn_sso&{CODE_PARAM}=ANabc123XYZ&openid.mode=x");
        assert_eq!(extract_code(&url).as_deref(), Some("ANabc123XYZ"));
        assert_eq!(extract_code(&format!("  {url}\n")).as_deref(), Some("ANabc123XYZ"));
        let first = format!("https://www.amazon.com/ap/maplanding?{CODE_PARAM}=AN%2Bz%3D#frag");
        assert_eq!(extract_code(&first).as_deref(), Some("AN+z="));
        assert_eq!(extract_code("ANabc123XYZ").as_deref(), Some("ANabc123XYZ"));
        assert_eq!(extract_code("\"ANabc123XYZ\"\n").as_deref(), Some("ANabc123XYZ"));
        assert_eq!(extract_code("https://www.amazon.com/ap/maplanding?openid.mode=cancel"), None);
        assert_eq!(extract_code(&format!("https://x/?{CODE_PARAM}=&a=b")), None);
        assert_eq!(extract_code("two words"), None);
        assert_eq!(extract_code(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn register_runs_nile_and_mirrors_the_account() {
        let env = Env::new();
        let body = format!(
            "mkdir -p \"$NILE_CONFIG_PATH/nile\"; echo '{USER}' > \"$NILE_CONFIG_PATH/nile/current_user.json\"; \
             echo 'INFO [AUTH]:\t Succesfully registered a device' >&2"
        );
        let nile = env.nile(&body);
        assert!(!is_logged_in(&env.paths));
        let account = register(&env.paths, &nile, &request(), &format!("https://amazon/redirect?{CODE_PARAM}=CODE1&b=c")).unwrap();
        assert_eq!(account, Account { user_id: "amzn1.account.AAA".into(), name: "Ana".into() });
        assert_eq!(env.calls(), ["register --code CODE1 --code-verifier ver --serial SER --client-id cid"]);
        assert!(is_logged_in(&env.paths));
        let stored = json_store::read_value(&env.paths.nile_store("config")).unwrap();
        assert_eq!(stored, json!({"userData": {"name": "Ana", "user_id": "amzn1.account.AAA"}}));
        // The library list exists (empty here) so the Collection treats it as synced.
        assert!(env.paths.nile_library().exists());
    }

    #[cfg(unix)]
    #[test]
    fn register_accepts_the_correct_spelling_too() {
        let env = Env::new();
        let body = format!(
            "mkdir -p \"$NILE_CONFIG_PATH/nile\"; echo '{USER}' > \"$NILE_CONFIG_PATH/nile/current_user.json\"; echo 'Successfully registered a device' >&2"
        );
        assert!(register(&env.paths, &env.nile(&body), &request(), "CODE1").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn register_failures_are_reported() {
        let env = Env::new();
        let nile = env.nile("echo 'ERROR [AUTH]:\t Failed to register device' >&2; exit 1");
        let err = register(&env.paths, &nile, &request(), "CODE1").unwrap_err().to_string();
        assert!(err.contains("Failed to register device"), "{err}");
        assert!(!is_logged_in(&env.paths));
        assert!(!env.paths.nile_store("config").exists());
        // Without a code nile is not even run.
        let calls = env.calls().len();
        assert!(register(&env.paths, &nile, &request(), "not a code at all").is_err());
        assert_eq!(env.calls().len(), calls);
    }

    #[cfg(unix)]
    #[test]
    fn logout_forgets_the_account_and_the_cached_lists() {
        let env = Env::new();
        env.write_nile_file("current_user.json", USER);
        env.write_nile_file("installed.json", r#"[{"id": "g", "version": "1", "path": "/g"}]"#);
        sync_account(&env.paths).unwrap();
        json_store::update_value(&env.paths.nile_store("config"), |d| {
            d["other"] = json!(1);
            Ok(())
        })
        .unwrap();
        CacheStore::new(&env.paths, "nile_library", None).set("library", json!([{"app_name": "g"}, {"app_name": "h"}])).unwrap();
        CacheStore::new(&env.paths, "nile_install_info", None).set("g", json!({"download_size": 1})).unwrap();
        json_store::write(&env.paths.nile_store("updates"), &json!({"updates": ["g"]})).unwrap();

        let nile =
            env.nile("rm -f \"$NILE_CONFIG_PATH/nile/current_user.json\"; echo 'INFO [AUTH]:\t Successfully deregistered a device' >&2");
        logout(&env.paths, &nile).unwrap();
        assert_eq!(env.calls(), ["auth --logout"]);
        assert!(!is_logged_in(&env.paths));
        let stored = json_store::read_value(&env.paths.nile_store("config")).unwrap();
        assert!(stored.get("userData").is_none());
        assert_eq!(stored["other"], 1);
        // The installed game stays listed, the rest of the library goes.
        assert_eq!(CacheStore::new(&env.paths, "nile_library", None).get("library").unwrap(), json!([{"app_name": "g"}]));
        assert_eq!(json_store::read_value(&env.paths.store_cache("nile_install_info")).unwrap(), json!({}));
        assert!(updates::cached_updates(&env.paths).is_empty());
        // Installed games stay.
        assert_eq!(crate::installed::load(&env.paths).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn logout_that_nile_could_not_do_is_an_error() {
        let env = Env::new();
        env.write_nile_file("current_user.json", USER);
        let nile = env.nile("echo 'ERROR [AUTH]:\t offline' >&2; exit 1");
        let err = logout(&env.paths, &nile).unwrap_err().to_string();
        assert!(err.contains("offline"), "{err}");
    }
}
