//! Store login window. Opens a store's login page in an embedded web view
//! (WebKitGTK on Linux, WebView2 on Windows), waits for the authorization
//! code the store hands back and prints it to stdout, then exits with status
//! 0. Exits with [`CLOSED`] when the window is closed first (on Windows also
//! when the program that started it exits), and on Windows with 5 when the
//! WebView2 runtime is not installed; any other status means it failed (the
//! app tells the user then).
//!
//! * no arguments (or `--store epic`): Epic Games;
//! * `--store gog`: GOG (the code comes back in the `on_login_success` page URL);
//! * `--store amazon --url <sign-in URL>`: Amazon Games (the URL is made by
//!   nile for this login; the code comes back in the return URL).
//! * `--store xbox --url <sign-in URL>`: a Microsoft account, for Xbox (the
//!   URL is made by the app for this sign-in). What it prints is the whole
//!   address the sign-in ends on (`login.live.com/oauth20_desktop.srf?code=…&state=…`,
//!   or `?error=…` when the user said no): the app checks it is its sign-in.
//! * `--store ea`: the EA account. The user signs in on EA's site
//!   ([`EA_LOGIN_URL`]), which then shows another of its pages: the window
//!   asks EA there for a token of its web client ([`EA_TOKEN_URL`]), which
//!   EA answers as JSON. Once it did, what is printed is
//!   the window's cookies for `accounts.ea.com` (`{"cookies":[{"name":…,
//!   "value":…}]}`), the web session the app asks for tokens with; the
//!   token itself is left behind.
//!
//! The main app spawns this helper and reads stdout; it lives in its own
//! binary so the GPUI app does not carry a web engine.
//!
//! `--smoke` loads a local page instead (never touches the network): prints
//! `SMOKE-READY` once it is loaded and stays open for a few seconds, for
//! checking that the window opens where expected.

// A window program on Windows: no console opens with it (the app reads its
// redirected stdout all the same).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
// The window is WebKitGTK's (Linux) or WebView2's (Windows); elsewhere only
// the tests use the parsing below.
#![cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]

#[cfg(windows)]
mod windows;

use url::Url;

/// Exit status when the user closes the window before a code arrived (GTK
/// itself exits with 1 when it loses the display).
const CLOSED: i32 = 4;

/// Epic's login with no client named: for the launcher's user agent
/// ([`EPIC_USER_AGENT`]) Epic picks legendary's client
/// (`34a02cf8f4414e29b15921876da36f9a`) and, once signed in, sends the page to
/// that client's own redirect, `https://localhost/launcher/authorized?code=...`,
/// where the code is taken. A code for any other client is refused by
/// `legendary auth`.
///
/// Not legendary's `redirectUrl=https://www.epicgames.com/id/api/redirect?...`
/// (what `legendary.gl/epiclogin` opens): Epic no longer knows that redirect
/// for the client and the page stops on "Invalid Client - Redirect URL is not
/// known to the client" after signing in. Not `clientId=` either: the page then
/// takes the window for the launcher and waits for its JS bridge.
const EPIC_LOGIN_URL: &str = "https://www.epicgames.com/id/login?responseType=code";
const EPIC_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) EpicGamesLauncher";
/// The sign-in of EA's own site. Not [`EA_TOKEN_URL`]'s client: EA no longer
/// signs anyone in for it ("Your request cannot be completed. Service
/// limitations apply." after the password), though it still gives its token
/// to a session signed in here.
const EA_LOGIN_URL: &str = "https://www.ea.com/login";
/// EA's token for the client of its web pages, answered as JSON
/// (`nucleus:rest`), without a page (`prompt=none`): `login_required` with
/// no session. The clients of Lutris and Playnite ask the same.
const EA_TOKEN_URL: &str =
    "https://accounts.ea.com/connect/auth?client_id=ORIGIN_JS_SDK&response_type=token&redirect_uri=nucleus:rest&prompt=none";
