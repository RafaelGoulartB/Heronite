//! Epic Games login window. Opens Epic's login page in an embedded WebKit
//! view (the same flow as Heroic's webview), waits for the authorization
//! code Epic hands back and prints it to stdout, then exits with status 0.
//! Exits with 1 when the window is closed first.
//!
//! The main app spawns this helper and reads stdout; it lives in its own
//! binary so the GPUI app does not carry a web engine.
//!
//! `--smoke` loads a local page instead (never touches the network): prints
//! `SMOKE-READY` once it is loaded and stays open for a few seconds, for
//! checking that the window opens where expected.

use url::Url;

const LOGIN_URL: &str = "https://www.epicgames.com/id/login?responseType=code";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) EpicGamesLauncher";

/// The authorization code in a URL Epic redirects to
/// (`https://localhost/launcher/authorized?code=...`).
fn code_from_url(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if url.host_str() != Some("localhost") {
        return None;
    }
    url.query_pairs().find(|(k, _)| k == "code").map(|(_, v)| v.into_owned()).filter(|c| !c.is_empty())
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
        .or_else(|| doc.get("redirectUrl").and_then(|u| u.as_str()).and_then(code_from_url))
}

#[cfg(target_os = "linux")]
fn main() {
    use std::sync::mpsc;
    use std::time::Duration;

    use gtk::glib;
    use gtk::prelude::*;
    use wry::{PageLoadEvent, WebViewBuilder, WebViewBuilderExtUnix};

    let smoke = std::env::args().any(|a| a == "--smoke");
    glib::set_prgname(Some("heronite-epic-login"));
    glib::set_application_name("Epic Games login");
    if gtk::init().is_err() {
        eprintln!("could not initialise GTK (no display?)");
        std::process::exit(2);
    }

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Epic Games login");
    window.set_default_size(520, 720);
    window.connect_delete_event(|_, _| {
        // Closed before a code arrived.
        std::process::exit(1);
    });

    let (tx, rx) = mpsc::channel::<String>();
    let (tx_nav, tx_load) = (tx.clone(), tx.clone());
    let mut builder = WebViewBuilder::new()
        .with_incognito(true)
        .with_user_agent(USER_AGENT)
        // Epic redirects to https://localhost/...?code=: take the code and do not load it.
        .with_navigation_handler(move |url| match code_from_url(&url) {
            Some(code) => {
                let _ = tx_nav.send(code);
                false
            }
            None => true,
        });
    builder = if smoke {
        builder.with_html("<html><body style='font-family:sans-serif;background:#222;color:#eee'><h2>Epic login helper</h2><p>Smoke test page.</p></body></html>")
    } else {
        builder.with_url(LOGIN_URL)
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
                if let Some(code) = code_from_url(&url) {
                    let _ = tx_load.send(code);
                }
            })
            .build_gtk(&window)
            .expect("creating the web view"),
    );

    // The JSON page of /id/api/redirect shows the code as text.
    let page_check = webview.clone();
    let tx_page = tx.clone();
    let mut last_url = String::new();
    glib::timeout_add_local(Duration::from_millis(700), move || {
        if smoke {
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
    eprintln!("The Epic login window is only available on Linux for now; use the browser login.");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_found_in_redirects() {
        assert_eq!(code_from_url("https://localhost/launcher/authorized?code=abc123").as_deref(), Some("abc123"));
        assert_eq!(code_from_url("http://localhost:3000/?state=x&code=zzz").as_deref(), Some("zzz"));
        assert_eq!(code_from_url("https://www.epicgames.com/id/login?code=nope"), None);
        assert_eq!(code_from_url("https://localhost/launcher/authorized"), None);
        assert_eq!(code_from_url("not a url"), None);
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
