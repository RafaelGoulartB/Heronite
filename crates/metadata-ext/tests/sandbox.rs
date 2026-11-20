//! The sandbox: what a script cannot do, the limits of a call, and the
//! host API (`ctx`), against the fixture server.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_metadata::http::APP_UA;
use heronite_metadata_ext::engine::{session, CallError, HostEnv, Limit, Script};
use heronite_metadata_ext::net::{HostRule, RateLimiter};
use heronite_metadata_ext::{ExtensionError, Limits};
use serde_json::{json, Value};
use support::{endpoints, Http, Resp, Server};

const HOSTS: [&str; 2] = ["api.example.org", "other.example.org"];

fn quick() -> Limits {
    Limits { time: Duration::from_millis(1500), memory: 16 * 1024 * 1024, stack: 256 * 1024, requests: 5, ..Limits::default() }
}

fn env(http: Http, limits: Limits) -> HostEnv {
    let mut settings = serde_json::Map::new();
    settings.insert("token".into(), json!("mine"));
    settings.insert("hd".into(), json!(true));
    HostEnv {
        id: "test".into(),
        http,
        hosts: Arc::new(HOSTS.iter().map(|h| HostRule::parse(h).unwrap()).collect()),
        rate: Arc::new(RateLimiter::new(Duration::ZERO)),
        limits,
        settings: Arc::new(settings),
        language: "pt-BR".into(),
    }
}

/// A module whose `find` has `body`, with a `fetch` that gives nothing.
fn module(body: &str) -> String {
    format!("export async function find(game, ctx) {{\n{body}\n}}\nexport function fetch() {{ return null; }}\n")
}

/// Runs `find(arg)` of a module with `body`.
fn run_with(env: &HostEnv, body: &str, arg: Value) -> Result<Value, CallError> {
    let script = Script::compile(&module(body), env).unwrap_or_else(|e| panic!("{e}"));
    session(&script, env, |calls| calls.call("find", Some(arg))).and_then(|r| r)
}

fn run(body: &str) -> Result<Value, CallError> {
    run_with(&env(Http::default(), quick()), body, json!({}))
}

fn server() -> Server {
    Server::start(|req| {
        let path = req.path();
        match path {
            "/api.example.org/json" => Resp::json(json!({"ok": true, "q": req.query("q")})),
            "/api.example.org/echo" => Resp::json(json!({
                "ua": req.header("user-agent"),
                "auth": req.header("authorization"),
                "ct": req.header("content-type"),
                "method": req.method,
                "body": req.body,
            })),
            "/api.example.org/big" => Resp::raw_json(format!("\"{}\"", "x".repeat(3 * 1024 * 1024))),
            "/api.example.org/to-other" => Resp::status(302).with_header("location", "https://other.example.org/json"),
            "/api.example.org/to-evil" => Resp::status(302).with_header("location", "https://evil.example.com/steal"),
            "/api.example.org/to-ip" => Resp::status(301).with_header("location", "http://127.0.0.1/"),
            "/api.example.org/loop" => Resp::status(302).with_header("location", "/loop"),
            "/api.example.org/page" => Resp::html(
                "<html><body><h1 class='t'> Portal   2 </h1><ul><li><a href='/a'>A</a></li><li><a href='/b'>B</a></li></ul>\
                 <div id=info><span data-year='2011'>Released</span></div></body></html>",
            ),
            "/other.example.org/json" => Resp::json(json!({"other": true})),
            _ => Resp::status(404),
        }
    })
}

fn served() -> (Server, HostEnv) {
    let server = server();
    let env = env(Http::new(endpoints(&server, &HOSTS)), quick());
    (server, env)
}

