//! No tray icon on this system yet (`Feature::Tray`): [`spawn`] finds none,
//! so closing the window quits the app.

use futures::channel::mpsc::UnboundedSender;
use heronite_data::DataPaths;

use super::TrayCmd;

/// Never made: [`spawn`] returns `None`.
pub struct TrayHandle(());

impl TrayHandle {
    pub fn set_window_open(&self, _open: bool) {}
    pub fn refresh(&self) {}
    pub fn shutdown(&self) {}
}

pub fn spawn(
    _data_paths: DataPaths,
    _dark: bool,
    _exit_to_tray: bool,
    _max_recent: usize,
    _window_open: bool,
    _tx: UnboundedSender<TrayCmd>,
) -> Option<TrayHandle> {
    None
}
