//! A script that tries to make the host copy or keep more than its limits,
//! to reach this computer, to leave a key in a message, or to pass its own
//! failures off as an unreachable service.

mod support;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use heronite_metadata_ext::engine::{session, CallError, HostEnv, Limit, Script};
use heronite_metadata_ext::net::{HostRule, RateLimiter};
use heronite_metadata_ext::Limits;
use serde_json::{json, Value};
use support::{endpoints, Http, Resp, Server};

const HOSTS: [&str; 1] = ["api.example.org"];

fn limits() -> Limits {
    Limits { time: Duration::from_millis(2500), ..Limits::default() }
}

fn env(http: Http) -> HostEnv {
    HostEnv {
        id: "hostile".into(),
        http,
        hosts: Arc::new(HOSTS.iter().map(|h| HostRule::parse(h).unwrap()).collect()),
        rate: Arc::new(RateLimiter::new(Duration::ZERO)),
        limits: limits(),
        settings: Arc::new(serde_json::Map::new()),
        language: "en".into(),
    }
}

fn run(env: &HostEnv, body: &str) -> Result<Value, CallError> {
    let source = format!("export async function find(game, ctx) {{\n{body}\n}}\nexport function fetch() {{ return null; }}\n");
    let script = Script::compile(&source, env).unwrap_or_else(|e| panic!("{e}"));
    session(&script, env, |c| c.call("find", Some(json!({})))).and_then(|r| r)
}

fn served() -> (Server, HostEnv) {
    let server = Server::start(|req| match req.path() {
        "/api.example.org/json" => Resp::json(json!({"ok": true})),
        _ => Resp::status(404),
    });
    let env = env(Http::new(endpoints(&server, &HOSTS)));
    (server, env)
}

