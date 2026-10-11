//! Real GitHub fetch + one real download/extract/remove through the manager.
//! Ignored by default (network, ~300 MB). Run against a fixture only:
//!   HERONITE_WINE_REAL=/tmp/heronite-fixture-wine cargo test -p heronite-wine --test real_network -- --ignored --nocapture

use std::time::{Duration, Instant};

use heronite_data::DataPaths;
use heronite_wine::{store, JobEvent, JobPhase, Repository, WineManager};

const SMALL: &str = "cachyos-10.0-20250702-slr";

#[test]
#[ignore]
fn refresh_install_remove_against_fixture() {
    let dir = std::env::var("HERONITE_WINE_REAL").expect("set HERONITE_WINE_REAL to a fixture dir");
    assert!(dir.starts_with("/tmp/heronite-fixture"), "refusing to run outside /tmp/heronite-fixture*");
    let data_paths = DataPaths::at(&dir);

    let out = store::refresh(&data_paths, &Repository::LINUX, 50).expect("refresh");
    eprintln!("refreshed: {} releases, errors {:?}", out.releases.len(), out.errors);
    assert!(out.releases.iter().any(|r| r.version == "GE-Proton-latest"));
    assert!(out.releases.iter().any(|r| r.version == "Proton-CachyOS-latest"));
    assert!(out.errors.is_empty());

    let info = store::read_releases(&data_paths)
        .into_iter()
        .find(|r| r.version == SMALL)
        .expect("small build in cache");
    let manager = WineManager::new(data_paths.clone());
    assert!(manager.start_install(info));
    assert!(!manager.start_install(store::read_releases(&data_paths).into_iter().find(|r| r.version == SMALL).unwrap()), "no duplicate job");
    let start = Instant::now();
    let mut last_print = Instant::now();
    let mut events = Vec::new();
    while manager.job(SMALL).is_some() {
        if last_print.elapsed() > Duration::from_secs(5) {
            last_print = Instant::now();
            match manager.job(SMALL).map(|j| j.phase) {
                Some(JobPhase::Downloading { percentage, speed, eta_secs, .. }) => {
                    eprintln!("  {percentage:.1}% {:.1} MB/s eta {eta_secs:?}", speed / 1e6)
                }
                other => eprintln!("  {other:?}"),
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        assert!(start.elapsed() < Duration::from_secs(1500), "too slow");
    }
    events.extend(manager.take_events());
    eprintln!("events: {events:?} after {:?}", start.elapsed());
    assert!(events.iter().any(|e| matches!(e, JobEvent::Installed { .. })), "{events:?}");
    let installed = data_paths.tools_dir().join("proton").join(SMALL);
    assert!(installed.join("proton").exists());
    let rel = store::read_releases(&data_paths).into_iter().find(|r| r.version == SMALL).unwrap();
    assert!(rel.installed() && rel.disksize > 500_000_000, "{rel:?}");
    let found = heronite_wine::list_installations(&data_paths);
    assert!(found.iter().any(|w| w.name == SMALL && w.kind == "proton"));

    assert!(manager.start_remove(rel));
    while manager.job(SMALL).is_some() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let events = manager.take_events();
    assert!(events.iter().any(|e| matches!(e, JobEvent::Removed { .. })), "{events:?}");
    assert!(!installed.exists());
    assert!(!store::read_releases(&data_paths).into_iter().find(|r| r.version == SMALL).unwrap().installed());
}