#[test]
fn nothing_of_the_host_is_in_the_global_scope() {
    let names = run(r#"
        const probe = ["fetch", "XMLHttpRequest", "WebSocket", "require", "process", "std", "os", "Deno", "Bun",
            "navigator", "performance", "print", "console", "setTimeout", "setInterval", "ctx", "http",
            "__host", "WeakRef", "FinalizationRegistry", "importScripts", "module", "exports"];
        return probe.filter((n) => typeof globalThis[n] !== "undefined");
    "#)
    .unwrap();
    assert_eq!(names, json!([]), "host objects in the global scope");
    // The engine's own globals only: nothing named after the app.
    let all = run("return Object.getOwnPropertyNames(globalThis);").unwrap();
    let all: Vec<&str> = all.as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(all.contains(&"JSON") && all.contains(&"Promise"));
    assert!(!all.iter().any(|n| n.to_lowercase().contains("heronite") || n.to_lowercase().contains("rust")), "{all:?}");
}

#[test]
fn no_eval_and_no_function_constructor() {
    for code in [
        "eval('1 + 1')",
        "new Function('return 1')()",
        "(function () {}).constructor('return 1')()",
        "(async function () {}).constructor('return 1')()",
        "(function* () {}).constructor('yield 1')().next()",
        "Reflect.construct(Function, ['return 1'])()",
    ] {
        let r = run(&format!("try {{ {code}; return 'ran'; }} catch (e) {{ return String(e); }}")).unwrap();
        assert!(r.as_str().unwrap().contains("not supported"), "{code}: {r}");
    }
}

#[test]
fn no_modules_can_be_imported() {
    let env = env(Http::default(), quick());
    let static_import = "import * as os from 'os';\nexport function find() { return []; }\nexport function fetch() { return null; }";
    let e = Script::compile(static_import, &env).err().expect("an import is refused");
    assert!(matches!(e, ExtensionError::Script(_)), "{e:?}");
    let e = Script::compile("import x from './source.js';\nexport function find() {}\nexport function fetch() {}", &env).err();
    assert!(e.is_some(), "not even its own file");
    let r = run("try { await import('std'); return 'imported'; } catch (e) { return String(e); }").unwrap();
    assert_ne!(r, json!("imported"));
    assert!(matches!(run("return await import('os');"), Err(CallError::Script(_))));
    // A script without the required exports does not load.
    let e = Script::compile("export function find() { return []; }", &env).err().unwrap();
    assert!(e.to_string().contains("fetch"), "{e}");
    assert!(Script::compile("export function find( {", &env).is_err(), "a syntax error");
}

#[test]
fn what_a_call_changes_dies_with_its_engine() {
    let env = env(Http::default(), quick());
    let script = Script::compile(
        &module(
            r#"
        if (game.first) {
            Object.prototype.polluted = "yes";
            Array.prototype.map = () => "hijacked";
            JSON.parse = () => ({ forged: true });
            globalThis.leftover = 1;
            ctx.http.get = () => "replaced";
            return "changed";
        }
        return [({}).polluted, [1].map((x) => x), typeof globalThis.leftover, typeof ctx.http.get];
    "#,
        ),
        &env,
    )
    .unwrap();
    let first = session(&script, &env, |c| c.call("find", Some(json!({"first": true})))).unwrap().unwrap();
    assert_eq!(first, json!("changed"));
    let second = session(&script, &env, |c| c.call("find", Some(json!({})))).unwrap().unwrap();
    assert_eq!(second, json!([null, [1], "undefined", "function"]));
}

#[test]
fn a_poisoned_prototype_does_not_reach_the_host() {
    let (_server, env) = served();
    // Getters, toJSON and a replaced JSON.parse change nothing the host reads.
    let r = run_with(
        &env,
        r#"
        JSON.parse = () => ({ forged: true });
        Object.defineProperty(Object.prototype, "title", { get() { return "from the prototype"; } });
        const r = await ctx.http.get("https://api.example.org/json", { query: { q: "a" } });
        const data = r.json();
        return { data, own: { toJSON() { return "replaced"; }, kept: 1 }, fn: () => 1, sym: Symbol("x") };
    "#,
        json!({}),
    )
    .unwrap();
    assert_eq!(r["data"], json!({"ok": true, "q": "a"}));
    assert_eq!(r["own"]["kept"], 1);
    assert!(r.get("fn").is_none() && r.get("sym").is_none(), "functions and symbols are not data");
    assert!(r.get("title").is_none(), "inherited properties are not read");
}

#[test]
fn an_endless_loop_is_stopped() {
    let started = Instant::now();
    let e = run("while (true) {}").unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Time, _)), "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    // Not even a catch keeps it going.
    let e = run("try { while (true) {} } catch (e) {} return 'escaped';").unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Time, _)), "{e:?}");
    // Nor a loop of jobs.
    let e = run("const spin = () => Promise.resolve().then(spin); await spin();").unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Time | Limit::Memory | Limit::NeverSettled, _)), "{e:?}");
    // Nor a regular expression that backtracks for ever.
    let e = run("return /^(a+)+$/.test('a'.repeat(40) + 'b');").unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Time, _)), "{e:?}");
}

