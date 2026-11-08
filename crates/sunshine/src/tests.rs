//! A throwaway local server that behaves like Sunshine's app API: basic
//! auth, the list sorted by name after a save, deletions by index, the
//! `index` key taken off what is stored and unknown keys kept.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::{json, Value};

use crate::api::{Client, Error};
use crate::entry::{Entry, MARKER};
use crate::sync::{remove_all, sync, Report};

#[derive(Default)]
struct State {
    apps: Vec<Value>,
    /// Request lines (`GET /api/apps`), in order.
    log: Vec<String>,
    /// Answers every request with this status instead (redirects send a `Location`).
    force: Option<u16>,
}

struct Mock {
    address: String,
    state: Arc<Mutex<State>>,
}

const USER: &str = "sunny";
const PASS: &str = "p@ss:word";

impl Mock {
    fn start(apps: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State { apps, ..Default::default() }));
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &shared);
            }
        });
        Self { address, state }
    }

    fn client(&self) -> Client {
        Client::new(&self.address, USER, PASS).unwrap()
    }

    fn apps(&self) -> Vec<Value> {
        self.state.lock().unwrap().apps.clone()
    }

    fn names(&self) -> Vec<String> {
        self.apps().iter().map(|a| a["name"].as_str().unwrap_or("").to_string()).collect()
    }

    fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }

    fn force(&self, status: u16) {
        self.state.lock().unwrap().force = Some(status);
    }
}

fn answer(mut stream: std::net::TcpStream, state: &Mutex<State>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let header = |name: &str| {
        head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    let length = header("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let mut body = vec![0u8; length];
    if stream.read_exact(&mut body).is_err() {
        return;
    }
    let line = head.lines().next().unwrap_or("").to_string();
    let mut parts = line.split(' ');
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut st = state.lock().unwrap();
    st.log.push(format!("{method} {path}"));
    let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASS}")));
    let (status, reply, extra) = if let Some(code) = st.force {
        (code, json!({ "status": false, "error": "forced" }), if code == 307 { "Location: /welcome\r\n" } else { "" })
    } else if header("authorization").as_deref() != Some(expected.as_str()) {
        (401, json!({ "status_code": 401, "status": false, "error": "Unauthorized" }), "")
    } else {
        let (s, r) = route(&mut st, &method, &path, &body, header("content-type"));
        (s, r, "")
    };
    let text = reply.to_string();
    let out = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.write_all(out.as_bytes());
}

fn bad(msg: &str) -> (u16, Value) {
    (400, json!({ "status_code": 400, "status": false, "error": msg }))
}

fn route(st: &mut State, method: &str, path: &str, body: &[u8], content_type: Option<String>) -> (u16, Value) {
    match (method, path) {
        ("GET", "/api/apps") => (200, json!({ "env": {}, "apps": st.apps })),
        ("POST", "/api/apps") => {
            if content_type.as_deref() != Some("application/json") {
                return bad("Content type mismatch");
            }
            let Ok(mut app) = serde_json::from_slice::<Value>(body) else { return bad("bad json") };
            let index = app["index"].as_i64().unwrap_or(-2);
            app.as_object_mut().unwrap().remove("index");
            if index == -1 {
                st.apps.push(app);
            } else if index >= 0 && (index as usize) < st.apps.len() {
                st.apps[index as usize] = app;
            } else {
                return bad("index");
            }
            st.apps.sort_by(|a, b| a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or("")));
            (200, json!({ "status": true }))
        }
        ("DELETE", p) if p.starts_with("/api/apps/") => {
            let Ok(i) = p["/api/apps/".len()..].parse::<usize>() else { return bad("index") };
            if i >= st.apps.len() {
                return bad("out of range");
            }
            st.apps.remove(i);
            (200, json!({ "status": true, "result": format!("application {i} deleted") }))
        }
        _ => (404, json!({ "status_code": 404, "error": "Not Found" })),
    }
}

fn game(id: &str, name: &str) -> Entry {
    Entry {
        id: id.into(),
        name: name.into(),
        cmd: format!("heronite --launch {id}"),
        image_path: Some(format!("/covers/{id}.png")),
        undo: Some(format!("heronite --stop {id}")),
    }
}

fn users_apps() -> Vec<Value> {
    vec![
        json!({ "name": "Desktop", "image-path": "desktop.png" }),
        json!({ "name": "Steam Big Picture", "detached": ["steam://open/bigpicture"], "image-path": "steam.png" }),
    ]
}

#[test]
fn first_sync_adds_entries_and_leaves_the_users_apps_alone() {
    let mock = Mock::start(users_apps());
    let report = sync(&mock.client(), &[game("gog_1", "Zeta"), game("legendary_2", "Alpha")]).unwrap();
    assert_eq!(report, Report { added: 2, updated: 0, removed: 0 });
    assert_eq!(mock.names(), ["Alpha", "Desktop", "Steam Big Picture", "Zeta"]);
    let apps = mock.apps();
    assert_eq!(apps[1], users_apps()[0]);
    assert_eq!(apps[2], users_apps()[1]);
    assert_eq!(apps[0][MARKER], "legendary_2");
    assert_eq!(apps[0]["cmd"], "heronite --launch legendary_2");
    assert_eq!(apps[0]["prep-cmd"][0]["undo"], "heronite --stop legendary_2");
    assert!(apps[0].get("index").is_none());
}