/// The address whose cookies are the session (the app sends them there).
const EA_COOKIES_URL: &str = "https://accounts.ea.com/connect/auth";
/// What the EA sign-in hands over in place of a code: the cookies follow.
const EA_DONE: &str = "ea-signed-in";
/// How many times the window goes to [`EA_TOKEN_URL`] from another page of
/// EA's site (each return to [`EA_LOGIN_URL`] from a `login_required`
/// follows one).
const EA_MAX_RETURNS: u32 = 5;
const GOG_LOGIN_URL: &str = "https://auth.gog.com/auth?client_id=46899977096215655&redirect_uri=https%3A%2F%2Fembed.gog.com%2Fon_login_success%3Forigin%3Dclient&response_type=code&layout=galaxy";
/// A desktop browser: the GOG and Amazon pages refuse unknown web views.
/// (WebView2's own user agent is one already: Microsoft Edge's.)
#[cfg(target_os = "linux")]
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Store {
    Epic,
    Gog,
    Amazon,
    Xbox,
    Ea,
}

impl Store {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "epic" => Some(Self::Epic),
            "gog" => Some(Self::Gog),
            "amazon" => Some(Self::Amazon),
            "xbox" => Some(Self::Xbox),
            "ea" => Some(Self::Ea),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Epic => "Epic Games login",
            Self::Gog => "GOG login",
            Self::Amazon => "Amazon Games login",
            Self::Xbox => "Xbox sign-in",
            Self::Ea => "EA sign-in",
        }
    }

    #[cfg(target_os = "linux")]
    fn user_agent(self) -> &'static str {
        match self {
            Self::Epic => EPIC_USER_AGENT,
            Self::Gog | Self::Amazon | Self::Xbox | Self::Ea => BROWSER_USER_AGENT,
        }
    }

    /// The authorization code in a URL the store navigates to.
    fn code_from_url(self, url: &str) -> Option<String> {
        let address = url;
        let url = Url::parse(url).ok()?;
        // Microsoft's desktop redirect: the whole address, for the app to check.
        if self == Self::Xbox {
            let done = url.host_str() == Some("login.live.com")
                && url.path() == "/oauth20_desktop.srf"
                && url.query_pairs().any(|(k, v)| (k == "code" || k == "error") && !v.is_empty());
            return done.then(|| address.to_string());
        }
        // The token is sent to `nucleus:rest` when EA redirects instead of
        // answering on its own address. An error sent there ends the window
        // too (the page could not open the address): the app finds out
        // from the session it is handed.
        if self == Self::Ea {
            return (url.scheme() == "nucleus").then(|| EA_DONE.to_string());
        }
        let (host_ok, param) = match self {
            // Epic redirects to https://localhost/launcher/authorized?code=...
            Self::Epic => (url.host_str() == Some("localhost"), "code"),
            Self::Gog => (url.host_str() == Some("embed.gog.com") && url.path().starts_with("/on_login_success"), "code"),
            // Any amazon.com page that carries the code (the sign-in's return URL).
            Self::Amazon => {
                (url.host_str().is_some_and(|h| h == "amazon.com" || h.ends_with(".amazon.com")), "openid.oa2.authorization_code")
            }
            Self::Xbox | Self::Ea => return None,
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
        (Store::Xbox, None) => return Err("--store xbox needs --url".into()),
        (Store::Ea, None) => EA_LOGIN_URL.to_string(),
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

/// Where the EA sign-in is, by the address of the page shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EaPage {
    /// `connect/auth` itself: its text may be the token.
    Token,
    /// Another page of EA's site (not its sign-in): the window goes to
    /// [`EA_TOKEN_URL`], which answers at once for a signed-in session.
    Elsewhere,
    /// The sign-in (EA's, and the pages of `www.ea.com` it passes through),
    /// or a page of another site.
    Other,
}

fn ea_page(url: &str) -> EaPage {
    let Ok(url) = Url::parse(url) else { return EaPage::Other };
    let host = url.host_str().unwrap_or("");
    if url.scheme() != "https" || !(host == "ea.com" || host.ends_with(".ea.com")) {
        return EaPage::Other;
    }
    match host {
        // Not the client EA's site signs in with on the way.
        "accounts.ea.com" if url.path() == "/connect/auth" => match url.query_pairs().find(|(k, _)| k == "client_id") {
            Some((_, id)) if id != "ORIGIN_JS_SDK" => EaPage::Other,
            _ => EaPage::Token,
        },
        "accounts.ea.com" | "signin.ea.com" => EaPage::Other,
        // `/login`, then `/login_check` once EA signed the user in.
        "www.ea.com" | "ea.com" if url.path_segments().is_some_and(|mut s| s.any(|s| s == "login" || s == "login_check")) => EaPage::Other,
        _ => EaPage::Elsewhere,
    }
}

/// Reads the text of `connect/auth` (in a `<pre>` of its own when a JSON
/// viewer formats it), for [`ea_token_in`]. A window not signed in
/// (`login_required`) goes on to [`EA_LOGIN_URL`] from the page itself, once
/// (the page is read again while the next one loads): the callback that
/// gets the text cannot reach the view.
const EA_READ_SCRIPT: &str = concat!(
    "(function(){var p=document.querySelector('pre');",
    "var t=p?p.innerText:(document.body?document.body.innerText:'');",
    "try{var d=JSON.parse(t.slice(t.indexOf('{'),t.lastIndexOf('}')+1));",
    "if((d.error==='login_required'||d.error_code==='login_required')&&!d.access_token&&!window.__heroniteBack)",
    "{window.__heroniteBack=1;location.replace('https://www.ea.com/login');}}catch(e){}",
    "return t;})()"
);

/// The text of `connect/auth` holds a token (JSON with `access_token`;
/// `eval` hands it back JSON-encoded, and a JSON viewer may add text).
fn ea_token_in(text: &str) -> bool {
    let text = serde_json::from_str::<String>(text).unwrap_or_else(|_| text.to_string());
    let json = match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if a < b => &text[a..=b],
        _ => return false,
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|d| d.get("access_token").and_then(|t| t.as_str()).map(|t| !t.is_empty()))
        .unwrap_or(false)
}

