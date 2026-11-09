//! The tray icon on Linux: a StatusNotifierItem over D-Bus, via `ksni`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use heronite_data::DataPaths;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};

use heronite_i18n::tr;

use super::common::{click_cmd, current_rows, icon_images, show_hide, Row};
use super::TrayCmd;

/// `_` starts a mnemonic in `ksni` labels; `__` is a literal underscore.
fn escape_label(s: &str) -> String {
    s.replace('_', "__")
}

/// ARGB32 (network byte order) pixmaps of the tray icon at 32 and 64 px.
pub fn icon_pixmaps(dark: bool) -> Vec<ksni::Icon> {
    icon_images(dark, &[32, 64])
        .into_iter()
        .map(|scaled| {
            let size = scaled.width();
            let mut data = Vec::with_capacity((size * size * 4) as usize);
            for p in scaled.pixels() {
                data.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
            }
            ksni::Icon { width: size as i32, height: size as i32, data }
        })
        .collect()
}

pub struct HeroniteTray {
    data_paths: DataPaths,
    dark: bool,
    exit_to_tray: bool,
    max_recent: usize,
    window_open: Arc<AtomicBool>,
    tx: UnboundedSender<TrayCmd>,
}

impl HeroniteTray {
    fn send(&self, cmd: TrayCmd) {
        let _ = self.tx.unbounded_send(cmd);
    }
}

impl ksni::Tray for HeroniteTray {
    fn id(&self) -> String {
        "app.heronite.Heronite".into()
    }

    fn title(&self) -> String {
        "Heronite".into()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        icon_pixmaps(self.dark)
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(click_cmd(self.exit_to_tray));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let open = self.window_open.load(Ordering::Relaxed);
        current_rows(&self.data_paths, self.max_recent, open)
            .into_iter()
            .map(|row| match row {
                Row::Recent { app_name, title } => StandardItem {
                    label: escape_label(&title),
                    activate: Box::new(move |t: &mut Self| t.send(TrayCmd::Launch(app_name.clone()))),
                    ..Default::default()
                }
                .into(),
                Row::Separator => MenuItem::Separator,
                Row::ShowHide { window_open } => {
                    let (label, cmd) = show_hide(window_open, self.exit_to_tray);
                    StandardItem { label: label.into(), activate: Box::new(move |t: &mut Self| t.send(cmd.clone())), ..Default::default() }
                        .into()
                }
                Row::Quit => StandardItem {
                    label: tr!("Quit").into(),
                    icon_name: "application-exit".into(),
                    activate: Box::new(|t: &mut Self| t.send(TrayCmd::Quit)),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}

/// Running tray. Dropping it does not remove the icon: call [`shutdown`](Self::shutdown).
pub struct TrayHandle {
    handle: Handle<HeroniteTray>,
    window_open: Arc<AtomicBool>,
}

impl TrayHandle {
    /// The window was opened or closed: the Show/Hide row changes.
    pub fn set_window_open(&self, open: bool) {
        self.window_open.store(open, Ordering::Relaxed);
        self.refresh();
    }

    /// Re-reads the menu (recent games changed). Off the UI thread: the
    /// handle's calls wait for the tray service.
    pub fn refresh(&self) {
        let handle = self.handle.clone();
        std::thread::spawn(move || {
            handle.update(|_| {});
        });
    }

    pub fn shutdown(&self) {
        let handle = self.handle.clone();
        // Waiting would block the UI thread while the service winds down.
        std::thread::spawn(move || handle.shutdown().wait());
    }
}

/// Starts the tray. `None` when there is no StatusNotifier host (no bar with
/// tray support) or D-Bus is unavailable: the caller then behaves as if the
/// tray did not exist, so the app can never end up with no window and no icon.
pub fn spawn(
    data_paths: DataPaths,
    dark: bool,
    exit_to_tray: bool,
    max_recent: usize,
    window_open: bool,
    tx: UnboundedSender<TrayCmd>,
) -> Option<TrayHandle> {
    let window_open = Arc::new(AtomicBool::new(window_open));
    let tray = HeroniteTray { data_paths, dark, exit_to_tray, max_recent, window_open: window_open.clone(), tx };
    // A Flatpak may not own the `StatusNotifierItem-<pid>-<n>` name.
    match tray.disable_dbus_name(heronite_platform::flatpak::active()).spawn() {
        Ok(handle) => Some(TrayHandle { handle, window_open }),
        Err(e) => {
            log::warn!("no tray icon: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_escape_mnemonics() {
        assert_eq!(escape_label("Half_Life"), "Half__Life");
        assert_eq!(escape_label("Plain"), "Plain");
    }

    #[test]
    fn pixmaps_are_argb32() {
        for dark in [true, false] {
            let icons = icon_pixmaps(dark);
            assert_eq!(icons.len(), 2);
            for i in &icons {
                assert_eq!(i.data.len(), (i.width * i.height * 4) as usize);
            }
            // Something is drawn (not fully transparent).
            assert!(icons[1].data.chunks(4).any(|p| p[0] > 0));
        }
    }
}
