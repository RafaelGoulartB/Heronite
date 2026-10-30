//! The tray icon on Windows: an icon in the taskbar's notification area, via
//! `heronite_platform::tray`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_platform::tray::{Icon, MenuItem, Options, Tray};

use super::common::{click_cmd, current_rows, icon_images, show_hide, Row};
use super::TrayCmd;

/// The notification area draws icons at 16 px at 100 % scaling, up to 64 px
/// at 400 %; the tray picks the size that fits.
const ICON_SIZES: [u32; 10] = [16, 20, 24, 28, 32, 36, 40, 48, 56, 64];

fn icons(dark: bool) -> Vec<Icon> {
    icon_images(dark, &ICON_SIZES).into_iter().filter_map(|img| Icon::from_rgba(img.width(), img.height(), img.into_raw())).collect()
}

/// The menu as it is when it opens: the recent games on disk, the label of
/// Show/Hide for the window's state, the interface language.
fn menu(data_paths: &DataPaths, max_recent: usize, exit_to_tray: bool, window_open: bool, tx: &UnboundedSender<TrayCmd>) -> Vec<MenuItem> {
    let send = |cmd: TrayCmd| {
        let tx = tx.clone();
        move || {
            let _ = tx.unbounded_send(cmd);
        }
    };
    current_rows(data_paths, max_recent, window_open)
        .into_iter()
        .map(|row| match row {
            Row::Recent { app_name, title } => MenuItem::entry(title, send(TrayCmd::Launch(app_name))),
            Row::Separator => MenuItem::Separator,
            Row::ShowHide { window_open } => {
                let (label, cmd) = show_hide(window_open, exit_to_tray);
                MenuItem::entry(label, send(cmd))
            }
            Row::Quit => MenuItem::entry(tr!("Quit"), send(TrayCmd::Quit)),
        })
        .collect()
}

/// Running tray. Dropping it removes the icon too.
pub struct TrayHandle {
    tray: Tray,
    window_open: Arc<AtomicBool>,
}

impl TrayHandle {
    /// The window was opened or closed: the Show/Hide row changes.
    pub fn set_window_open(&self, open: bool) {
        self.window_open.store(open, Ordering::Relaxed);
    }

    /// Nothing to do: the menu is built each time it opens.
    pub fn refresh(&self) {}

    /// Removes the icon; waits for that (a second at most), or a dead icon
    /// would stay in the notification area after the process ends.
    pub fn shutdown(&self) {
        self.tray.remove();
    }
}

/// Starts the tray. `None` when there is no notification area (Explorer is
/// not running): the caller then behaves as if the tray did not exist, so the
/// app can never end up with no window and no icon.
pub fn spawn(
    data_paths: DataPaths,
    dark: bool,
    exit_to_tray: bool,
    max_recent: usize,
    window_open: bool,
    tx: UnboundedSender<TrayCmd>,
) -> Option<TrayHandle> {
    let window_open = Arc::new(AtomicBool::new(window_open));
    let open = window_open.clone();
    let click = tx.clone();
    let options = Options {
        tooltip: "Heronite".into(),
        icons: icons(dark),
        on_click: Box::new(move || {
            let _ = click.unbounded_send(click_cmd(exit_to_tray));
        }),
        menu: Box::new(move || menu(&data_paths, max_recent, exit_to_tray, open.load(Ordering::Relaxed), &tx)),
    };
    match Tray::spawn(options) {
        Ok(tray) => Some(TrayHandle { tray, window_open }),
        Err(e) => {
            log::warn!("no tray icon: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::unbounded;

    #[test]
    fn icons_come_at_every_size() {
        assert_eq!(icons(false).len(), ICON_SIZES.len());
    }

    #[test]
    fn menu_rows_send_their_commands() {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("store")).unwrap();
        std::fs::write(data_paths.store_config(), r#"{"games":{"recent":[{"appName":"tj","title":"Tom & Jerry"}]}}"#).unwrap();
        let (tx, mut rx) = unbounded();
        let items = menu(&data_paths, 5, true, true, &tx);
        let labels: Vec<Option<&str>> = items
            .iter()
            .map(|i| match i {
                MenuItem::Entry { label, .. } => Some(label.as_str()),
                MenuItem::Separator => None,
            })
            .collect();
        // Plain text: the platform escapes the `&`.
        assert_eq!(labels, vec![Some("Tom & Jerry"), None, Some("Hide"), Some("Quit")]);
        for item in items {
            if let MenuItem::Entry { action, .. } = item {
                action();
            }
        }
        let sent: Vec<TrayCmd> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(sent, vec![TrayCmd::Launch("tj".into()), TrayCmd::Toggle, TrayCmd::Quit]);
    }
}
