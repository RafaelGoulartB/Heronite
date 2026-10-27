//! `HERONITE_DEBUG_CLIENT=<id>:<state>` shows one client in a state without
//! Wine, for screenshots: `<id>` is `battlenet`, `ea`, `ubisoft` or
//! `rockstar`, and the state one of
//! - `none` (not set up), `error` (a setup failed), `wine` (waiting for the
//!   Proton download), `download`, `prefix`, `setup` (the installer runs),
//!   `cancelling`;
//! - `ready` (set up, sign-in not seen), `signedout`, `signedin`, `empty`
//!   (signed in, no games), `games` (signed in, three games), `running`,
//!   `gamerunning` (a game of it runs);
//! - `forget` and `settings`, which are `games` with the page's inline
//!   confirmation or its Wine settings dialog open.
//!
//! Nothing is looked at or run for that client; the others are real.

use std::path::PathBuf;

use heronite_clients::setup::Step;
use heronite_clients::{Account, Client, ClientGame, Status};
use heronite_data::clients::LastSync;
use heronite_data::proc::Cancel;

use super::{Activity, ClientState, Notice, Progress};
use crate::services::proton_setup::Setup as WineSetup;

/// The client and state of `HERONITE_DEBUG_CLIENT`.
pub fn debug_client() -> Option<(Client, String)> {
    let v = crate::env::value("HERONITE_DEBUG_CLIENT")?;
    let (id, state) = v.split_once(':').unwrap_or((&v, "games"));
    Some((Client::from_id(id)?, state.to_string()))
}

fn titles(client: Client) -> [&'static str; 3] {
    match client {
        Client::BattleNet => ["Diablo IV", "Overwatch 2", "World of Warcraft"],
        Client::Ea => ["It Takes Two", "Mass Effect Legendary Edition", "Unravel Two"],
        Client::Ubisoft => ["Assassin's Creed Odyssey", "Far Cry 6", "Rayman Legends"],
        Client::Rockstar => ["Grand Theft Auto V", "L.A. Noire", "Red Dead Redemption 2"],
    }
}

/// The facts of `client` in `state`.
pub(super) fn fake(client: Client, state: &str) -> ClientState {
    let mut st = ClientState { status: Some(Status::default()), ..Default::default() };
    let setting_up = |p| Activity::SettingUp(p);
    match state {
        "none" => return st,
        "error" => {
            st.notice = Some(Notice::error("Could not download the installer: the network is unreachable"));
            return st;
        }
        "wine" => {
            let setup = WineSetup::Downloading { version: "GE-Proton10-25".into(), percentage: 42. };
            st.activity = setting_up(Progress::Wine(setup));
            return st;
        }
        "download" => {
            st.activity = setting_up(Progress::Step(Step::Downloading(Some(42.))));
            return st;
        }
        "prefix" => {
            st.activity = setting_up(Progress::Step(Step::CreatingPrefix));
            return st;
        }
        "setup" | "cancelling" => {
            st.activity = setting_up(Progress::Step(Step::Installing));
            if state == "cancelling" {
                let cancel = Cancel::new();
                cancel.request();
                st.cancel = Some(cancel);
            }
            return st;
        }
        _ => {}
    }
    let prefix = PathBuf::from(format!("/home/rafa/Games/Heronite/Prefixes/{}", client.prefix_folder()));
    let games: Vec<ClientGame> = if matches!(state, "ready" | "signedout" | "empty") {
        Vec::new()
    } else {
        titles(client)
            .iter()
            .enumerate()
            .map(|(i, title)| ClientGame {
                id: format!("fake{i}"),
                title: title.to_string(),
                folder: Some(prefix.join("drive_c/Program Files (x86)").join(title)),
                launch: String::new(),
            })
            .collect()
    };
    let account = match state {
        "ready" => Account::Unknown,
        "signedout" => Account::SignedOut,
        _ => Account::SignedIn(Some("player@example.com".into())),
    };
    st.in_collection = games.len();
    st.last = Some(LastSync { at: (chrono::Utc::now() - chrono::Duration::minutes(2)).to_rfc3339(), games: games.len() });
    st.running = matches!(state, "running" | "gamerunning");
    st.game_running = state == "gamerunning";
    st.status = Some(Status { exe: Some(prefix.join("drive_c/client.exe")), prefix: Some(prefix), account, games });
    st
}
