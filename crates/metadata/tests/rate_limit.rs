//! A rate limit or server error is not remembered as "not found".

mod common;

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use common::{fixture, Resp, Server};
use heronite_data::paths::DataPaths;
use heronite_metadata::MetadataService;
use serde_json::{json, Value};

fn appdetails_hits(s: &Server) -> usize {
    s.count(|r| r.path() == "/steam/api/appdetails")
}

#[test]
fn steam_details_are_asked_again_after_a_429_and_a_503() {
    let dir = tempfile::tempdir().unwrap();
    let status = Arc::new(AtomicU16::new(429));
    let st = status.clone();
    let server = Server::start(move |req| match (req.path(), st.load(Ordering::SeqCst)) {
        ("/steam/api/appdetails", 200) => {
            let all: Value = serde_json::from_str(&fixture("steam_appdetails_400.json")).unwrap();
            Resp::json(json!({ req.query("appids").unwrap(): all["400"].clone() }))
        }
        ("/steam/api/appdetails", code) => Resp::status(code),
        _ => Resp::status(404),
    });
    let svc = MetadataService::with_endpoints(DataPaths::at(dir.path()), server.endpoints());

    assert!(svc.steam_details("400", Some("en")).is_none());
    // Only the first language is tried: English would be limited too.
    assert_eq!(appdetails_hits(&server), 1);
    status.store(503, Ordering::SeqCst);
    assert!(svc.steam_details("400", Some("en")).is_none());
    assert_eq!(appdetails_hits(&server), 2, "not blocked by a remembered miss");

    status.store(200, Ordering::SeqCst);
    assert_eq!(svc.steam_details("400", Some("en")).map(|d| d.name).as_deref(), Some("Portal"));
}

#[test]
fn a_plain_404_is_still_remembered_as_missing() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start(|_| Resp::status(404));
    let svc = MetadataService::with_endpoints(DataPaths::at(dir.path()), server.endpoints());
    assert!(svc.steam_details("400", Some("en")).is_none());
    let first = appdetails_hits(&server);
    assert!(svc.steam_details("400", Some("en")).is_none());
    assert_eq!(appdetails_hits(&server), first);
}
