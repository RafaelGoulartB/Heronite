//! A store sign-in that stopped working (its refresh token expired or was
//! revoked): only signing in again helps. Each store's crate turns what its
//! tool or API says into [`SessionExpired`], so the app can ask for a new
//! sign-in instead of showing the raw failure.

use std::fmt;

use heronite_i18n::tr;

/// The saved sign-in was refused. Wrapped with `.context(..)` for the text the
/// user sees; find it with [`is_expired`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionExpired;

impl fmt::Display for SessionExpired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(tr!("The sign-in expired: log in again"))
    }
}

impl std::error::Error for SessionExpired {}

/// Whether `err`, or what caused it, is a [`SessionExpired`].
pub fn is_expired(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.is::<SessionExpired>())
}

/// What legendary logs when Epic refuses the stored login (legendary deletes
/// its `user.json` then). A network failure ("HTTP request for login
/// failed") is not one of them.
pub fn legendary_expired(output: &str) -> bool {
    let text = output.to_ascii_lowercase();
    ["stored credentials are no longer valid", "invalid_refresh_token", "invalid_grant"].iter().any(|p| text.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, Context};

    #[test]
    fn expired_sessions_are_found_through_the_context() {
        let err = Err::<(), _>(anyhow!(SessionExpired)).context("You are not signed in").unwrap_err();
        assert!(is_expired(&err));
        assert_eq!(err.to_string(), "You are not signed in");
        assert!(!is_expired(&anyhow!("HTTP 500")));
    }

    #[test]
    fn legendary_messages() {
        assert!(legendary_expired("[Core] ERROR: Stored credentials are no longer valid! Please login again."));
        assert!(legendary_expired("errors.com.epicgames.account.auth_token.invalid_refresh_token"));
        assert!(!legendary_expired("[Core] ERROR: HTTP request for login failed: ConnectionError(), please try again later."));
        assert!(!legendary_expired("[cli] ERROR: Login failed, cannot continue!"));
    }
}
