//! The real Comet beside a stand-in GOG game (`sleep`): started before it,
//! listening on GOG Galaxy's port, stopped with Ctrl+C after it, its log
//! written without the tokens. Ignored by default (downloads Comet, about
//! 15 MB, and needs the port free):
//!   cargo test -p heronite-launch --test real_comet -- --ignored --nocapture

// `sleep` plays the game.
#![cfg(unix)]

use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::DataPaths;
use heronite_launch::galaxy::{port_taken, GalaxyAccount, GalaxyLogin};
use heronite_launch::steam::SteamLocator;
use heronite_launch::store_tools::COMET;
use heronite_launch::{Command, LaunchEvent, LaunchPlan, LaunchRequest, Launcher, ProcessPlan};

struct Account;

impl GalaxyAccount for Account {
    fn login(&self) -> Result<Option<GalaxyLogin>, String> {
        Ok(Some(GalaxyLogin {
            access_token: "SECRET-ACCESS".into(),
            refresh_token: "SECRET-REFRESH".into(),
            user_id: "1".into(),
            username: "tester".into(),
        }))
    }
}

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore = "downloads Comet and runs it"]
fn the_real_comet_serves_the_game_and_stops_after_it() {
    assert!(!port_taken(), "something already listens on port 9977");
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("data"));
    COMET.ensure(&paths, &heronite_launch::SystemProbe, &heronite_launch::net::SystemNet, &|| false).unwrap();
    let (tx, rx) = channel();
    let steam = SteamLocator::detect(&Default::default(), &heronite_launch::SystemProbe);
    let launcher = Launcher::system(paths.clone(), steam, tx);
    launcher.set_galaxy_account(Arc::new(Account));
    let mut plan =
        ProcessPlan { command: Command { program: "sleep".into(), args: vec!["3".into()], ..Default::default() }, ..Default::default() };
    plan.prep.galaxy = true;
    let req = LaunchRequest { app_name: "g".into(), runner: "gog".into(), title: "Game".into(), plan: LaunchPlan::Native(plan) };
    launcher.launch(req).unwrap();
    loop {
        match rx.recv_timeout(Duration::from_secs(30)).unwrap() {
            LaunchEvent::Running { .. } => break,
            ev @ LaunchEvent::Error { .. } => panic!("{ev:?}"),
            _ => {}
        }
    }
    wait_for("Comet to listen", 10, port_taken);
    loop {
        if let LaunchEvent::Exited { .. } = rx.recv_timeout(Duration::from_secs(30)).unwrap() {
            break;
        }
    }
    // Ctrl+C: it shuts down (and would upload) well before being ended.
    wait_for("Comet to stop", 10, || !port_taken());
    let log = std::fs::read_to_string(paths.logs_dir().join("runners/comet.log")).unwrap();
    println!("{log}");
    assert!(log.contains("--username tester") && log.contains("Listening on port 9977"), "{log}");
    assert!(!log.contains("SECRET"), "{log}");
}
