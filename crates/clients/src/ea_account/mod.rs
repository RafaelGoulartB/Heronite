//! The user's EA account (optional): with it, the games the account owns
//! that are not installed come into the Collection too, as not installed
//! ([`crate::owned`]). Without it, or when EA stops answering the way it
//! does today, only the installed games are there, as before.
//!
//! It works like the EA libraries of Lutris (`lutris/services/ea_app.py`)
//! and Playnite (its former `OriginLibrary`): the user signs in on EA's own
//! page in the login window (`heronite-epic-login --store ea`), which hands
//! back the cookies of that web session for `accounts.ea.com`, never a
//! password. Each read turns them into a short-lived token
//! (`connect/auth`, the `ORIGIN_JS_SDK` client of EA's web pages, with
//! `prompt=none`) and asks EA's GraphQL service, the one the EA app and its
//! website use, for the games the account owns ([`api`]). Every request
//! only reads; nothing is bought, installed or changed in the account.
//!
//! Where the [`Session`] is kept is the app's: sealed for this Windows user,
//! or in the desktop's keyring.

pub mod api;
pub mod http;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use api::{read, sign_in};

/// A cookie of the EA web session.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}

/// The signed-in account: the web session's cookies for `accounts.ea.com`
/// and the name EA shows for it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// Never prints the cookies (a log line or a panic would hold the session).
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.cookies.iter().map(|c| c.name.as_str()).collect();
        f.debug_struct("Session").field("cookies", &names).field("name", &self.name).finish()
    }
}

impl Session {
    /// What the login window prints: `{"cookies":[{"name":..,"value":..}]}`.
    pub fn from_login_window(text: &str) -> Option<Self> {
        let session: Session = serde_json::from_str(text.trim()).ok()?;
        let cookies: Vec<Cookie> = session.cookies.into_iter().filter(|c| valid_cookie(&c.name, &c.value)).collect();
        (!cookies.is_empty()).then_some(Self { cookies, name: None })
    }

    /// The `Cookie` header.
    fn header(&self) -> String {
        self.cookies.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; ")
    }

    /// Applies a `Set-Cookie` line of `accounts.ea.com` (EA may renew the
    /// session as it hands out a token). Whether anything changed.
    fn apply(&mut self, line: &str) -> bool {
        let mut parts = line.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else { return false };
        let (name, value) = (name.trim(), value.trim().trim_matches('"'));
        if !valid_cookie(name, "x") {
            return false;
        }
        let gone = value.is_empty()
            || parts.any(|a| {
                let a = a.trim().to_ascii_lowercase();
                a == "max-age=0" || a.starts_with("max-age=-") || a.starts_with("expires=thu, 01 jan 1970")
            });
        let at = self.cookies.iter().position(|c| c.name == name);
        match (at, gone) {
            (Some(i), true) => {
                self.cookies.remove(i);
                true
            }
            (Some(i), false) if self.cookies[i].value != value && valid_cookie(name, value) => {
                self.cookies[i].value = value.to_string();
                true
            }
            (None, false) if valid_cookie(name, value) => {
                self.cookies.push(Cookie { name: name.to_string(), value: value.to_string() });
                true
            }
            _ => false,
        }
    }
}

/// A cookie that fits in a `Cookie` header as it is.
fn valid_cookie(name: &str, value: &str) -> bool {
    let token = |s: &str, extra: &[char]| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic() && !matches!(c, ';' | ',' | '"' | '\\') && !extra.contains(&c))
    };
    token(name, &['=']) && token(value, &[]) && name.len() <= 256 && value.len() <= 8192
}

/// Why the account could not be read.
#[derive(Debug)]
pub enum Error {
    /// EA no longer takes the sign-in: the user signs in again.
    SignedOut,
    /// EA answered in a way this version does not know (its service
    /// changed); what was expected.
    Changed(String),
    /// The network, or EA's service, failed this time.
    Failed(anyhow::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedOut => f.write_str("EA asks to sign in again"),
            Self::Changed(what) => write!(f, "EA answered in an unexpected way ({what})"),
            Self::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(pairs: &[(&str, &str)]) -> Session {
        Session { cookies: pairs.iter().map(|(n, v)| Cookie { name: n.to_string(), value: v.to_string() }).collect(), name: None }
    }

    #[test]
    fn the_login_window_output_keeps_only_usable_cookies() {
        let text = r#"{"cookies":[{"name":"sid","value":"abc"},{"name":"bad name","value":"x"},{"name":"remid","value":"r;1"},{"name":"_nx","value":"q.1"}]}"#;
        let s = Session::from_login_window(text).unwrap();
        assert_eq!(s.header(), "sid=abc; _nx=q.1");
        assert!(Session::from_login_window(r#"{"cookies":[]}"#).is_none());
        assert!(Session::from_login_window("code").is_none());
    }

    #[test]
    fn set_cookie_lines_renew_add_and_drop_cookies() {
        let mut s = session(&[("sid", "old"), ("remid", "keep")]);
        assert!(s.apply("sid=new; Path=/; Secure; HttpOnly"));
        assert!(!s.apply("sid=new; Path=/"), "the same value changes nothing");
        assert!(s.apply("_nx=1; Max-Age=3600"));
        assert!(s.apply("remid=; Max-Age=0"));
        assert!(s.apply("_nx=gone; Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
        assert!(!s.apply("garbage"));
        assert_eq!(s.header(), "sid=new");
    }

    #[test]
    fn debug_output_hides_the_values() {
        let s = session(&[("sid", "SECRET")]);
        let shown = format!("{s:?}");
        assert!(shown.contains("sid") && !shown.contains("SECRET"));
    }
}
