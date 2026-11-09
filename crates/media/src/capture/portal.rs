//! XDG desktop portals through `ashpd`: GlobalShortcuts and Screenshot.
//! Only the pure parts are unit-tested; `live_portal` talks to the session's
//! portal and runs on request (`cargo test -p heronite-media -- --ignored`).
//!
//! # The app id
//! xdg-desktop-portal 1.21 and later refuse a GlobalShortcuts session from an
//! app they cannot name ("An app id is required"). An app outside a sandbox
//! is named by its systemd unit (`app-<id>-*.scope`, which GNOME and KDE put
//! the apps of their menus in, and little else does) or by
//! `Registry.Register`, which must be the first portal call of its D-Bus
//! connection. ashpd's shared connection has long been used by then (GPUI
//! reads the Settings portal at startup), so the portals here have a
//! connection of their own, registered when it is made.

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::screenshot::Screenshot;
use ashpd::zbus;
use futures_util::future::{select, Either};
use futures_util::StreamExt;
use heronite_i18n::tr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Id of the screenshot shortcut in its portal session.
pub const SHORTCUT_ID: &str = "game-screenshot";

/// The connection every portal call here goes through.
static CONNECTION: Mutex<Option<zbus::Connection>> = Mutex::new(None);

/// The portals' connection, made on first use and registered as `app_id`
/// (the first caller's; every caller passes the same one). Blocking. A
/// failed registration is logged and the connection still used: the desktop
/// may know the app by its systemd unit, and portals older than the registry
/// ask for no app id.
fn connection(app_id: Option<&str>) -> Result<zbus::Connection, String> {
    let mut shared = CONNECTION.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(connection) = shared.as_ref() {
        return Ok(connection.clone());
    }
    let connection = pollster::block_on(async {
        let connection = zbus::Connection::session().await.map_err(|e| e.to_string())?;
        if let Some(id) = app_id {
            match id.parse::<ashpd::AppID>() {
                Ok(parsed) => match ashpd::register_host_app_with_connection(connection.clone(), parsed).await {
                    Ok(()) => log::info!("registered with the desktop portal as {id}"),
                    Err(e) => log::warn!("the desktop portal did not register the app as {id}: {e}"),
                },
                Err(e) => log::warn!("invalid portal app id {id:?}: {e}"),
            }
        }
        Ok::<_, String>(connection)
    })?;
    *shared = Some(connection.clone());
    Ok(connection)
}

/// What to tell the user when the portal refuses a shortcut.
fn shortcut_error(error: &ashpd::Error) -> String {
    use ashpd::desktop::ResponseError;
    use ashpd::PortalError;
    match error {
        ashpd::Error::PortalNotFound(_) => tr!("This desktop has no global shortcuts service").into(),
        ashpd::Error::Portal(PortalError::NotAllowed(reason)) if is_missing_app_id(reason) => {
            tr!("The desktop refused the shortcut: it could not tell that Heronite asked for it").into()
        }
        ashpd::Error::Response(ResponseError::Cancelled) => tr!("The shortcut was not approved in the desktop's dialog").into(),
        other => other.to_string(),
    }
}

/// The portal's "An app id is required".
fn is_missing_app_id(reason: &str) -> bool {
    reason.to_ascii_lowercase().contains("app id")
}

