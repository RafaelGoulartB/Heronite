//! The console options that act while console mode is off: how the app
//! opens and what a controller connecting does. They are kept in
//! `console.json` and mirrored where the app reads them at start, so a user
//! who never turns these on pays for no file read and no watcher.

use heronite_data::console_theme::{AutoConsole, OpenIn};
use heronite_data::paths::AppPaths;

pub fn set_open_in(app: &AppPaths, v: OpenIn) {
    super::themes::update(app, |p| p.open_in = v);
}

pub fn set_auto_console(app: &AppPaths, v: AutoConsole) {
    super::themes::update(app, |p| p.auto_console = v);
}
