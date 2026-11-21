//! The handle every Epic operation goes through: legendary's config folder,
//! where the account, the library and the installs are kept in legendary's
//! files, and the client of Epic's services ([`heronite_epic_api`]).

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use heronite_data::DataPaths;
use heronite_epic_api::{is_sign_in_refused, legendary_config, Client, Session};
use heronite_i18n::tr;

use crate::session::SessionExpired;

/// legendary's config folder and the client that talks to Epic. Cheap to
/// make: nothing is read but the cached launcher settings.
#[derive(Clone)]
pub struct Epic {
    /// `<data>/legendaryConfig/legendary`.
    pub config_dir: PathBuf,
    pub client: Client,
    /// The launcher identity is refreshed from legendary's API (off for a
    /// client of other servers).
    launcher_config: bool,
}

impl std::fmt::Debug for Epic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Epic").field("config_dir", &self.config_dir).finish_non_exhaustive()
    }
}

impl Epic {
    /// The data folder's Epic files, with the launcher identity legendary's
    /// API last published (cached by the library refresh).
    pub fn new(data_paths: &DataPaths) -> Self {
        let config_dir = data_paths.legendary_config_dir();
        Self { client: legendary_config::client_for(&config_dir), config_dir, launcher_config: true }
    }

    /// `client` working on `config_dir` (tests point it at a local server).
    pub fn with_client(config_dir: impl Into<PathBuf>, client: Client) -> Self {
        Self { config_dir: config_dir.into(), client, launcher_config: false }
    }

    /// Fetches the launcher identity legendary's API publishes (the version
    /// Epic's services expect), at most once a day, for the next clients.
    /// Blocking, network; a failure keeps the cached one.
    pub fn refresh_launcher_config(&self) {
        if self.launcher_config {
            legendary_config::refresh(&self.client, &self.config_dir);
        }
    }

    /// The account's sign-in, renewed first when it runs out soon. Signed
    /// out is an error that says so; a sign-in Epic refuses (it is then
    /// forgotten, as legendary does) is a [`SessionExpired`]. Blocking,
    /// network when renewing.
    pub fn session(&self) -> Result<Session> {
        match self.client.ensure_session(&self.config_dir) {
            Ok(Some(session)) => Ok(session),
            Ok(None) => Err(anyhow!(tr!("Log in to your Epic account first"))),
            Err(err) if is_sign_in_refused(&err) => {
                log::warn!("epic: the sign-in was refused: {err:#}");
                Err(anyhow!(SessionExpired))
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::is_expired;

    #[test]
    fn signed_out_and_refused_sign_ins_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let epic = Epic::with_client(dir.path(), Client::new(heronite_epic_api::Endpoints::all_at("http://127.0.0.1:9")));
        let err = epic.session().unwrap_err();
        assert!(!is_expired(&err));
        assert!(err.to_string().contains("Log in"), "{err}");
        // A token that ran out, renewed against a server that is not there:
        // a network failure, never an expired sign-in.
        std::fs::write(
            dir.path().join("user.json"),
            r#"{"access_token": "t", "refresh_token": "r", "account_id": "a", "expires_at": "2000-01-01T00:00:00.000Z"}"#,
        )
        .unwrap();
        let err = epic.session().unwrap_err();
        assert!(!is_expired(&err), "{err:#}");
        assert!(dir.path().join("user.json").exists(), "a failed renewal keeps the sign-in");
    }
}
