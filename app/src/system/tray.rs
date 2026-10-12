//! System tray icon (StatusNotifierItem over D-Bus, via `ksni`), the
//! counterpart of Heroic's `tray_icon.ts`: recent games, Show / Hide the
//! window, Quit. Runs on its own thread; every action is sent to the UI as a
//! [`TrayCmd`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use heronite_data::DataPaths;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};

use heronite_i18n::tr;

use super::settings::recent_games;

const ICON_DARK: &[u8] = include_bytes!("../../assets/tray/icon-dark.png");
const ICON_LIGHT: &[u8] = include_bytes!("../../assets/tray/icon-light.png");

/// What the user asked for in the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCmd {
    /// Bring the window up (create it when it was closed to the tray).
    Show,
    /// Left click: hide when `exitToTray` and visible, else show.
    Toggle,
    Quit,
    /// Launch a recent game (`appName`; the runner is searched).
    Launch(String),
}

/// One row of the menu, before it becomes a `ksni` item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Recent { app_name: String, title: String },
    Separator,
    ShowHide { window_open: bool },
    Quit,
}

/// Heroic's order: recent games, separator, Show, Quit. The label says what
/// the click does ("Hide" only makes sense when closing the window keeps the
/// app alive).
pub fn menu_rows(recent: &[(String, String)], window_open: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = recent.iter().map(|(a, t)| Row::Recent { app_name: a.clone(), title: t.clone() }).collect();
    if !rows.is_empty() {
        rows.push(Row::Separator);
    }
    rows.push(Row::ShowHide { window_open });
    rows.push(Row::Quit);
    rows
}

/// `_` starts a mnemonic in `ksni` labels; `__` is a literal underscore.
fn escape_label(s: &str) -> String {
    s.replace('_', "__")
}

/// ARGB32 (network byte order) pixmaps of the tray icon at 32 and 64 px.
pub fn icon_pixmaps(dark: bool) -> Vec<ksni::Icon> {
    let bytes = if dark { ICON_DARK } else { ICON_LIGHT };
    let Ok(img) = image::load_from_memory(bytes) else { return Vec::new() };
    let rgba = img.to_rgba8();
    [32u32, 64]
        .into_iter()
        .map(|size| {
            let scaled = if rgba.width() == size {
                rgba.clone()
            } else {
                image::imageops::resize(&rgba, size, size, image::imageops::FilterType::Lanczos3)
            };
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
        "com.heronite.rs".into()
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
        self.send(if self.exit_to_tray { TrayCmd::Toggle } else { TrayCmd::Show });
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let open = self.window_open.load(Ordering::Relaxed);
        let recent = recent_games(&self.data_paths, self.max_recent);
        menu_rows(&recent, open)
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
                    let hide = window_open && self.exit_to_tray;
                    StandardItem {
                        label: if hide { tr!("Hide") } else { tr!("Show") }.into(),
                        activate: Box::new(move |t: &mut Self| t.send(if hide { TrayCmd::Toggle } else { TrayCmd::Show })),
                        ..Default::default()
                    }
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
    match tray.spawn() {
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
    fn rows_follow_heroic_order() {
        let recent = vec![("a".to_string(), "A".to_string()), ("b".to_string(), "B".to_string())];
        assert_eq!(
            menu_rows(&recent, false),
            vec![
                Row::Recent { app_name: "a".into(), title: "A".into() },
                Row::Recent { app_name: "b".into(), title: "B".into() },
                Row::Separator,
                Row::ShowHide { window_open: false },
                Row::Quit
            ]
        );
        // No recent games: no separator at the top.
        assert_eq!(menu_rows(&[], true), vec![Row::ShowHide { window_open: true }, Row::Quit]);
    }

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