#[test]
fn a_huge_allocation_is_stopped() {
    for code in [
        "const a = []; while (true) a.push(new Array(1000).fill('xxxxxxxx'));",
        "let s = 'x'; while (true) s = s + s;",
        "return 'x'.repeat(1 << 29);",
        "return new ArrayBuffer(1 << 30);",
    ] {
        let e = run(code).unwrap_err();
        assert!(matches!(e, CallError::Limit(Limit::Memory | Limit::Time, _) | CallError::Script(_)), "{code}: {e:?}");
        // The engine's own caps (a string's length) stop some first.
        assert!(!matches!(e, CallError::Script(ref m) if !m.contains("too long") && !m.contains("RangeError")), "{code}: {e:?}");
    }
}

#[test]
fn deep_recursion_is_stopped() {
    let e = run("function down(n) { return down(n + 1) + 1; } return down(0);").unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Stack, _)), "{e:?}");
    // The engine is still usable for the next call.
    assert_eq!(run("return 1 + 1;").unwrap(), json!(2));
}

#[test]
fn a_promise_that_never_settles_ends_the_call() {
    let started = Instant::now();
    let e = run("return await new Promise(() => {});").unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::NeverSettled, _)), "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(1), "known at once: there are no timers");
}

#[test]
fn a_thousand_requests_stop_at_the_limit() {
    let (server, env) = served();
    let e = run_with(&env, "for (let i = 0; i < 1000; i++) await ctx.http.get('https://api.example.org/json'); return 'done';", json!({}))
        .unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Requests, _)), "{e:?}");
    assert_eq!(server.count(|r| r.path() == "/api.example.org/json"), 5);
    // Catching the error does not help.
    let e = run_with(
        &env,
        "for (let i = 0; i < 1000; i++) { try { await ctx.http.get('https://api.example.org/json'); } catch (e) {} } return 'done';",
        json!({}),
    )
    .unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Requests, _)), "{e:?}");
}

#[test]
fn only_the_manifests_hosts_over_https() {
    let (server, env) = served();
    for (address, why) in [
        ("https://evil.example.com/", "another host"),
        ("https://api.example.org.evil.com/", "a look-alike"),
        ("http://api.example.org/json", "http when only https is declared"),
        ("https://user:secret@api.example.org/json", "credentials"),
        ("https://127.0.0.1/json", "an IP address"),
        ("https://[::1]/json", "an IPv6 address"),
        ("https://2130706433/json", "a number for an IP address"),
        ("https://api.example.org:8443/json", "another port"),
        ("file:///etc/passwd", "a file"),
        ("data:text/plain,hello", "a data address"),
    ] {
        let body = format!("try {{ await ctx.http.get({address:?}); }} catch (e) {{}} return 'went on';");
        let e = run_with(&env, &body, json!({})).unwrap_err();
        assert!(matches!(e, CallError::Denied(_)), "{why}: {e:?}");
    }
    assert_eq!(server.count(|_| true), 0, "nothing was sent");
}

#[test]
fn redirects_stay_within_the_hosts() {
    let (server, env) = served();
    let r =
        run_with(&env, "const r = await ctx.http.get('https://api.example.org/to-other'); return [r.status, r.url, r.json()];", json!({}))
            .unwrap();
    assert_eq!(r, json!([200, "https://other.example.org/json", {"other": true}]));
    for to in ["to-evil", "to-ip"] {
        let e = run_with(&env, &format!("return await ctx.http.get('https://api.example.org/{to}');"), json!({})).unwrap_err();
        assert!(matches!(e, CallError::Denied(_)), "{to}: {e:?}");
    }
    let e = run_with(&env, "return await ctx.http.get('https://api.example.org/loop');", json!({})).unwrap_err();
    assert!(matches!(e, CallError::Denied(_) | CallError::Limit(Limit::Requests, _)), "{e:?}");
    assert_eq!(server.count(|r| r.path().starts_with("/evil") || r.path() == "/"), 0);
}

