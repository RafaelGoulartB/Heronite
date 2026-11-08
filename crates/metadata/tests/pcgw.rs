//! PCGamingWiki against a local mock. The answers are made up in the shape
//! of the MediaWiki API (no wiki text is copied: it is CC BY-NC-SA).

mod common;

use std::time::{Duration, Instant};

use common::{Resp, Server};
use heronite_data::paths::DataPaths;
use heronite_data::pcgw::{PcgwCache, PcgwQuery, Support};
use heronite_metadata::MetadataService;
use serde_json::json;

const PAGE: &str = "{{Infobox game\n|steam appid  = 4242\n|gogcom id    = \n|hltb         = 777\n}}\n\
{{Fixbox|description=A fix}}\n\
{{Game data/saves|Windows|{{p|localappdata}}\\Example\\}}\n\
{{Video\n|ultrawidescreen = true\n|hdr = limited\n}}\n\
{{Input\n|controller support = true\n|full controller = true\n}}\n";

const OTHER: &str = "{{Infobox game\n|steam appid  = 9999\n}}\n{{Video\n|hdr = true\n}}\n";

fn parse(title: &str, text: &str) -> Resp {
    Resp::json(json!({"parse": {"title": title, "pageid": 1, "revid": 1234, "wikitext": text}}))
}

fn server() -> Server {
    Server::start(|req| {
        assert!(req.header("user-agent").unwrap_or("").starts_with("Heronite/"), "honest User-Agent");
        match (req.path(), req.query("appid").as_deref(), req.query("action").as_deref()) {
            ("/pcgw/api/appid.php", Some("4242"), _) => Resp::redirect("https://www.pcgamingwiki.com/wiki/Example_Quest"),
            ("/pcgw/api/appid.php", Some("503"), _) => Resp::status(503),
            ("/pcgw/api/appid.php", _, _) => Resp::status(404),
            ("/pcgw/w/api.php", _, Some("parse")) => match req.query("page").as_deref() {
                Some("Example Quest") => parse("Example Quest", PAGE),
                Some("Other Game") => parse("Other Game", OTHER),
                _ => Resp::json(json!({"error": {"code": "missingtitle", "info": "x"}})),
            },
            ("/pcgw/w/api.php", _, Some("query")) => match req.query("titles").as_deref() {
                Some("Other Game") => Resp::json(json!({"query": {"pages": [{"pageid": 2, "title": "Other Game"}]}})),
                Some(t) => Resp::json(json!({"query": {"pages": [{"title": t, "missing": true}]}})),
                None => Resp::status(400),
            },
            _ => Resp::status(404),
        }
    })
}

fn service(server: &Server) -> (tempfile::TempDir, MetadataService) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    std::fs::create_dir_all(paths.pcgw().parent().unwrap()).unwrap();
    let svc = MetadataService::with_endpoints(paths, server.endpoints());
    svc.set_pc_gaming_wiki_gap(Duration::ZERO);
    (dir, svc)
}

fn query(steam: Option<&str>, title: &str) -> PcgwQuery {
    PcgwQuery { steam_id: steam.map(str::to_owned), gog_id: None, title: title.into() }
}

#[test]
fn steam_id_redirect_then_page_then_cache() {
    let s = server();
    let (_dir, svc) = service(&s);
    let q = query(Some("4242"), "Example Quest");
    let got = svc.pc_gaming_wiki(&q).unwrap();
    assert!(!got.from_cache);
    let f = got.facts.unwrap();
    assert_eq!((f.page.as_str(), f.revid, f.fixes), ("Example Quest", Some(1234), 1));
    assert_eq!((f.ultrawide, f.hdr, f.controller), (Some(Support::Yes), Some(Support::Limited), Some(Support::Yes)));
    assert_eq!(f.hltb_id.as_deref(), Some("777"));
    assert_eq!(f.save_paths().unwrap().paths, [r"%LOCALAPPDATA%\Example\"]);
    assert_eq!(f.url(), "https://www.pcgamingwiki.com/wiki/Example_Quest");
    // The redirect was read, not followed: one appid.php and one parse request.
    assert_eq!(s.count(|r| r.path() == "/pcgw/api/appid.php"), 1);
    assert_eq!(s.count(|r| r.path().starts_with("/wiki/")), 0);
    let requests = s.count(|_| true);
    // Fresh: from the file, no request.
    let again = svc.pc_gaming_wiki(&q).unwrap();
    assert!(again.from_cache && again.facts.is_some());
    assert_eq!(s.count(|_| true), requests);
    // The data crate reads it.
    let cache = PcgwCache::load(&svc.paths);
    assert_eq!(cache.facts(&q).unwrap().fixes, 1);
    // A refresh asks again.
    assert!(!svc.refresh_pc_gaming_wiki(&q).unwrap().from_cache);
}

#[test]
fn title_fallback_refuses_another_game_and_remembers_no_page() {
    let s = server();
    let (_dir, svc) = service(&s);
    // Steam has no page for 5555; the title's page lists app 9999: not ours.
    let got = svc.pc_gaming_wiki(&query(Some("5555"), "Other Game")).unwrap();
    assert!(got.facts.is_none());
    // Without a Steam id the title's page is taken.
    let got = svc.pc_gaming_wiki(&query(None, "Other Game")).unwrap();
    assert_eq!(got.facts.unwrap().hdr, Some(Support::Yes));
    // No page at all: remembered, not asked again.
    assert!(svc.pc_gaming_wiki(&query(None, "Nowhere")).unwrap().facts.is_none());
    let n = s.count(|_| true);
    let again = svc.pc_gaming_wiki(&query(None, "Nowhere")).unwrap();
    assert!(again.from_cache && again.facts.is_none());
    assert_eq!(s.count(|_| true), n);
}

#[test]
fn server_errors_and_no_network_write_nothing() {
    let s = server();
    let (_dir, svc) = service(&s);
    assert!(svc.pc_gaming_wiki(&query(Some("503"), "Busy")).is_err());
    assert!(PcgwCache::load(&svc.paths).entry(&query(Some("503"), "Busy")).is_none());
    let dir = tempfile::tempdir().unwrap();
    let off = MetadataService::with_endpoints(
        DataPaths::at(dir.path()),
        heronite_metadata::Endpoints { pcgamingwiki: "http://127.0.0.1:9".into(), ..Default::default() },
    );
    assert!(off.pc_gaming_wiki(&query(Some("4242"), "Example Quest")).is_err());
    assert!(!off.paths.pcgw().exists());
}

#[test]
fn requests_are_spaced() {
    let s = server();
    let (_dir, svc) = service(&s);
    svc.set_pc_gaming_wiki_gap(Duration::from_millis(300));
    let start = Instant::now();
    // appid.php then the page: two requests, one gap between them.
    svc.pc_gaming_wiki(&query(Some("4242"), "Example Quest")).unwrap();
    assert!(start.elapsed() >= Duration::from_millis(300));
}
