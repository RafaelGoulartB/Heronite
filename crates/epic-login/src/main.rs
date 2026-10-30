//! Store login window. Opens a store's login page in an embedded WebKit
//! view, waits for the authorization code the store hands back and prints it
//! to stdout, then exits with status 0. Exits with [`CLOSED`] when the
//! window is closed first; any other status means it failed (the app tells
//! the user then).
//!
//! * no arguments (or `--store epic`): Epic Games;
//! * `--store gog`: GOG (the code comes back in the `on_login_success` page URL);
//! * `--store amazon --url <sign-in URL>`: Amazon Games (the URL is made by
//!   nile for this login; the code comes back in the return URL).
//!
//! The main app spawns this helper and reads stdout; it lives in its own
//! binary so the GPUI app does not carry a web engine.
//!
//! `--smoke` loads a local page instead (never touches the network): prints
//! `SMOKE-READY` once it is loaded and stays open for a few seconds, for
//! checking that the window opens where expected.

// The window is WebKitGTK's (Linux); elsewhere only the tests use the
// parsing below.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use url::Url;

/// Exit status when the user closes the window before a code arrived (GTK
/// itself exits with 1 when it loses the display).
const CLOSED: i32 = 4;

const EPIC_LOGIN_URL: &str = "https://www.epicgames.com/id/login?responseType=code";
const EPIC_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) EpicGamesLauncher";
const GOG_LOGIN_URL: &str = "https://auth.gog.com/auth?client_id=46899977096215655&redirect_uri=https%3A%2F%2Fembed.gog.com%2Fon_login_success%3Forigin%3Dclient&response_type=code&layout=galaxy";
/// A desktop browser: the GOG and Amazon pages refuse unknown web views.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Store {
    Epic,
    Gog,
    Amazon,
}

impl Store {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "epic" => Some(Self::Epic),
            "gog" => Some(Self::Gog),
            "amazon" => Some(Self::Amazon),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Epic => "Epic Games login",
            Self::Gog => "GOG login",
            Self::Amazon => "Amazon Games login",
        }
    }

    fn user_agent(self) -> &'static str {
        match self {
            Self::Epic => EPIC_USER_AGENT,
            Self::Gog | Self::Amazon => BROWSER_USER_AGENT,
        }
    }

    /// The authorization code in a URL the store navigates to.
    fn code_from_url(self, url: &str) -> Option<String> {
        let url = Url::parse(url).ok()?;
        let (host_ok, param) = match self {
            // Epic redirects to https://localhost/launcher/authorized?code=...
            Self::Epic => (url.host_str() == Some("localhost"), "code"),
            Self::Gog => (url.host_str() == Some("embed.gog.com") && url.path().starts_with("/on_login_success"), "code"),
            // Any amazon.com page that carries the code (the sign-in's return URL).
            Self::Amazon => {
                (url.host_str().is_some_and(|h| h == "amazon.com" || h.ends_with(".amazon.com")), "openid.oa2.authorization_code")
            }
        };
        if !host_ok {
            return None;
        }
        url.query_pairs().find(|(k, _)| k == param).map(|(_, v)| v.into_owned()).filter(|c| !c.is_empty())
    }
}

/// What the command line asks for: the store and the page to open.
fn parse_args(args: &[String]) -> Result<(Store, String), String> {
    let mut store = Store::Epic;
    let mut url = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--store" => {
                let name = it.next().ok_or("--store needs a value")?;
                store = Store::parse(name).ok_or_else(|| format!("unknown store {name}"))?;
            }
            "--url" => url = Some(it.next().ok_or("--url needs a value")?.clone()),
            "--smoke" => {}
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let url = match (store, url) {
        (_, Some(u)) => u,
        (Store::Epic, None) => EPIC_LOGIN_URL.to_string(),
        (Store::Gog, None) => GOG_LOGIN_URL.to_string(),
        (Store::Amazon, None) => return Err("--store amazon needs --url".into()),
    };
    Ok((store, url))
}

