//! The lists of extensions: the committed `extensions/metadata/index.json`
//! against its folders, and the client against the fixture server (fetch,
//! the copy kept offline, what is refused, updates, a list removed,
//! installing an entry through to a gather).

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use heronite_data::json_store;
use heronite_data::paths::DataPaths;
use heronite_metadata::*;
use heronite_metadata_ext::index::{self, IndexError};
use heronite_metadata_ext::lists::{self, ListError, Lists, Origin};
use heronite_metadata_ext::manifest::Manifest;
use heronite_metadata_ext::{ExtensionError, ExtensionsExt, InstallOutcome};
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::{endpoints, example, fixture, Resp, Server};

const LIST: &str = "https://lists.example.org/index.json";
const OTHER: &str = "https://other.example.net/heronite.json";
const FILES: &str = "https://lists.example.org/ext/";

fn repo_extensions() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/metadata")
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The committed list, with its addresses under [`FILES`].
fn list_json() -> Value {
    let text = std::fs::read_to_string(repo_extensions().join("index.json")).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    for e in v["extensions"].as_array_mut().unwrap() {
        let id = e["id"].as_str().unwrap().to_owned();
        for file in ["source.toml", "source.js"] {
            e["files"][file]["url"] = json!(format!("{FILES}{id}/{file}"));
        }
    }
    v
}

fn entry_mut<'a>(list: &'a mut Value, id: &str) -> &'a mut Value {
    list["extensions"].as_array_mut().unwrap().iter_mut().find(|e| e["id"] == id).unwrap()
}

#[test]
fn the_committed_list_matches_the_folders() {
    let text = std::fs::read(repo_extensions().join("index.json")).unwrap();
    let list = index::parse(&text).expect("extensions/metadata/index.json reads");
    let raw: Value = serde_json::from_slice(&text).unwrap();
    assert_eq!(raw["type"], "metadata", "the committed list says what it holds");
    let mut folders: Vec<PathBuf> =
        std::fs::read_dir(repo_extensions()).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("source.toml").is_file()).collect();
    folders.sort();
    let ids: Vec<String> = folders.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    let listed: Vec<&str> = list.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(listed, ids, "one entry per folder, in order: run python3 scripts/extensions-index.py");
    for (dir, entry) in folders.iter().zip(&list.entries) {
        let toml = std::fs::read(dir.join("source.toml")).unwrap();
        let js = std::fs::read(dir.join("source.js")).unwrap();
        let m = Manifest::parse(std::str::from_utf8(&toml).unwrap()).unwrap();
        let stale = format!("{} is out of date: run python3 scripts/extensions-index.py", entry.id);
        assert_eq!(entry.version, m.version, "{stale}");
        assert_eq!((entry.manifest.sha256.as_str(), entry.manifest.size), (sha(&toml).as_str(), toml.len() as u64), "{stale}");
        assert_eq!((entry.script.sha256.as_str(), entry.script.size), (sha(&js).as_str(), js.len() as u64), "{stale}");
        assert_eq!(entry.fields, m.provides, "{stale}");
        assert_eq!(entry.hosts, m.network.hosts.iter().map(|h| h.text()).collect::<Vec<_>>(), "{stale}");
        assert_eq!((entry.kind, entry.adult, &entry.name), (m.kind, m.adult, &m.name), "{stale}");
        assert!(entry.manifest.url.starts_with("https://raw.githubusercontent.com/"), "{stale}");
    }
}

#[test]
// On Windows `python3` may be the Microsoft Store's stub, which only says
// how to install Python; the Linux job runs the generator.
#[cfg_attr(windows, ignore = "the generator runs on the Linux job")]
fn the_generator_writes_the_committed_list() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/extensions-index.py");
    match std::process::Command::new("python3").arg(&script).arg("--check").output() {
        Ok(out) => assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr)),
        // Without Python the test above still compares the list with the folders.
        Err(e) => eprintln!("python3 not found ({e}): the generator was not run"),
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    server: Server,
    svc: MetadataService,
    list: Arc<Mutex<Value>>,
    /// Files served in place of the repository's (`<id>/<file>`).
    files: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    lists: Lists,
}