/// A live GlobalShortcuts session on its own thread.
pub struct PortalShortcut {
    stop: Option<futures_channel::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PortalShortcut {
    /// Binds `trigger` as the shortcut `id` and calls `on_activated` for
    /// each press. `on_ready` receives the registration result exactly once.
    pub fn start(
        app_id: Option<String>,
        (id, description): (&'static str, &'static str),
        trigger: String,
        on_activated: Arc<dyn Fn() + Send + Sync>,
        on_ready: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) -> Self {
        let (stop_tx, stop_rx) = futures_channel::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("heronite-shortcut-portal".into())
            .spawn(move || match connection(app_id.as_deref()) {
                Ok(connection) => pollster::block_on(run(connection, id, description, trigger, on_activated, on_ready, stop_rx)),
                Err(e) => on_ready(Err(e)),
            })
            .ok();
        Self { stop: Some(stop_tx), thread }
    }

    /// Closes the portal session and joins the thread.
    pub fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PortalShortcut {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run(
    connection: zbus::Connection,
    id: &'static str,
    description: &'static str,
    trigger: String,
    on_activated: Arc<dyn Fn() + Send + Sync>,
    on_ready: Box<dyn FnOnce(Result<(), String>) + Send>,
    mut stop: futures_channel::oneshot::Receiver<()>,
) {
    let setup = async {
        let shortcuts = GlobalShortcuts::with_connection(connection).await?;
        let session = shortcuts.create_session(Default::default()).await?;
        let activated = shortcuts.receive_activated().await?;
        let request = shortcuts
            .bind_shortcuts(
                &session,
                &[NewShortcut::new(id, description).preferred_trigger(trigger.as_str())],
                None,
                BindShortcutsOptions::default(),
            )
            .await?;
        request.response()?;
        Ok::<_, ashpd::Error>((shortcuts, session, activated))
    };
    let (shortcuts, session, mut activated) = match setup.await {
        Ok(v) => {
            on_ready(Ok(()));
            v
        }
        Err(e) => {
            log::warn!("the desktop portal refused the shortcut {id}: {e}");
            on_ready(Err(shortcut_error(&e)));
            return;
        }
    };
    while let Either::Left((Some(a), _)) = select(activated.next(), &mut stop).await {
        if a.shortcut_id() == id {
            on_activated();
        }
    }
    let _ = session.close().await;
    drop(shortcuts);
}

/// `org.freedesktop.portal.Screenshot` (non-interactive): path of the PNG the
/// portal wrote. `app_id`: see [`connection`] (the desktop keeps the
/// permission to capture under it).
pub fn screenshot(app_id: Option<&str>) -> Result<PathBuf, String> {
    let connection = connection(app_id)?;
    pollster::block_on(async {
        let response = Screenshot::request()
            .interactive(false)
            .connection(Some(connection))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|e| e.to_string())?;
        file_uri_to_path(response.uri().as_str()).ok_or_else(|| format!("unsupported screenshot uri {}", response.uri()))
    })
}

/// `file:///a%20b.png` -> `/a b.png`.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        if let Some(b) = decoded {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_uris() {
        assert_eq!(file_uri_to_path("file:///tmp/a%20b.png"), Some(PathBuf::from("/tmp/a b.png")));
        assert_eq!(file_uri_to_path("file://localhost/tmp/x.png"), Some(PathBuf::from("/tmp/x.png")));
        assert_eq!(file_uri_to_path("file:///tmp/%E2%9C%93.png"), Some(PathBuf::from("/tmp/✓.png")));
        assert_eq!(file_uri_to_path("file:///tmp/100%"), Some(PathBuf::from("/tmp/100%")));
        assert!(file_uri_to_path("https://x/y.png").is_none());
    }

    /// The whole way through the session's portal: a connection registered
    /// as Heronite gets a GlobalShortcuts session and its shortcut bound.
    /// Needs xdg-desktop-portal with a GlobalShortcuts backend and
    /// `app.heronite.Heronite.desktop` installed; a desktop that asks the
    /// user to approve shortcuts shows its dialog.
    #[test]
    #[ignore = "talks to the desktop's portal"]
    fn live_portal_binds_a_shortcut() {
        // ashpd's shared connection is in use before any shortcut is wanted,
        // as it is in the app (GPUI reads the Settings portal at startup):
        // registering on it afterwards is refused.
        let shared = pollster::block_on(async { GlobalShortcuts::new().await?.create_session(Default::default()).await.map(|_| ()) });
        eprintln!("a session on ashpd's shared connection: {shared:?}");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut shortcut = PortalShortcut::start(
            Some("app.heronite.Heronite".into()),
            ("heronite-live-test", "Heronite portal test"),
            "CTRL+ALT+SHIFT+F10".into(),
            Arc::new(|| {}),
            Box::new(move |result| {
                let _ = tx.send(result);
            }),
        );
        let result = rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the portal answers");
        shortcut.stop();
        result.expect("the shortcut is bound");
    }

    #[test]
    fn portal_refusals_are_explained() {
        use ashpd::desktop::ResponseError;
        use ashpd::PortalError;
        let missing = ashpd::Error::Portal(PortalError::NotAllowed("An app id is required".into()));
        assert_eq!(shortcut_error(&missing), "The desktop refused the shortcut: it could not tell that Heronite asked for it");
        // Another refusal keeps the portal's own words.
        let other = ashpd::Error::Portal(PortalError::NotAllowed("Not on this seat".into()));
        assert!(shortcut_error(&other).contains("Not on this seat"), "{}", shortcut_error(&other));
        assert_eq!(
            shortcut_error(&ashpd::Error::Response(ResponseError::Cancelled)),
            "The shortcut was not approved in the desktop's dialog"
        );
        let name = zbus::names::OwnedInterfaceName::try_from("org.freedesktop.portal.GlobalShortcuts").unwrap();
        assert_eq!(shortcut_error(&ashpd::Error::PortalNotFound(name)), "This desktop has no global shortcuts service");
    }
}