/// The code in the JSON page of `epicgames.com/id/api/redirect`
/// (`{"redirectUrl": ..., "sid": null, "authorizationCode": "..."}`), given
/// the text the page shows. `eval` results arrive JSON-encoded as a string.
fn code_from_page_text(text: &str) -> Option<String> {
    let text = match serde_json::from_str::<String>(text) {
        Ok(inner) => inner,
        Err(_) => text.to_string(),
    };
    let doc: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    doc.get("authorizationCode")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .or_else(|| doc.get("redirectUrl").and_then(|u| u.as_str()).and_then(|u| Store::Epic.code_from_url(u)))
}

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::mpsc;
    use std::time::Duration;

    use gtk::glib;
    use gtk::prelude::*;
    use wry::{PageLoadEvent, WebViewBuilder, WebViewBuilderExtUnix};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke = args.iter().any(|a| a == "--smoke");
    let (store, start_url) = match parse_args(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // WebKitGTK's DMA-BUF renderer gets the window killed with a Wayland
    // protocol error (71) as soon as it is drawn on some drivers (NVIDIA); a
    // login page renders fine without it. Set before GTK starts any thread.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    glib::set_prgname(Some("heronite-epic-login"));
    glib::set_application_name(store.title());
    if gtk::init().is_err() {
        eprintln!("could not initialise GTK (no display?)");
        std::process::exit(2);
    }

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(store.title());
    window.set_default_size(520, 720);
    window.connect_delete_event(|_, _| {
        // Closed before a code arrived.
        std::process::exit(CLOSED);
    });

    let (tx, rx) = mpsc::channel::<String>();
    let (tx_nav, tx_load) = (tx.clone(), tx.clone());
    let mut builder = WebViewBuilder::new()
        .with_incognito(true)
        .with_user_agent(store.user_agent())
        // The page that carries the code is not loaded: take the code and stop there.
        .with_navigation_handler(move |url| match store.code_from_url(&url) {
            Some(code) => {
                let _ = tx_nav.send(code);
                false
            }
            None => true,
        });
    builder = if smoke {
        builder.with_html("<html><body style='font-family:sans-serif;background:#222;color:#eee'><h2>Login helper</h2><p>Smoke test page.</p></body></html>")
    } else {
        builder.with_url(&start_url)
    };
    let webview = std::rc::Rc::new(
        builder
            .with_on_page_load_handler(move |event, url| {
                if !matches!(event, PageLoadEvent::Finished) {
                    return;
                }
                if smoke {
                    println!("SMOKE-READY");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                    return;
                }
                if let Some(code) = store.code_from_url(&url) {
                    let _ = tx_load.send(code);
                }
            })
            .build_gtk(&window)
            .expect("creating the web view"),
    );

    // Epic's JSON page of /id/api/redirect shows the code as text.
    let page_check = webview.clone();
    let tx_page = tx.clone();
    let mut last_url = String::new();
    glib::timeout_add_local(Duration::from_millis(700), move || {
        if smoke || store != Store::Epic {
            return glib::ControlFlow::Break;
        }
        if let Ok(url) = page_check.url() {
            if url != last_url {
                last_url = url.clone();
                if url.contains("/id/api/redirect") {
                    let tx = tx_page.clone();
                    let _ = page_check.evaluate_script_with_callback("document.body ? document.body.innerText : ''", move |text| {
                        if let Some(code) = code_from_page_text(&text) {
                            let _ = tx.send(code);
                        }
                    });
                }
            }
        }
        glib::ControlFlow::Continue
    });

    // Hand the code over on the main thread and finish.
    let started = std::time::Instant::now();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Ok(code) = rx.try_recv() {
            println!("{code}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            gtk::main_quit();
            return glib::ControlFlow::Break;
        }
        if smoke && started.elapsed() > Duration::from_secs(15) {
            gtk::main_quit();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });

    window.show_all();
    gtk::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The login window is only available on Linux for now; use the browser login.");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn epic_codes_are_found_in_redirects() {
        let epic = Store::Epic;
        assert_eq!(epic.code_from_url("https://localhost/launcher/authorized?code=abc123").as_deref(), Some("abc123"));
        assert_eq!(epic.code_from_url("http://localhost:3000/?state=x&code=zzz").as_deref(), Some("zzz"));
        assert_eq!(epic.code_from_url("https://www.epicgames.com/id/login?code=nope"), None);
        assert_eq!(epic.code_from_url("https://localhost/launcher/authorized"), None);
        assert_eq!(epic.code_from_url("not a url"), None);
    }

    #[test]
    fn gog_and_amazon_codes_are_found_in_their_return_urls() {
        let gog = Store::Gog;
        assert_eq!(gog.code_from_url("https://embed.gog.com/on_login_success?origin=client&code=G0G").as_deref(), Some("G0G"));
        assert_eq!(gog.code_from_url("https://auth.gog.com/auth?code=nope"), None);
        assert_eq!(gog.code_from_url("https://embed.gog.com/on_login_success?origin=client"), None);
        let amazon = Store::Amazon;
        let back =
            "https://www.amazon.com/?openid.assoc_handle=amzn_sonic_games_launcher&openid.oa2.authorization_code=ANxyz&openid.mode=id_res";
        assert_eq!(amazon.code_from_url(back).as_deref(), Some("ANxyz"));
        assert_eq!(amazon.code_from_url("https://www.amazon.com/ap/signin?openid.mode=checkid_setup"), None);
        assert_eq!(amazon.code_from_url("https://evil.example/?openid.oa2.authorization_code=x"), None);
    }

    #[test]
    fn arguments_pick_the_store_and_page() {
        assert_eq!(parse_args(&[]).unwrap(), (Store::Epic, EPIC_LOGIN_URL.to_string()));
        assert_eq!(parse_args(&args(&["--smoke"])).unwrap().0, Store::Epic);
        assert_eq!(parse_args(&args(&["--store", "gog"])).unwrap(), (Store::Gog, GOG_LOGIN_URL.to_string()));
        assert_eq!(parse_args(&args(&["--store", "amazon", "--url", "https://amazon.com/ap/signin?x"])).unwrap().0, Store::Amazon);
        assert!(parse_args(&args(&["--store", "amazon"])).is_err());
        assert!(parse_args(&args(&["--store", "steam"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }

    #[test]
    fn codes_are_found_in_the_json_page() {
        let page = r#"{"redirectUrl":"https://localhost/launcher/authorized?code=fromurl","sid":null,"authorizationCode":"thecode"}"#;
        assert_eq!(code_from_page_text(page).as_deref(), Some("thecode"));
        // `eval` hands text back JSON-encoded.
        let encoded = serde_json::to_string(page).unwrap();
        assert_eq!(code_from_page_text(&encoded).as_deref(), Some("thecode"));
        let no_code = r#"{"redirectUrl":"https://localhost/launcher/authorized?code=fromurl","authorizationCode":null}"#;
        assert_eq!(code_from_page_text(no_code).as_deref(), Some("fromurl"));
        assert_eq!(code_from_page_text("\"<html>\""), None);
    }
}
