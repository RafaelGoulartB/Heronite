//! The user's Xbox account (optional): with it, the Xbox games get the play
//! time and the last play Xbox network counted (also outside Heronite, on
//! any device), their achievements and gamerscore, an image, and the PC games
//! played on the account that are not installed here
//! ([`crate::details`]).
//!
//! * [`auth`]: the sign-in, on Microsoft's own page, and its tokens;
//! * [`account`]: the sealed file that keeps them, refreshing, signing out;
//! * [`titlehub`]: the account's titles and their minutes;
//! * [`http`]: the requests, which the tests answer from fixtures.
//!
//! Heronite signs in as its own app registration ([`client_id`]).

pub mod account;
pub mod auth;
pub mod http;
pub mod titlehub;

use std::fmt;

use anyhow::Result;
use heronite_data::DataPaths;

use crate::details::{self, Applied};
use http::Http;

/// The application (client) id of Heronite's own registration with
/// Microsoft (personal accounts, public client, redirect
/// `https://login.live.com/oauth20_desktop.srf`), registered by the
/// maintainer (issue #173). Another program's registration is not to be
/// borrowed.
pub const DEFAULT_CLIENT_ID: &str = "c8d70a4c-7469-4f4a-8dea-f98a3d12fe0c";

/// Overrides [`DEFAULT_CLIENT_ID`] at build time
/// (`HERONITE_XBOX_CLIENT_ID=<id> cargo build`) or at run time (the same
/// variable), for a fork with its own registration.
pub const CLIENT_ID_VAR: &str = "HERONITE_XBOX_CLIENT_ID";

/// Heronite's client id: the override when it looks like one (a GUID), else
/// the default.
pub fn client_id() -> Option<String> {
    let run = std::env::var(CLIENT_ID_VAR).ok();
    let built = option_env!("HERONITE_XBOX_CLIENT_ID").map(str::to_string);
    [run, built]
        .into_iter()
        .flatten()
        .map(|id| id.trim().to_ascii_lowercase())
        .find(|id| guid(id))
        .or_else(|| Some(DEFAULT_CLIENT_ID.to_string()))
}

/// `8-4-4-4-12` hexadecimal digits.
fn guid(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12]) && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The account must sign in again: no sign-in saved, or Microsoft no longer
/// takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedOut;

impl fmt::Display for SignedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not signed in to Xbox")
    }
}

impl std::error::Error for SignedOut {}

/// What a sync of the account did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    /// Titles the account has played.
    pub titles: usize,
    pub applied: Applied,
}