fn write(path: &Path, v: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    json_store::write(path, &v).unwrap();
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path());
    let games = json!({
        "portal2": {"appName": "portal2", "runner": "sideload", "title": "Portal 2", "playniteId": "", "source": "manual", "launchKind": "executable"},
    });
    write(&paths.local_meta(), json!({ "games": games }));
    write(&paths.collection_settings(), json!({"metadata": {}}));
    let list = Arc::new(Mutex::new(list_json()));
    let files: Arc<Mutex<Vec<(String, Vec<u8>)>>> = Arc::default();
    let (served, swapped) = (list.clone(), files.clone());
    let server = Server::start(move |req| {
        let path = req.path();
        if path == "/lists.example.org/index.json" || path == "/other.example.net/heronite.json" {
            return Resp::json(served.lock().clone());
        }
        if let Some(rest) = path.strip_prefix("/lists.example.org/ext/") {
            if let Some((_, bytes)) = swapped.lock().iter().find(|(p, _)| p == rest) {
                return Resp::image(bytes.clone());
            }
            return match std::fs::read(repo_extensions().join(rest)) {
                Ok(bytes) => Resp::image(bytes),
                Err(_) => Resp::status(404),
            };
        }
        match path {
            "/en.wikipedia.org/w/rest.php/v1/search/title" if req.query("q").as_deref() == Some("Portal 2") => {
                Resp::raw_json(fixture("wikipedia_search_portal2.json"))
            }
            "/en.wikipedia.org/api/rest_v1/page/summary/Portal_2" => Resp::raw_json(fixture("wikipedia_summary_en_portal2.json")),
            "/lutris/api/games" => Resp::json(json!({"results": []})),
            _ => Resp::status(404),
        }
    });
    let svc = MetadataService::with_endpoints(
        paths.clone(),
        endpoints(&server, &["lists.example.org", "other.example.net", "en.wikipedia.org", "pt.wikipedia.org"]),
    );
    Setup { _dir: dir, paths, server, svc, list, files, lists: Lists::new(LIST) }
}

impl Setup {
    fn entry(&self, id: &str) -> index::IndexEntry {
        let copy = self.lists.read(&self.svc, LIST);
        copy.index.unwrap().get(id).cloned().unwrap()
    }

    /// A service over the same data folder whose network answers nothing.
    fn offline(&self) -> MetadataService {
        let e = Endpoints { origins: vec![("https://lists.example.org".into(), "http://127.0.0.1:9".into())], ..Default::default() };
        MetadataService::with_endpoints(self.paths.clone(), e)
    }
}

#[test]
fn a_list_is_read_kept_and_shown_offline_with_its_age() {
    let s = setup();
    let copies = s.lists.refresh(&s.svc);
    assert_eq!(copies.len(), 1, "the app's own list only, until the user adds one");
    let copy = &copies[0];
    assert!(copy.builtin && copy.error.is_none());
    let ids: Vec<&str> = copy.index.as_ref().unwrap().entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["vndb", "wikipedia"]);
    let read = s.server.log.lock().iter().find(|r| r.path() == "/lists.example.org/index.json").cloned().unwrap();
    assert!(read.header("user-agent").is_some_and(|ua| ua.starts_with("Heronite/")));

    // Offline: the copy kept, how old it is, and why it was not read now.
    let cache = std::fs::read_dir(s.paths.extension_lists_cache_dir()).unwrap().flatten().find(|e| e.path().is_file()).unwrap().path();
    let mut kept: Value = serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
    let three_days_ago = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 3 * 86_400;
    kept["readAt"] = json!(three_days_ago);
    std::fs::write(&cache, kept.to_string()).unwrap();
    let offline = s.offline();
    let copy = s.lists.read(&offline, LIST);
    assert!(matches!(copy.error, Some(ListError::Unreachable(_))), "{:?}", copy.error);
    assert_eq!(copy.index.as_ref().unwrap().entries.len(), 2);
    assert_eq!(copy.read_at, Some(UNIX_EPOCH + Duration::from_secs(three_days_ago)));
    // Without the network at all, the same copy.
    assert_eq!(s.lists.cached(&offline)[0].index, copy.index);

    // No copy and no network: nothing to show, and why.
    let fresh = tempfile::tempdir().unwrap();
    let alone = MetadataService::with_endpoints(DataPaths::at(fresh.path()), offline.http.endpoints.clone());
    let copy = s.lists.read(&alone, LIST);
    assert!(copy.index.is_none() && copy.read_at.is_none() && matches!(copy.error, Some(ListError::Unreachable(_))));
}

