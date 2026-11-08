//! Settings → Integrations → Sunshine, kept in `AppPaths::sunshine_prefs()`
//! (`sunshine.json`): they belong to this computer, not to the library.
//!
//! - `enabled`: off until the user turns it on.
//! - `address`: Sunshine's web UI, [`DEFAULT_ADDRESS`] unless changed.
//! - `username`: the web UI's user name.
//! - `games`: `installed` (every installed game) or `favourites` (the
//!   installed favourites).
//! - `stopOnQuit`: quitting the application in Moonlight closes the game.
//! - `consoleEntry`: one more application that opens console mode.
//! - `password`: on Windows, the password sealed with DPAPI for this user
//!   (base64). Never in the clear: other systems keep it in the desktop's
//!   keyring instead, and this key stays empty.
//! - `certificate`: for a Sunshine on another computer, the fingerprint of
//!   the certificate it had the first time (trust on first use, see
//!   [`crate::pin`]); empty until then, and for this computer.

use anyhow::Result;
use base64::Engine;
use heronite_data::json_store;
use heronite_data::paths::AppPaths;
use serde_json::{json, Value};

pub use crate::api::DEFAULT_ADDRESS;

/// Which games go to Sunshine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Games {
    #[default]
    Installed,
    Favourites,
}

impl Games {
    pub fn as_str(self) -> &'static str {
        match self {
            Games::Installed => "installed",
            Games::Favourites => "favourites",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "installed" => Some(Games::Installed),
            "favourites" => Some(Games::Favourites),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub enabled: bool,
    pub address: String,
    pub username: String,
    pub games: Games,
    pub stop_on_quit: bool,
    pub console_entry: bool,
    /// The DPAPI-sealed password, base64 (Windows); empty when none is kept here.
    pub sealed_password: String,
    /// The pinned certificate fingerprint; empty for none yet.
    pub certificate: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            enabled: false,
            address: DEFAULT_ADDRESS.into(),
            username: String::new(),
            games: Games::Installed,
            stop_on_quit: true,
            console_entry: true,
            sealed_password: String::new(),
            certificate: String::new(),
        }
    }
}

impl Prefs {
    /// A missing or invalid file (or key) gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        let v = json_store::read_value(&paths.sunshine_prefs()).unwrap_or(Value::Null);
        let d = Self::default();
        let flag = |k: &str, default: bool| v.get(k).and_then(Value::as_bool).unwrap_or(default);
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Self {
            enabled: flag("enabled", d.enabled),
            address: text("address").filter(|a| !a.trim().is_empty()).unwrap_or(d.address),
            username: text("username").unwrap_or_default(),
            games: text("games").as_deref().and_then(Games::parse).unwrap_or_default(),
            stop_on_quit: flag("stopOnQuit", d.stop_on_quit),
            console_entry: flag("consoleEntry", d.console_entry),
            sealed_password: text("password").unwrap_or_default(),
            certificate: text("certificate").unwrap_or_default(),
        }
    }

    /// Writes atomically, keeping keys this struct does not model.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        json_store::update_value(&paths.sunshine_prefs(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            doc["enabled"] = json!(self.enabled);
            doc["address"] = json!(self.address);
            doc["username"] = json!(self.username);
            doc["games"] = json!(self.games.as_str());
            doc["stopOnQuit"] = json!(self.stop_on_quit);
            doc["consoleEntry"] = json!(self.console_entry);
            doc["password"] = json!(self.sealed_password);
            doc["certificate"] = json!(self.certificate);
            Ok(())
        })
    }
}

/// `password` sealed for this Windows user, as stored in `password`. Fails
/// on systems without DPAPI: nothing is ever kept in the clear.
pub fn seal(password: &str) -> std::io::Result<String> {
    let sealed = heronite_platform::secret::protect(password.as_bytes())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(sealed))
}

/// What [`seal`] made, when this user made it on this computer.
pub fn unseal(stored: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(stored.trim()).ok()?;
    let plain = heronite_platform::secret::unprotect(&bytes).ok()?;
    String::from_utf8(plain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    #[test]
    fn defaults_then_round_trip_keeping_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let d = Prefs::load(&p);
        assert_eq!(d, Prefs::default());
        assert_eq!(d.address, "https://localhost:47990");
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.sunshine_prefs(), r#"{"other": 1, "games": "nope"}"#).unwrap();
        assert_eq!(Prefs::load(&p).games, Games::Installed);
        let prefs = Prefs {
            enabled: true,
            address: "https://pc:47990".into(),
            username: "me".into(),
            games: Games::Favourites,
            stop_on_quit: false,
            console_entry: false,
            sealed_password: "c2VhbGVk".into(),
            certificate: "AB:CD".into(),
        };
        prefs.save(&p).unwrap();
        assert_eq!(Prefs::load(&p), prefs);
        assert_eq!(json_store::read_value(&p.sunshine_prefs()).unwrap()["other"], 1);
    }

    #[cfg(windows)]
    #[test]
    fn the_password_is_sealed_not_stored() {
        let sealed = seal("hunter2-secret").unwrap();
        assert!(!sealed.contains("hunter2"));
        let raw = base64::engine::general_purpose::STANDARD.decode(&sealed).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"hunter2"));
        assert_eq!(unseal(&sealed).as_deref(), Some("hunter2-secret"));
        assert_eq!(unseal("bm90IGEgYmxvYg=="), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn sealing_fails_without_dpapi() {
        assert!(seal("x").is_err());
    }
}