#[test]
fn requests_carry_the_apps_user_agent_and_the_scripts_options() {
    let (_server, env) = served();
    let r = run_with(
        &env,
        r#"
        const get = (await ctx.http.get("https://api.example.org/echo", { headers: { Authorization: "Bearer t" } })).json();
        const json = (await ctx.http.post("https://api.example.org/echo", { json: { a: 1 } })).json();
        const form = (await ctx.http.post("https://api.example.org/echo", { form: { q: "a b", n: 2 } })).json();
        let ua;
        try { await ctx.http.get("https://api.example.org/echo", { headers: { "User-Agent": "spoof" } }); } catch (e) { ua = String(e); }
        return { get, json, form, ua };
    "#,
        json!({}),
    )
    .unwrap();
    assert_eq!(r["get"]["ua"], APP_UA);
    assert_eq!(r["get"]["auth"], "Bearer t");
    assert_eq!(
        (r["json"]["method"].as_str(), r["json"]["ct"].as_str(), r["json"]["body"].as_str()),
        (Some("POST"), Some("application/json"), Some("{\"a\":1}"))
    );
    assert_eq!(r["form"]["body"], "q=a+b&n=2");
    assert!(r["ua"].as_str().unwrap().contains("set by the app"));
}

#[test]
fn answers_over_the_size_cap_end_the_call() {
    let (_server, env) = served();
    let limits = Limits { response_bytes: 1024 * 1024, ..quick() };
    let env = HostEnv { limits, ..env };
    let e = run_with(&env, "try { await ctx.http.get('https://api.example.org/big'); } catch (e) {} return 'ok';", json!({})).unwrap_err();
    assert!(matches!(e, CallError::Limit(Limit::Bytes, _)), "{e:?}");
}

#[test]
fn an_unreachable_service_is_not_a_failure() {
    // A server that closes every connection unanswered. Not a closed port:
    // Windows tries a refused connection again for about two seconds, past
    // the call's time here.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", dead.local_addr().unwrap());
    std::thread::spawn(move || {
        for s in dead.incoming().flatten() {
            drop(s);
        }
    });
    let http = Http::new(heronite_metadata::Endpoints { origins: vec![("https://api.example.org".into(), base)], ..Default::default() });
    let e = run_with(&env(http, quick()), "return (await ctx.http.get('https://api.example.org/json')).json();", json!({})).unwrap_err();
    assert!(matches!(e, CallError::Unreachable(_)), "{e:?}");
    assert!(!e.is_failure());
}

#[test]
fn the_rate_is_kept_across_calls() {
    let (server, env) = served();
    let env = HostEnv { rate: Arc::new(RateLimiter::new(Duration::from_millis(150))), ..env };
    let started = Instant::now();
    run_with(&env, "for (let i = 0; i < 3; i++) await ctx.http.get('https://api.example.org/json'); return 1;", json!({})).unwrap();
    run_with(&env, "await ctx.http.get('https://api.example.org/json'); return 1;", json!({})).unwrap();
    // On Windows the timer a sleep waits on and `Instant` are different
    // clocks: a wait may end a little earlier as `Instant` measures it.
    let early = Duration::from_millis(if cfg!(windows) { 50 } else { 0 });
    assert!(started.elapsed() + early >= Duration::from_millis(450), "{:?}", started.elapsed());
    assert_eq!(server.count(|_| true), 4);
    // A rate that leaves no time for a request ends the call; the service
    // could not be asked now, which is no failure of the extension.
    let slow = HostEnv { rate: Arc::new(RateLimiter::new(Duration::from_secs(60))), ..env };
    let e = run_with(
        &slow,
        "await ctx.http.get('https://api.example.org/json'); await ctx.http.get('https://api.example.org/json');",
        json!({}),
    )
    .unwrap_err();
    assert!(matches!(e, CallError::Unreachable(_)) && !e.is_failure(), "{e:?}");
}

