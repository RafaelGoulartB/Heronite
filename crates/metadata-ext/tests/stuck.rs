//! A script that loops where the engine cannot interrupt it never blocks
//! its caller past the call's time limit and the grace: at its module's top
//! level (`inspect`, load), in `find` and in `fetch`. Its own test binary:
//! the threads it leaves running count against a cap of the whole process.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_data::paths::DataPaths;
use heronite_metadata::sources::SourceHealth;
use heronite_metadata::{Lookup, MetadataSource, SourceContext, SourceGame};
use heronite_metadata_ext::engine::{self, session, CallError, HostEnv, Limit, Script, GRACE, MAX_STUCK};
use heronite_metadata_ext::manifest::Manifest;
use heronite_metadata_ext::net::RateLimiter;
use heronite_metadata_ext::source::ScriptSource;
use heronite_metadata_ext::{ExtensionError, ExtensionsExt, Limits};
use serde_json::json;
use support::{manifest, Http};

/// The loops the review found running past 30 s inside QuickJS's own
/// built-in functions.
const PAYLOADS: [&str; 4] = [
    r#"Array.prototype.join.call({ length: 2 ** 53 - 1 }, "")"#,
    "Array.prototype.reverse.call({ length: 2 ** 53 - 1 })",
    "Array.prototype.copyWithin.call(Object.freeze({ length: 2 ** 53 - 1 }), 0, 1)",
    "Array.prototype.flat.call({ length: 2 ** 53 - 1 })",
];

/// The same calls on an object whose `length` reads small the first time
/// (the guard's check) and huge after: they get past the guard and loop.
const SLIPPERY: &str = "let reads = 0; const big = { get length() { return reads++ === 0 ? 0 : 2 ** 53 - 1; } };";

fn limits() -> Limits {
    Limits { time: Duration::from_millis(800), ..Limits::default() }
}

/// What a caller may wait at most: the time limit and the grace, with room
/// for a slow machine.
fn bound() -> Duration {
    limits().time + GRACE + Duration::from_millis(700)
}

fn env(id: &str) -> HostEnv {
    HostEnv {
        id: id.into(),
        http: Http::default(),
        hosts: Arc::new(Vec::new()),
        rate: Arc::new(RateLimiter::new(Duration::ZERO)),
        limits: limits(),
        settings: Arc::new(serde_json::Map::new()),
        language: "en".into(),
    }
}

#[derive(Clone, Copy, Debug)]
enum Place {
    Top,
    Find,
    Fetch,
}

fn script(place: Place, code: &str) -> String {
    let (top, find, fetch) = match place {
        Place::Top => (code, "", ""),
        Place::Find => ("", code, ""),
        Place::Fetch => ("", "", code),
    };
    format!("{top}\nexport function find(game, ctx) {{ {find}; return []; }}\nexport function fetch(id, ctx) {{ {fetch}; return null; }}\n")
}

/// Loads the script (its top level runs) and calls `find` or `fetch`: the
/// error, and how long the caller waited.
fn run(id: &str, place: Place, code: &str) -> (String, Duration) {
    let env = env(id);
    let started = Instant::now();
    let error = match Script::compile(&script(place, code), &env) {
        Err(ExtensionError::Script(m)) => m,
        Err(e) => panic!("{e:?}"),
        Ok(s) => {
            let name = if matches!(place, Place::Fetch) { "fetch" } else { "find" };
            let arg = if matches!(place, Place::Fetch) { json!("1") } else { json!({ "title": "Doom" }) };
            match session(&s, &env, move |c| c.call(name, Some(arg))).and_then(|r| r) {
                Err(e) => e.to_string(),
                Ok(v) => panic!("{place:?} {code}: it returned {v}"),
            }
        }
    };
    (error, started.elapsed())
}

fn source(id: &str, place: Place, code: &str) -> ScriptSource {
    let m = Manifest::parse(&manifest(id, "1.0.0", "details", &["description"], "")).unwrap();
    ScriptSource::new(m, &script(place, code), limits(), None).unwrap()
}