/// Reads the account's titles and minutes and writes what they say about
/// the Collection's Xbox games. `installed` are the package family names of
/// the games installed here: those the history does not have (never played
/// on the account) are asked for by name. Blocking, network.
pub fn sync(paths: &DataPaths, http: &dyn Http, client_id: &str, installed: &[String]) -> Result<Synced> {
    let xsts = account::xsts(paths, http, client_id)?;
    let mut titles = titlehub::history(http, &xsts)?;
    let missing: Vec<String> = installed
        .iter()
        .filter(|pfn| !titles.iter().any(|t| t.pfn.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(pfn))))
        .cloned()
        .collect();
    // Never played: nothing more than what the title is.
    match titlehub::by_package(http, &xsts, &missing) {
        Ok(more) => titles.extend(more.into_iter().filter(|t| !titles.iter().any(|h| h.title_id == t.title_id)).collect::<Vec<_>>()),
        Err(e) => log::warn!("xbox: titles of the packages never played: {e:#}"),
    }
    // Minutes only for what can show them: PC games with an entry, or
    // about to get one.
    let played: Vec<usize> = (0..titles.len()).filter(|i| titles[*i].is_pc_game() || titles[*i].pfn.is_some()).collect();
    let mut asked: Vec<titlehub::Title> = played.iter().map(|i| titles[*i].clone()).collect();
    match titlehub::add_minutes(http, &xsts, &mut asked) {
        Ok(()) => {
            for (i, t) in played.iter().zip(asked) {
                titles[*i].minutes = t.minutes;
            }
        }
        // The rest still helps; the last minutes known stay.
        Err(e) => log::warn!("xbox: play time: {e:#}"),
    }
    let applied = details::apply(paths, &titles)?;
    Ok(Synced { titles: titles.len(), applied })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sign-in saved, then one sync: the history, the never-played packages,
    /// the minutes; the Collection shows them. Every answer is a fixture.
    #[cfg(windows)]
    #[test]
    fn a_sync_reads_the_account_and_fills_the_collection() {
        use crate::live::auth::tests::{CLIENT, LIVE_OK, USER_OK, XSTS_OK};
        use crate::live::http::{Body, Fake};
        use crate::live::titlehub::tests::HISTORY;

        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let game = crate::XboxGame {
            family: "Fable.SkywardLantern_abc123".into(),
            app_id: "Game".into(),
            title: "Skyward Lantern".into(),
            folder: dir.path().join("Content"),
            store_id: None,
            title_id: None,
            package: "Fable.SkywardLantern_1.0.0.0_x64__abc123".into(),
        };
        let never = "Fable.NeverPlayed_abc123".to_string();
        crate::collection::sync(&paths, std::slice::from_ref(&game)).unwrap();
        let sign_in = auth::SignIn::new(CLIENT).unwrap();
        account::finish(&paths, &Fake::new(&[(200, LIVE_OK), (200, USER_OK), (200, XSTS_OK)]), CLIENT, &sign_in, "code").unwrap();
        let expire = |paths: &DataPaths| {
            let mut a = account::load(paths).unwrap();
            a.xsts.not_after = chrono::Utc::now() - chrono::Duration::hours(1);
            account::save(paths, &a).unwrap();
        };
        expire(&paths);

        let stats = r#"{"statlistscollection": [{"stats": [{"titleid": "1784935649", "value": "754"}]}]}"#;
        let never_title = r#"{"titles": [{"titleId": "42", "pfn": "Fable.NeverPlayed_abc123", "name": "Never Played", "type": "Game", "devices": ["PC"]}]}"#;
        // The saved XSTS token has ended: renewed first.
        let fake = Fake::new(&[(200, LIVE_OK), (200, USER_OK), (200, XSTS_OK), (200, HISTORY), (200, never_title), (200, stats)]);
        let synced = sync(&paths, &fake, CLIENT, &[game.family.clone(), never]).unwrap();
        assert_eq!(synced.titles, 6);
        assert_eq!(synced.applied.added, 2, "Cloud Harbor and the never-played package's title");
        let sent = fake.sent();
        let Body::Json(batch) = &sent[4].body else { panic!() };
        assert_eq!(batch["pfns"], serde_json::json!(["Fable.NeverPlayed_abc123"]), "only what the history lacks");

        let lib = heronite_data::Library::load(&paths).unwrap();
        assert_eq!(lib.get("sideload_xbox_fable_skywardlantern_abc123").unwrap().playtime_minutes, 754);

        // Microsoft no longer takes the sign-in: signed out, the file is gone.
        expire(&paths);
        let revoked = Fake::new(&[(400, r#"{"error":"invalid_grant"}"#)]);
        assert!(sync(&paths, &revoked, CLIENT, &[]).unwrap_err().is::<SignedOut>());
        assert!(account::load(&paths).is_none());
    }

    #[test]
    fn client_ids_must_look_like_one() {
        assert!(guid(DEFAULT_CLIENT_ID));
        assert!(guid("0b9f6e5a-1c2d-4e3f-8a9b-0c1d2e3f4a5b"));
        assert!(!guid("0b9f6e5a1c2d4e3f8a9b0c1d2e3f4a5b"));
        assert!(!guid("0b9f6e5a-1c2d-4e3f-8a9b-0c1d2e3f4a5"));
        assert!(!guid("zzzzzzzz-1c2d-4e3f-8a9b-0c1d2e3f4a5b"));
        assert!(!guid(""));
    }
}
