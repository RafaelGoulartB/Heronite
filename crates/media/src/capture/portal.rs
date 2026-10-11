//! XDG desktop portals through `ashpd`: GlobalShortcuts and Screenshot.
//! Not exercised by the tests (they need a session bus and a compositor).

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::screenshot::Screenshot;
use futures_util::future::{select, Either};
use futures_util::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;

const SHORTCUT_ID: &str = "game-screenshot";

/// A live GlobalShortcuts session on its own thread.
pub struct PortalShortcut {
    stop: Option<futures_channel::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PortalShortcut {
    /// Binds `trigger` and calls `on_activated` for each press.
    /// `on_ready` receives the registration result exactly once.
    pub fn start(
        app_id: Option<String>,
        trigger: String,
        on_activated: Arc<dyn Fn() + Send + Sync>,
        on_ready: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) -> Self {
        let (stop_tx, stop_rx) = futures_channel::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("heronite-shortcut-portal".into())
            .spawn(move || pollster::block_on(run(app_id, trigger, on_activated, on_ready, stop_rx)))
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
    app_id: Option<String>,
    trigger: String,
    on_activated: Arc<dyn Fn() + Send + Sync>,
    on_ready: Box<dyn FnOnce(Result<(), String>) + Send>,
    mut stop: futures_channel::oneshot::Receiver<()>,
) {
    if let Some(id) = app_id {
        // Non-sandboxed apps must tell the portal who they are.
        match id.parse::<ashpd::AppID>() {
            Ok(id) => {
                if let Err(e) = ashpd::register_host_app(id).await {
                    log::warn!("portal host app registration failed: {e}");
                }
            }
            Err(e) => log::warn!("invalid portal app id {id:?}: {e}"),
        }
    }
    let setup = async {
        let shortcuts = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
        let session = shortcuts.create_session(Default::default()).await.map_err(|e| e.to_string())?;
        let activated = shortcuts.receive_activated().await.map_err(|e| e.to_string())?;
        let request = shortcuts
            .bind_shortcuts(
                &session,
                &[NewShortcut::new(SHORTCUT_ID, super::BIND_DESCRIPTION).preferred_trigger(trigger.as_str())],
                None,
                BindShortcutsOptions::default(),
            )
            .await
            .map_err(|e| e.to_string())?;
        request.response().map_err(|e| e.to_string())?;
        Ok::<_, String>((shortcuts, session, activated))
    };
    let (shortcuts, session, mut activated) = match setup.await {
        Ok(v) => {
            on_ready(Ok(()));
            v
        }
        Err(e) => {
            on_ready(Err(e));
            return;
        }
    };
    while let Either::Left((Some(a), _)) = select(activated.next(), &mut stop).await {
        if a.shortcut_id() == SHORTCUT_ID {
            on_activated();
        }
    }
    let _ = session.close().await;
    drop(shortcuts);
}

/// `org.freedesktop.portal.Screenshot` (non-interactive): path of the PNG the
/// portal wrote.
pub fn screenshot() -> Result<PathBuf, String> {
    pollster::block_on(async {
        let response =
            Screenshot::request().interactive(false).send().await.map_err(|e| e.to_string())?.response().map_err(|e| e.to_string())?;
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
}