/// One test: the threads it leaves running count against the cap of the
/// whole process, so the cases run in this order.
#[test]
fn a_script_never_blocks_its_caller_past_the_time_limit() {
    // The known loops are refused before they run, wherever they are.
    for code in PAYLOADS {
        for place in [Place::Top, Place::Find, Place::Fetch] {
            let (error, waited) = run("guarded", place, code);
            assert!(waited < bound(), "{place:?} {code}: {waited:?}");
            assert!(error.contains("is too large"), "{place:?} {code}: {error}");
        }
    }
    assert_eq!(engine::stuck("guarded"), 0, "nothing was left running");

    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("data"));
    let http = Http::default();
    let ctx = SourceContext::new(&paths, &http);
    let game = SourceGame { title: "Doom".into(), ..Default::default() };
    let slippery = |code: &str| format!("{SLIPPERY} {}", code.replace("{ length: 2 ** 53 - 1 }", "big"));
    let hangs = source("hangs", Place::Find, &slippery(PAYLOADS[1]));
    let innocent = source("innocent", Place::Find, "");

    // Past the guard, the caller still has its answer in time; the thread
    // is left behind. At the top level, inspecting a package (the real
    // limits: 10 s and the grace):
    let svc = heronite_metadata::MetadataService::new(paths.clone());
    let code = slippery(PAYLOADS[0]);
    let top = support::folder(dir.path(), "top", &manifest("top", "1.0.0", "details", &["description"], ""), &script(Place::Top, &code));
    let started = Instant::now();
    let e = svc.inspect_extension(&top).unwrap_err();
    assert!(started.elapsed() < Limits::default().time + GRACE + Duration::from_millis(700), "inspect: {:?}", started.elapsed());
    assert!(e.to_string().contains("could not be stopped"), "{e}");
    assert_eq!(engine::stuck("top"), 1);
    // Installed by hand, then loaded at startup: not started while its
    // thread runs, and never loaded again until an update replaces it.
    let installed = paths.metadata_extensions_dir().join("top");
    std::fs::create_dir_all(&installed).unwrap();
    for f in ["source.toml", "source.js"] {
        std::fs::copy(top.join(f), installed.join(f)).unwrap();
    }
    std::fs::write(installed.join("install.json"), r#"{"installedAt": "", "from": "", "sha256": "x", "version": "1.0.0"}"#).unwrap();
    for _ in 0..2 {
        let started = Instant::now();
        let loaded = heronite_metadata::MetadataService::new(paths.clone()).load_extensions();
        assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
        let heronite_metadata_ext::ExtensionState::LoadFailed(why) = &loaded[0].state else { panic!("{loaded:?}") };
        assert!(heronite_metadata_ext::source::is_stuck(why), "{why}");
    }
    let record = heronite_metadata_ext::store::read_record(&installed).unwrap();
    assert!(record.failing.is_some_and(|f| f.starts_with("loading: ")));
    assert_eq!(engine::stuck("top"), 1);
    // In `fetch`:
    let (error, waited) = run("fetch", Place::Fetch, &slippery(PAYLOADS[3]));
    assert!(waited < bound(), "fetch: waited {waited:?}");
    assert!(error.contains("could not be stopped"), "{error}");
    // In `find`, through the source: stopped at once, saying why.
    let started = Instant::now();
    assert!(matches!(hangs.lookup(&ctx, &game), Lookup::NotFound));
    assert!(started.elapsed() < bound(), "find: waited {:?}", started.elapsed());
    let SourceHealth::Failing(why) = hangs.health() else { panic!("{:?}", hangs.health()) };
    assert!(why.contains("could not be stopped") && why.contains("processor"), "{why}");
    assert!(heronite_metadata_ext::source::is_stuck(&why) && !heronite_metadata_ext::source::is_stuck("Error: Stuck on a page"));

    // Switched on again, it starts nothing while its thread runs.
    hangs.reset_health();
    let started = Instant::now();
    assert!(matches!(hangs.lookup(&ctx, &game), Lookup::NotFound));
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(matches!(hangs.health(), SourceHealth::Failing(why) if why.contains("still running")));
    assert_eq!(engine::stuck("hangs"), 1);

    // At the cap in all, no extension starts, and that is not its failure.
    assert_eq!(MAX_STUCK, 3, "three are left running above");
    assert!(matches!(innocent.lookup(&ctx, &game), Lookup::Skipped));
    assert_eq!(innocent.health(), SourceHealth::Ok);
    let e = Script::compile(&script(Place::Find, ""), &env("new")).err().expect("not started");
    assert!(e.to_string().contains("not started"), "{e}");
    assert!(!CallError::Busy(String::new()).is_failure());
    let _ = Limit::Stuck;
}