#[test]
fn a_slow_rate_never_turns_into_failures() {
    let (server, env) = served();
    // One request every 2 s, calls of 1.5 s: a call while the next slot is
    // too far gives up without using it.
    let env = HostEnv { rate: Arc::new(RateLimiter::new(Duration::from_secs(2))), ..env };
    let get = "return (await ctx.http.get('https://api.example.org/json')).status;";
    let started = Instant::now();
    assert_eq!(run_with(&env, get, json!({})).unwrap(), json!(200));
    for _ in 0..6 {
        let e = run_with(&env, get, json!({})).unwrap_err();
        assert!(matches!(e, CallError::Unreachable(_)) && !e.is_failure(), "{e:?}");
    }
    // Once the slot comes, the next call goes out as if nothing happened.
    std::thread::sleep((started + Duration::from_millis(2050)).saturating_duration_since(Instant::now()));
    assert_eq!(run_with(&env, get, json!({})).unwrap(), json!(200));
    assert_eq!(server.count(|_| true), 2);
}

#[test]
fn html_documents_answer_css_selectors() {
    let (_server, env) = served();
    let r = run_with(
        &env,
        r##"
        const doc = ctx.html.parse((await ctx.http.get("https://api.example.org/page")).text);
        const links = doc.all("ul a");
        let bad;
        try { doc.text("::nonsense(("); } catch (e) { bad = String(e); }
        return {
            title: doc.text("h1.t"),
            missing: doc.text(".nope"),
            hrefs: links.map((a) => a.attr("href")),
            texts: links.map((a) => a.text()),
            year: doc.attr("#info span", "data-year"),
            nested: doc.all("#info").map((d) => d.text("span"))[0],
            html: links[0].html(),
            bad,
        };
    "##,
        json!({}),
    )
    .unwrap();
    assert_eq!(r["title"], "Portal 2");
    assert_eq!(r["missing"], Value::Null);
    assert_eq!(r["hrefs"], json!(["/a", "/b"]));
    assert_eq!(r["texts"], json!(["A", "B"]));
    assert_eq!(r["year"], "2011");
    assert_eq!(r["nested"], "Released");
    assert_eq!(r["html"], "<a href=\"/a\">A</a>");
    assert!(r["bad"].as_str().unwrap().contains("selector"));
}

#[test]
fn dates_titles_language_and_logs() {
    let r = run(r#"
        for (let i = 0; i < 1000; i++) ctx.log("line " + i);
        return [ctx.date.parse("April 18, 2011"), ctx.date.parse("someday"), ctx.normalize("Pokémon: Red & Blue!"), ctx.language];
    "#)
    .unwrap();
    assert_eq!(r, json!(["2011-04-18", null, heronite_metadata::util::normalize_title("Pokémon: Red & Blue!"), "pt-BR"]));
}

#[test]
fn settings_are_the_extensions_own() {
    let r = run(
        "return [ctx.settings.get('token'), ctx.settings.get('hd'), ctx.settings.get('clientSecret'), ctx.settings.get('igdbClientId')];",
    )
    .unwrap();
    assert_eq!(r, json!(["mine", true, null, null]));
}

/// The cost of a call: a fresh runtime, the bytecode loaded, one call.
#[test]
fn a_runtime_per_call_is_cheap() {
    let env = env(Http::default(), Limits::default());
    let source = std::fs::read_to_string(support::example("wikipedia").join("source.js")).unwrap();
    let started = Instant::now();
    let script = Script::compile(&source, &env).unwrap();
    let compile = started.elapsed();
    let n = 200;
    let started = Instant::now();
    for _ in 0..n {
        let r = session(&script, &env, |c| c.call("fetch", None).is_err()).unwrap();
        assert!(r, "fetch without an id fails");
    }
    let per_call = started.elapsed() / n;
    eprintln!("compile {compile:?}, session + call {per_call:?} (a thread, a runtime, the module loaded, one call)");
    assert!(per_call < Duration::from_millis(50), "{per_call:?}");
}

#[test]
fn the_same_script_runs_on_many_threads_at_once() {
    let (server, env) = served();
    let script =
        Script::compile(&module("return (await ctx.http.get('https://api.example.org/json', { query: { q: game.n } })).json().q;"), &env)
            .unwrap();
    let results: Vec<Value> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let (script, env) = (&script, &env);
                s.spawn(move || session(script, env, move |c| c.call("find", Some(json!({"n": i.to_string()})))).unwrap().unwrap())
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(results, (0..12).map(|i| json!(i.to_string())).collect::<Vec<_>>());
    assert_eq!(server.count(|_| true), 12);
}