#[test]
fn a_second_sync_with_nothing_new_only_reads() {
    let mock = Mock::start(users_apps());
    let wanted = [game("gog_1", "Zeta"), game("legendary_2", "Alpha")];
    sync(&mock.client(), &wanted).unwrap();
    let before = mock.log().len();
    let report = sync(&mock.client(), &wanted).unwrap();
    assert!(!report.changed());
    assert_eq!(mock.log()[before..], ["GET /api/apps"]);
}

#[test]
fn renames_update_in_place_and_keep_keys_the_user_added() {
    let mock = Mock::start(users_apps());
    let client = mock.client();
    sync(&client, &[game("gog_1", "Zeta"), game("legendary_2", "Alpha"), game("nile_3", "Mid")]).unwrap();
    // The user gave one entry a log file in Sunshine's web UI.
    {
        let mut st = mock.state.lock().unwrap();
        let i = st.apps.iter().position(|a| a[MARKER] == "gog_1").unwrap();
        st.apps[i]["output"] = json!("zeta.log");
    }
    // Both renames move their entries across the sorted list.
    let report = sync(&client, &[game("gog_1", "Aardvark"), game("legendary_2", "Zulu"), game("nile_3", "Mid")]).unwrap();
    assert_eq!(report, Report { added: 0, updated: 2, removed: 0 });
    assert_eq!(mock.names(), ["Aardvark", "Desktop", "Mid", "Steam Big Picture", "Zulu"]);
    let apps = mock.apps();
    assert_eq!(apps[0][MARKER], "gog_1");
    assert_eq!(apps[0]["output"], "zeta.log");
    assert_eq!(apps[4][MARKER], "legendary_2");
}

#[test]
fn games_that_leave_are_removed_but_the_users_apps_stay() {
    let mut apps = users_apps();
    // An app of the user's that looks like one of Heronite's but has no marker.
    apps.push(json!({ "name": "Hand-made", "cmd": "heronite --launch gog_9" }));
    let mock = Mock::start(apps);
    let client = mock.client();
    sync(&client, &[game("a_1", "B"), game("a_2", "Ha"), game("a_3", "Zz")]).unwrap();
    let report = sync(&client, &[game("a_2", "Ha")]).unwrap();
    assert_eq!(report, Report { added: 0, updated: 0, removed: 2 });
    assert_eq!(mock.names(), ["Desktop", "Ha", "Hand-made", "Steam Big Picture"]);
    assert_eq!(remove_all(&client).unwrap(), 1);
    assert_eq!(mock.names(), ["Desktop", "Hand-made", "Steam Big Picture"]);
}

#[test]
fn duplicate_entries_of_one_game_are_cleaned_up() {
    let [desktop, steam] = <[Value; 2]>::try_from(users_apps()).unwrap();
    let apps = vec![desktop, game("gog_1", "Quail").to_app(), game("gog_1", "Quail").to_app(), steam];
    let mock = Mock::start(apps);
    let report = sync(&mock.client(), &[game("gog_1", "Quail"), game("gog_1", "Quail")]).unwrap();
    assert_eq!(report, Report { added: 0, updated: 0, removed: 1 });
    assert_eq!(mock.names(), ["Desktop", "Quail", "Steam Big Picture"]);
}

#[test]
fn errors_say_what_is_wrong() {
    let mock = Mock::start(users_apps());
    let wrong = Client::new(&mock.address, USER, "nope").unwrap();
    assert_eq!(sync(&wrong, &[game("gog_1", "Q")]), Err(Error::Unauthorized));
    // Nothing was changed by the refused attempt.
    assert_eq!(mock.apps(), users_apps());

    mock.force(307);
    assert_eq!(mock.client().apps(), Err(Error::NotSetUp));
    mock.force(403);
    assert_eq!(mock.client().apps(), Err(Error::Forbidden));
    mock.force(500);
    assert!(matches!(mock.client().apps(), Err(Error::Api(e)) if e.contains("500") && e.contains("forced")));

    // Nothing listens on a port that was just freed.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let gone = Client::new(&format!("http://127.0.0.1:{port}"), USER, PASS).unwrap();
    assert!(matches!(gone.apps(), Err(Error::Unreachable(_))));
    assert_eq!(Client::new("ftp://x", USER, PASS).err(), Some(Error::BadAddress));
}

#[test]
fn an_empty_or_missing_list_reads_as_no_apps() {
    let mock = Mock::start(Vec::new());
    assert!(mock.client().apps().unwrap().is_empty());
    let report = sync(&mock.client(), &[game("gog_1", "Quail")]).unwrap();
    assert_eq!(report.added, 1);
}