/// What the EA sign-in prints: the cookies, as JSON.
fn ea_cookies_json<'a>(cookies: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let list: Vec<serde_json::Value> = cookies.into_iter().map(|(name, value)| serde_json::json!({"name": name, "value": value})).collect();
    serde_json::json!({ "cookies": list }).to_string()
}

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::mpsc;
    use std::time::Duration;

    use gtk::glib;
    use gtk::prelude::*;
    use wry::{PageLoadEvent, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

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
    let (tx_nav, tx_load, tx_popup) = (tx.clone(), tx.clone(), tx.clone());
    // The popups and their views live as long as the login window.
    let popups: std::rc::Rc<std::cell::RefCell<Vec<(gtk::Window, wry::WebView)>>> = Default::default();
    let (popup_parent, popup_list) = (window.clone(), popups.clone());
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
        })
        // "Sign in with Google" and the like may open a popup, which
        // WebKitGTK drops unless it is given a view.
        .with_new_window_req_handler(move |_, features| {
            match open_popup(store, &popup_parent, features.opener.webview, tx_popup.clone()) {
                Some((popup, view)) => {
                    let webview = view.webview();
                    popup_list.borrow_mut().push((popup, view));
                    wry::NewWindowResponse::Create { webview }
                }
                None => wry::NewWindowResponse::Deny,
            }
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

    // Epic's JSON page of /id/api/redirect shows the code as text, and EA's
    // connect/auth its token. Read again while it is open: the address
    // changes before the text is there.
    let page_check = webview.clone();
    let tx_page = tx.clone();
    let ea_returns = std::cell::Cell::new(0u32);
    let ea_left_from = std::cell::RefCell::new(String::new());
    glib::timeout_add_local(Duration::from_millis(700), move || {
        if smoke || !matches!(store, Store::Epic | Store::Ea) {
            return glib::ControlFlow::Break;
        }
        if store == Store::Ea {
            let Ok(url) = page_check.url() else { return glib::ControlFlow::Continue };
            // Left once while it shows: the window may come back to it
            // later (signed in by then).
            if *ea_left_from.borrow() != url {
                ea_left_from.borrow_mut().clear();
            }
            match ea_page(&url) {
                EaPage::Token => {
                    let tx = tx_page.clone();
                    let _ = page_check.evaluate_script_with_callback(EA_READ_SCRIPT, move |text| {
                        if ea_token_in(&text) {
                            let _ = tx.send(EA_DONE.to_string());
                        }
                    });
                }
                EaPage::Elsewhere if *ea_left_from.borrow() != url && ea_returns.get() < EA_MAX_RETURNS => {
                    ea_returns.set(ea_returns.get() + 1);
                    *ea_left_from.borrow_mut() = url;
                    let _ = page_check.load_url(EA_TOKEN_URL);
                }
                _ => {}
            }
            return glib::ControlFlow::Continue;
        }
        if page_check.url().is_ok_and(|url| url.contains("/id/api/redirect")) {
            let tx = tx_page.clone();
            let _ = page_check.evaluate_script_with_callback("document.body ? document.body.innerText : ''", move |text| {
                if let Some(code) = code_from_page_text(&text) {
                    let _ = tx.send(code);
                }
            });
        }
        glib::ControlFlow::Continue
    });

    // Hand the code over on the main thread and finish.
    let started = std::time::Instant::now();
    let session_view = webview.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Ok(code) = rx.try_recv() {
            if store == Store::Ea {
                // EA's token came: the session is the window's cookies.
                let cookies = match session_view.cookies_for_url(EA_COOKIES_URL) {
                    Ok(c) if !c.is_empty() => c,
                    other => {
                        eprintln!("no EA cookies after the sign-in ({:?})", other.err());
                        std::process::exit(1);
                    }
                };
                println!("{}", ea_cookies_json(cookies.iter().map(|c| (c.name(), c.value()))));
                use std::io::Write;
                let _ = std::io::stdout().flush();
                gtk::main_quit();
                return glib::ControlFlow::Break;
            }
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
    drop(popups);
}

/// Opens the popup a page asked for in a window over the login window, with
/// a view related to the page's (same session, and `window.opener` works).
/// It closes when its page closes itself; a code it navigates to counts too.
#[cfg(target_os = "linux")]
fn open_popup(
    store: Store,
    parent: &gtk::Window,
    opener: webkit2gtk::WebView,
    tx: std::sync::mpsc::Sender<String>,
) -> Option<(gtk::Window, wry::WebView)> {
    use gtk::prelude::*;
    use webkit2gtk::WebViewExt as _;
    use wry::{WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

    let popup = gtk::Window::new(gtk::WindowType::Toplevel);
    popup.set_title(store.title());
    popup.set_transient_for(Some(parent));
    popup.set_default_size(480, 640);
    let built = WebViewBuilder::new()
        .with_related_view(opener)
        .with_user_agent(store.user_agent())
        .with_navigation_handler(move |url| match store.code_from_url(&url) {
            Some(code) => {
                let _ = tx.send(code);
                false
            }
            None => true,
        })
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .build_gtk(&popup);
    let view = match built {
        Ok(view) => view,
        Err(e) => {
            eprintln!("could not open a popup window: {e}");
            return None;
        }
    };
    let closing = popup.clone();
    view.webview().connect_close(move |_| closing.close());
    popup.show_all();
    Some((popup, view))
}

#[cfg(windows)]
fn main() {
    windows::main()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn main() {
    eprintln!("The login window is only available on Linux and Windows for now; use the browser login.");
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
    fn the_xbox_sign_in_hands_back_its_whole_end_address() {
        let xbox = Store::Xbox;
        let end = "https://login.live.com/oauth20_desktop.srf?code=M.C5_BAY.2.abc&state=st";
        assert_eq!(xbox.code_from_url(end).as_deref(), Some(end));
        let denied = "https://login.live.com/oauth20_desktop.srf?error=access_denied&state=st";
        assert_eq!(xbox.code_from_url(denied).as_deref(), Some(denied));
        assert_eq!(xbox.code_from_url("https://login.live.com/oauth20_desktop.srf?lc=1033"), None, "the blank page before");
        assert_eq!(xbox.code_from_url("https://login.live.com/oauth20_authorize.srf?code=x"), None);
        assert_eq!(xbox.code_from_url("https://evil.example/oauth20_desktop.srf?code=x"), None);
    }

    #[test]
    fn arguments_pick_the_store_and_page() {
        assert_eq!(parse_args(&[]).unwrap(), (Store::Epic, EPIC_LOGIN_URL.to_string()));
        assert_eq!(parse_args(&args(&["--smoke"])).unwrap().0, Store::Epic);
        assert_eq!(parse_args(&args(&["--store", "gog"])).unwrap(), (Store::Gog, GOG_LOGIN_URL.to_string()));
        assert_eq!(parse_args(&args(&["--store", "amazon", "--url", "https://amazon.com/ap/signin?x"])).unwrap().0, Store::Amazon);
        assert!(parse_args(&args(&["--store", "amazon"])).is_err());
        assert!(parse_args(&args(&["--store", "xbox"])).is_err());
        assert_eq!(
            parse_args(&args(&["--store", "xbox", "--url", "https://login.live.com/oauth20_authorize.srf?x"])).unwrap().0,
            Store::Xbox
        );
        assert_eq!(parse_args(&args(&["--store", "ea"])).unwrap(), (Store::Ea, EA_LOGIN_URL.to_string()));
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

    #[test]
    fn the_ea_sign_in_ends_on_the_token_of_its_own_address() {
        assert_eq!(ea_page(EA_TOKEN_URL), EaPage::Token);
        assert_eq!(ea_page("https://accounts.ea.com/connect/auth?fid=abc"), EaPage::Token);
        assert_eq!(ea_page("https://signin.ea.com/p/juno/login?fid=abc"), EaPage::Other);
        // EA's site signs the user in (#417): its sign-in pages are left alone.
        assert_eq!(ea_page(EA_LOGIN_URL), EaPage::Other);
        assert_eq!(ea_page("https://www.ea.com/login_check?code=abc&state=x"), EaPage::Other);
        assert_eq!(ea_page("https://www.ea.com/pt-br/login"), EaPage::Other);
        assert_eq!(ea_page("https://ea.com/login_check?code=abc"), EaPage::Other);
        let site_client = "https://accounts.ea.com/connect/auth?client_id=EADOTCOM-WEB-SERVER&response_type=code";
        assert_eq!(ea_page(site_client), EaPage::Other, "the site's own client");
        assert_eq!(ea_page("https://www.ea.com/news/login-rewards"), EaPage::Elsewhere);
        assert_eq!(ea_page("https://www.ea.com/"), EaPage::Elsewhere, "where EA's sign-in ends");
        assert_eq!(ea_page("https://accounts.ea.com/connect/logout"), EaPage::Other);
        assert_eq!(ea_page("https://www.ea.com/pt-br"), EaPage::Elsewhere);
        assert_eq!(ea_page("https://myaccount.ea.com/cp-ui/aboutme/index"), EaPage::Elsewhere);
        assert_eq!(ea_page("https://evilea.com/connect/auth"), EaPage::Other);
        assert_eq!(ea_page("http://accounts.ea.com/connect/auth"), EaPage::Other);
        assert_eq!(ea_page("https://accounts.google.com/o/oauth2"), EaPage::Other);

        let token = r#"{"access_token":"QVQx","token_type":"Bearer","expires_in":"3599"}"#;
        assert!(ea_token_in(token));
        assert!(ea_token_in(&serde_json::to_string(token).unwrap()), "as eval hands it back");
        assert!(ea_token_in(&format!("Pretty-print\n{token}\n")), "inside a JSON viewer's text");
        assert!(!ea_token_in(r#"{"error_code":"login_required","error":"login_required"}"#));
        assert!(!ea_token_in(r#"{"access_token":""}"#));
        assert!(!ea_token_in("<html>Sign in</html>"));
        assert!(EA_TOKEN_URL.contains("client_id=ORIGIN_JS_SDK") && EA_TOKEN_URL.ends_with("&prompt=none"), "no sign-in page of its own");
        assert!(EA_READ_SCRIPT.contains(&format!("location.replace('{EA_LOGIN_URL}')")), "signed out: back to EA's sign-in");

        assert_eq!(Store::Ea.code_from_url("nucleus:rest#access_token=QVQx&expires_in=3599").as_deref(), Some(EA_DONE));
        assert_eq!(Store::Ea.code_from_url("https://www.ea.com/?code=x"), None);
        assert_eq!(
            ea_cookies_json([("sid", "S1"), ("remid", "R\"1")]),
            r#"{"cookies":[{"name":"sid","value":"S1"},{"name":"remid","value":"R\"1"}]}"#
        );
    }
}
