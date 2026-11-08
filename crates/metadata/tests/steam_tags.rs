//! Steam store tags against a local mock of the Steam Web API (recorded
//! answers trimmed to the tags of Portal 2 and Hades).

mod common;

use std::time::Duration;

use common::{fixture, Req, Resp, Server};
use heronite_data::paths::DataPaths;
use heronite_data::steam_tags::{SteamTags, SteamTagsFile};
use heronite_metadata::steam_tags::{refresh, BATCH};
use heronite_metadata::{Endpoints, Http};
use serde_json::Value;

const ITEMS: &str = "/steamapi/IStoreBrowseService/GetItems/v1/";
const TAG_LIST: &str = "/steamapi/IStoreService/GetTagList/v1/";

fn server() -> Server {
    Server::start(|req| match req.path() {
        ITEMS => Resp::raw_json(fixture("steam_getitems_tags.json")),
        TAG_LIST => match req.query("language").as_deref() {
            Some("brazilian") => Resp::raw_json(fixture("steam_taglist_br.json")),
            Some("english") => Resp::raw_json(fixture("steam_taglist_en.json")),
            _ => Resp::status(400),
        },
        _ => Resp::status(404),
    })
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn asked_ids(req: &Req) -> Vec<u64> {
    let input: Value = serde_json::from_str(&req.query("input_json").unwrap()).unwrap();
    input["ids"].as_array().unwrap().iter().map(|i| i["appid"].as_u64().unwrap()).collect()
}

#[test]
fn fetches_tags_and_names_once_then_serves_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let s = server();
    let http = Http::new(s.endpoints());

    let r = refresh(&paths, &http, &ids(&["620", "1145360", "999999999", "620", "x"]), "brazilian", Duration::ZERO, &|| false).unwrap();
    assert_eq!((r.apps, r.names, r.unreachable), (3, 2, false));
    let log = s.log.lock().clone();
    let items: Vec<&Req> = log.iter().filter(|r| r.path() == ITEMS).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(asked_ids(items[0]), [620, 1145360, 999999999], "each id once, digits only");
    assert_eq!(s.count(|r| r.path() == TAG_LIST), 2, "the language and the English fallback");
    for req in &log {
        let ua = req.header("user-agent").unwrap_or_default();
        assert!(ua.starts_with("Heronite/"), "says who asks: {ua}");
    }

    let file = SteamTagsFile::load(&paths);
    assert!(file.apps["999999999"].tags.is_empty(), "an unlisted app is remembered without tags");
    let tags = SteamTags::from_file(&file, 5, "brazilian");
    assert_eq!(tags.of("620")[0], "Um Jogador");
    assert_eq!(tags.of("620").len(), 5);
    assert_eq!(tags.of("1145360")[0], "Roguelike de Ação");

    // Fresh: nothing is asked again.
    let again = refresh(&paths, &http, &ids(&["620", "1145360", "999999999"]), "brazilian", Duration::ZERO, &|| false).unwrap();
    assert!(!again.changed());
    assert_eq!(s.log.lock().len(), log.len());
}

#[test]
fn many_games_go_in_batches_and_stopping_keeps_what_was_fetched() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let s = server();
    let http = Http::new(s.endpoints());
    let many: Vec<String> = (1..=BATCH as u64 * 2 + 3).map(|i| i.to_string()).collect();

    let r = refresh(&paths, &http, &many, "english", Duration::ZERO, &|| false).unwrap();
    assert_eq!(r.apps, many.len());
    let sizes: Vec<usize> = s.log.lock().iter().filter(|r| r.path() == ITEMS).map(|r| asked_ids(r).len()).collect();
    assert_eq!(sizes, [BATCH, BATCH, 3]);
    assert_eq!(s.count(|r| r.path() == TAG_LIST), 1, "English only");

    let dir2 = tempfile::tempdir().unwrap();
    let paths2 = DataPaths::at(dir2.path());
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let stop_after_one = || asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 1;
    let r = refresh(&paths2, &http, &many, "english", Duration::ZERO, &stop_after_one).unwrap();
    assert_eq!(r.apps, BATCH, "the first batch is kept");
    assert_eq!(SteamTagsFile::load(&paths2).apps.len(), BATCH);
}

#[test]
fn an_unreachable_steam_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    // Nothing listens on port 9 of localhost.
    let http = Http::new(Endpoints { steam_api: "http://127.0.0.1:9".into(), ..Endpoints::default() });
    let r = refresh(&paths, &http, &ids(&["620"]), "english", Duration::ZERO, &|| false).unwrap();
    assert!(r.unreachable && !r.changed());
    assert!(!paths.steam_tags().exists());
}

#[test]
fn a_server_error_stops_the_pass_without_marking_the_games() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let s = Server::start(|_| Resp::status(503));
    let http = Http::new(s.endpoints());
    let r = refresh(&paths, &http, &ids(&["620"]), "english", Duration::ZERO, &|| false).unwrap();
    assert!(!r.unreachable && r.apps == 0);
    assert!(SteamTagsFile::load(&paths).apps.is_empty(), "asked again next time");
}
