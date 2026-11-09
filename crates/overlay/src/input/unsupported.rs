//! Systems without evdev: no controllers are read (`Feature::Overlay` is off
//! there).

use std::io;

use super::{Controller, Event, Options};

/// Never runs: [`Watcher::start`] fails.
pub struct Watcher {
    _private: (),
}

impl Watcher {
    pub fn start(_opts: Options, _on_event: impl FnMut(Event) + Send + 'static) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "controllers are read through evdev, which this system lacks"))
    }

    pub fn close(&self) {}

    pub fn open(&self) {}

    pub fn set_options(&self, _opts: Options) {}
}

pub fn list_controllers() -> Vec<Controller> {
    Vec::new()
}