#[test]
fn keys_count_against_the_text_a_script_may_return() {
    // 40,000 references to one object with an 8 MB key: 320 GB if each
    // key were copied out.
    let started = Instant::now();
    let e = run(&env(Http::default()), r#"const k = "x".repeat(8 << 20); const o = {}; o[k] = 1; return new Array(40000).fill(o);"#)
        .unwrap_err();
    assert!(matches!(&e, CallError::Limit(Limit::Size, m) if m.contains("bytes of text")), "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    // Keys under the budget still come through.
    assert_eq!(run(&env(Http::default()), "return { a: 1, b: { c: 'd' } };").unwrap(), json!({"a": 1, "b": {"c": "d"}}));
}

#[test]
fn headers_query_and_form_are_bounded_before_they_are_copied() {
    let (server, env) = served();
    for (what, options) in [("headers", "{ headers: h }"), ("query", "{ query: h }"), ("form fields", "{ form: h }")] {
        // 2,000 values of 16 MB: 32 GB if each were copied out.
        let body = format!(
            r#"const b = "x".repeat(16e6); const h = {{}}; for (let i = 0; i < 2000; i++) h["a" + i] = b;
               await ctx.http.post("https://api.example.org/json", {options});"#
        );
        let started = Instant::now();
        let e = run(&env, &body).unwrap_err();
        assert!(matches!(&e, CallError::Script(m) if m.contains(&format!("the {what} of a request are larger"))), "{what}: {e:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "{what}: {:?}", started.elapsed());
    }
    let e = run(&env, r#"await ctx.http.get("https://api.example.org/" + "x".repeat(1e6));"#).unwrap_err();
    assert!(matches!(&e, CallError::Script(m) if m.contains("too long")), "{e:?}");
    assert_eq!(server.count(|_| true), 0, "nothing was sent");
}

#[test]
fn parsed_documents_are_bounded_per_call() {
    let env = env(Http::default());
    // A million tags in 4 MB: refused before it is parsed.
    let e = run(&env, r#"ctx.html.parse("<br>".repeat(1e6)); return 1;"#).unwrap_err();
    assert!(matches!(&e, CallError::Script(m) if m.contains("too large")), "{e:?}");
    // Any number of small documents: eight per call.
    let r = run(&env, "let n = 0; try { for (;;) { ctx.html.parse('<p>x</p>'); n++; } } catch (e) { return [n, String(e)]; }").unwrap();
    assert_eq!(r[0], json!(8));
    assert!(r[1].as_str().unwrap().contains("documents"));
    // Bytes of all of them together: 3 MB documents, two of them.
    let r = run(
        &env,
        r#"const d = "<p>" + "x".repeat(3e6) + "</p>"; let n = 0; try { for (;;) { ctx.html.parse(d); n++; } } catch (e) { return n; }"#,
    )
    .unwrap();
    assert_eq!(r, json!(2));
}

#[test]
fn a_name_of_this_computer_is_never_reached() {
    // A host the manifest could list (the rule is built by hand: the
    // manifest refuses a name without a dot) whose name leads home.
    let mut env = env(Http::default());
    env.hosts = Arc::new(vec![HostRule { host: "localhost".into(), subdomains: false, http: true }]);
    let e = run(&env, "try { await ctx.http.get('http://localhost/admin'); } catch (e) {} return 'went on';").unwrap_err();
    assert!(matches!(&e, CallError::Denied(m) if m.contains("local network")), "{e:?}");
    assert!(e.is_failure());
}

/// A server that accepts and never answers.
fn silent() -> (TcpListener, Http) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let http = Http::new(heronite_metadata::Endpoints { origins: vec![("https://api.example.org".into(), base)], ..Default::default() });
    (l, http)
}

#[test]
fn a_service_too_slow_to_answer_is_unreachable_not_a_failure() {
    let (listener, http) = silent();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in listener.incoming().flatten() {
            held.push(s);
        }
    });
    // The wait uses up the call's time; the request's own error ends it.
    let e = run(&env(http), "return (await ctx.http.get('https://api.example.org/json?key=secret')).json();").unwrap_err();
    assert!(matches!(e, CallError::Unreachable(_)), "{e:?}");
    assert!(!e.is_failure());
    assert!(!e.to_string().contains("secret"), "no query in the message: {e}");
}

/// A server that closes every connection at once, unanswered. Not a closed
/// port: Windows tries a refused connection again for about two seconds,
/// most of a call's time here.
fn closing() -> Http {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            drop(s);
        }
    });
    Http::new(heronite_metadata::Endpoints { origins: vec![("https://api.example.org".into(), base)], ..Default::default() })
}

#[test]
fn its_own_errors_after_an_unreachable_request_are_failures() {
    let http = closing();
    let env = env(http);
    // Uncaught, the request's error is unreachable, its address shown
    // without the query.
    let e = run(&env, "await ctx.http.get('https://api.example.org/json?apikey=secret');").unwrap_err();
    assert!(matches!(&e, CallError::Unreachable(m) if !m.contains("secret")), "{e:?}");
    // Caught, then another error of the script's own: its failure.
    let e = run(&env, "try { await ctx.http.get('https://api.example.org/json'); } catch (e) {} null.boom;").unwrap_err();
    assert!(matches!(e, CallError::Script(_)) && e.is_failure(), "{e:?}");
    // A header that would split the request: refused, nothing sent, its failure.
    let (server, served) = served();
    let e = run(&served, r#"await ctx.http.get("https://api.example.org/json", { headers: { "X-A": "a\r\nHost: evil" } });"#).unwrap_err();
    assert!(matches!(&e, CallError::Script(m) if m.contains("not a valid")), "{e:?}");
    assert_eq!(server.count(|_| true), 0);
}

#[test]
fn a_denied_address_shows_no_key() {
    let env = env(Http::default());
    let e = run(&env, "await ctx.http.get('https://elsewhere.example.com/x?token=secret#also');").unwrap_err();
    assert!(
        matches!(&e, CallError::Denied(m) if m.contains("elsewhere.example.com") && !m.contains("secret") && !m.contains("also")),
        "{e:?}"
    );
}