#[test]
fn what_a_list_may_not_be() {
    let s = setup();
    assert_eq!(lists::fetch_list(&s.svc.http, "http://lists.example.org/index.json").unwrap_err(), ListError::NotHttps);
    assert_eq!(s.lists.add(&s.svc, "http://other.example.net/heronite.json"), Err(ListError::NotHttps));
    assert_eq!(s.lists.add(&s.svc, LIST), Err(ListError::AlreadyAdded));
    assert_eq!(lists::fetch_list(&s.svc.http, "https://lists.example.org/nothing.json").unwrap_err(), ListError::Status(404));

    // Larger than a list may be.
    let mut big = list_json();
    big["padding"] = json!("x".repeat(index::MAX_INDEX_BYTES as usize));
    *s.list.lock() = big;
    assert_eq!(lists::fetch_list(&s.svc.http, LIST).unwrap_err(), ListError::TooLarge);

    // A later format, or a wrong entry: refused, the copy kept stays.
    *s.list.lock() = json!({"index": 2, "extensions": []});
    assert_eq!(lists::fetch_list(&s.svc.http, LIST).unwrap_err(), ListError::Invalid(IndexError::NewerFormat(2)));
    // A list of another type of extensions is refused, with its own error.
    let mut other = list_json();
    other["type"] = json!("themes");
    *s.list.lock() = other;
    assert_eq!(lists::fetch_list(&s.svc.http, LIST).unwrap_err(), ListError::Invalid(IndexError::WrongType("themes".into())));
    let mut wrong = list_json();
    entry_mut(&mut wrong, "wikipedia")["files"]["source.js"]["url"] = json!("http://lists.example.org/ext/wikipedia/source.js");
    *s.list.lock() = wrong;
    assert!(matches!(lists::fetch_list(&s.svc.http, LIST).unwrap_err(), ListError::Invalid(IndexError::Invalid(_))));
}

#[test]
fn an_entry_installs_as_a_file_would_and_answers_through_the_gather() {
    let s = setup();
    let entry = s.entry("wikipedia");
    let prepared = lists::prepare(&s.svc, LIST, &entry).unwrap();
    // The confirmation shows what a file's would.
    assert_eq!((prepared.info.id.as_str(), prepared.info.version.as_str()), ("wikipedia", "1.0.0"));
    assert_eq!(prepared.info.hosts, ["*.wikipedia.org"]);
    assert!(s.svc.sources().get("wikipedia").is_none(), "nothing is installed before the user confirms");

    let out = lists::install_prepared(&s.svc, &prepared, false).unwrap();
    let InstallOutcome::Installed(ext) = out else { panic!("{out:?}") };
    assert_eq!(ext.list.as_deref(), Some(LIST));
    assert!(ext.from.ends_with("/wikipedia/source.toml"));
    // The app's own list here is not at the Heronite list's address.
    assert_eq!(s.lists.origin(&s.svc, &ext), Origin::List(LIST.into()));
    drop(prepared);
    let downloads = s.paths.extension_lists_cache_dir().join("downloads");
    assert_eq!(std::fs::read_dir(&downloads).map(|d| d.count()).unwrap_or(0), 0, "the downloaded files go once used");

    // Registered, and it answers through the gather.
    assert!(s.svc.sources().get("wikipedia").is_some());
    let saved = s
        .svc
        .apply_game_metadata(&MetadataApplyArgs {
            app_name: "portal2".into(),
            runner: "sideload".into(),
            title: "Portal 2".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(saved.description.as_deref().unwrap().starts_with("Portal 2 is a 2011 first person puzzle-platform game"));
    assert_eq!(saved.field_sources["description"], "wikipedia");
}

#[test]
fn a_file_that_is_not_the_one_listed_is_refused() {
    let s = setup();
    let mut list = list_json();
    entry_mut(&mut list, "wikipedia")["files"]["source.js"]["sha256"] = json!("1".repeat(64));
    *s.list.lock() = list;
    let err = lists::prepare(&s.svc, LIST, &s.entry("wikipedia")).unwrap_err();
    assert_eq!(err, ExtensionError::Checksum("source.js".into()));

    // Larger than the list says.
    *s.list.lock() = list_json();
    let entry = s.entry("wikipedia");
    let mut longer = std::fs::read(example("wikipedia").join("source.js")).unwrap();
    longer.extend(b"\n// more\n");
    s.files.lock().push(("wikipedia/source.js".into(), longer));
    assert_eq!(lists::prepare(&s.svc, LIST, &entry).unwrap_err(), ExtensionError::Checksum("source.js".into()));

    // Gone from the server.
    s.files.lock().clear();
    let mut list = list_json();
    entry_mut(&mut list, "wikipedia")["files"]["source.js"]["url"] = json!(format!("{FILES}nowhere/source.js"));
    *s.list.lock() = list;
    assert!(matches!(lists::prepare(&s.svc, LIST, &s.entry("wikipedia")), Err(ExtensionError::Download(_))));
    assert!(s.svc.installed_extensions().is_empty());
}

#[test]
fn a_manifest_that_is_not_what_its_entry_says_is_refused() {
    let s = setup();
    let cases: Vec<(&str, Value)> = vec![
        ("hosts", json!(["en.wikipedia.org"])),
        ("version", json!("1.0.1")),
        ("kind", json!("extras")),
        ("fields", json!(["description"])),
    ];
    for (key, value) in cases {
        let mut list = list_json();
        entry_mut(&mut list, "wikipedia")[key] = value;
        *s.list.lock() = list;
        let err = lists::prepare(&s.svc, LIST, &s.entry("wikipedia")).unwrap_err();
        assert!(matches!(err, ExtensionError::NotAsListed(_)), "{key}: {err:?}");
    }
    // Another id: the files are another extension's.
    let mut list = list_json();
    let vndb = entry_mut(&mut list, "vndb").clone();
    let w = entry_mut(&mut list, "wikipedia");
    w["files"] = vndb["files"].clone();
    *s.list.lock() = list;
    let err = lists::prepare(&s.svc, LIST, &s.entry("wikipedia")).unwrap_err();
    assert!(matches!(err, ExtensionError::NotAsListed(ref m) if m.contains("vndb")), "{err:?}");
    assert!(s.svc.installed_extensions().is_empty());
}

#[test]
fn updates_are_flagged_from_the_list_an_extension_came_from_only() {
    let s = setup();
    let prepared = lists::prepare(&s.svc, LIST, &s.entry("wikipedia")).unwrap();
    lists::install_prepared(&s.svc, &prepared, false).unwrap();
    // The same extension from a file says nothing about any list.
    s.svc.install_extension(&example("vndb"), false).unwrap();
    let installed = s.svc.installed_extensions();
    assert!(lists::updates(&s.lists.refresh(&s.svc), &installed).is_empty(), "the same versions");

    let mut list = list_json();
    for id in ["wikipedia", "vndb"] {
        entry_mut(&mut list, id)["version"] = json!("1.1.0");
    }
    *s.list.lock() = list;
    let available = lists::updates(&s.lists.refresh(&s.svc), &installed);
    assert_eq!(available.len(), 1, "{available:?}");
    assert_eq!((available[0].id.as_str(), available[0].installed.as_str()), ("wikipedia", "1.0.0"));
    assert_eq!(available[0].offered.to_string(), "1.1.0");
    assert_eq!(s.svc.installed_extensions()[1].version, "1.0.0", "nothing is updated by itself");
}

#[test]
fn a_list_removed_leaves_what_came_from_it() {
    let s = setup();
    assert_eq!(s.lists.add(&s.svc, &format!("  {OTHER} ")).unwrap(), OTHER);
    assert_eq!(s.lists.add(&s.svc, OTHER), Err(ListError::AlreadyAdded));
    assert_eq!(s.lists.in_use(&s.svc), [(LIST.to_owned(), true), (OTHER.to_owned(), false)]);
    let copies = s.lists.refresh(&s.svc);
    assert!(copies.iter().all(|c| c.error.is_none() && c.index.is_some()));
    assert!(!copies[1].builtin);

    let entry = copies[1].index.as_ref().unwrap().get("vndb").cloned().unwrap();
    let prepared = lists::prepare(&s.svc, OTHER, &entry).unwrap();
    let InstallOutcome::Installed(ext) = lists::install_prepared(&s.svc, &prepared, false).unwrap() else { panic!() };
    assert_eq!(s.lists.origin(&s.svc, &ext), Origin::List(OTHER.into()));
    assert_eq!(lists::host_of(OTHER), "other.example.net");

    s.lists.remove(&s.svc, OTHER).unwrap();
    s.lists.remove(&s.svc, LIST).unwrap();
    assert_eq!(s.lists.in_use(&s.svc), [(LIST.to_owned(), true)], "the app's own list stays");
    assert_eq!(s.lists.cached(&s.svc).len(), 1);
    let ext = s.svc.installed_extensions().into_iter().find(|e| e.id == "vndb").unwrap();
    assert_eq!(s.lists.origin(&s.svc, &ext), Origin::GoneList(OTHER.into()));
    assert!(s.svc.sources().get("vndb").is_some(), "still installed and asked");
}

#[test]
fn only_the_heronite_lists_address_is_the_heronite_list() {
    // `HERONITE_EXTENSIONS_INDEX=https://…` makes another address the app's own.
    let own = Lists::new(LIST);
    let in_use = [(LIST.to_owned(), true), (OTHER.to_owned(), false)];
    assert_eq!(lists::origin_among(&own, &in_use, Some(LIST)), Origin::List(LIST.into()));
    assert_eq!(lists::origin_among(&own, &[], Some(LIST)), Origin::List(LIST.into()), "the app's own, listed or not");
    assert_eq!(lists::origin_among(&own, &in_use, Some(lists::HERONITE_LIST)), Origin::Heronite);
    assert_eq!(
        lists::origin_among(&Lists::default(), &[(lists::HERONITE_LIST.into(), true)], Some(lists::HERONITE_LIST)),
        Origin::Heronite
    );
    assert_eq!(lists::origin_among(&own, &in_use, Some(OTHER)), Origin::List(OTHER.into()));
    assert_eq!(lists::origin_among(&own, &in_use, None), Origin::File);
}

#[test]
fn an_entrys_files_come_from_public_addresses_only() {
    // A list may name any https address for an entry's files: one of this
    // computer or the local network is never asked.
    let s = setup();
    let mut list = list_json();
    entry_mut(&mut list, "wikipedia")["files"]["source.toml"]["url"] = json!("https://localhost/ext/wikipedia/source.toml");
    *s.list.lock() = list;
    let err = lists::prepare(&s.svc, LIST, &s.entry("wikipedia")).unwrap_err();
    assert!(matches!(err, ExtensionError::Download(ref m) if m.contains("local network")), "{err:?}");
    assert!(s.svc.installed_extensions().is_empty());
}
