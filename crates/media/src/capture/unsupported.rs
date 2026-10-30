//! Systems without a capture backend (`Feature::Capture` is off there): the
//! service never captures and a shortcut never registers.

use std::sync::mpsc::Sender;
use std::sync::Arc;

use heronite_i18n::tr;

use super::{ActiveGame, CaptureConfig, CaptureEvent, CaptureOutcome, CaptureStatus, HotkeySpec, ServiceOptions, ShortcutBackend};

/// Captures nothing; [`CaptureService::status`] says so.
#[derive(Clone)]
pub struct CaptureService {
    _private: (),
}

impl CaptureService {
    pub fn new(_events: Sender<CaptureEvent>, _options: ServiceOptions) -> Self {
        Self { _private: () }
    }

    pub fn set_config(&self, _config: CaptureConfig) {}

    pub fn begin_session(&self, _game: ActiveGame) {}

    pub fn end_session(&self, _runner: &str, _app_name: &str) {}

    pub fn shutdown(&self) {}

    pub fn status(&self) -> CaptureStatus {
        CaptureStatus { registered: false, accelerator: String::new(), active_game: None, error: Some(unavailable()) }
    }

    pub fn capture_now(&self) -> CaptureOutcome {
        CaptureOutcome::Failed(unavailable())
    }

    pub fn capture_quiet(&self) -> CaptureOutcome {
        CaptureOutcome::Failed(unavailable())
    }
}

fn unavailable() -> String {
    tr!("Screenshots are not available").to_string()
}

/// Never registered: `on_ready` hears so at once.
pub struct GlobalHotkey {
    _private: (),
}

type Ready = Box<dyn FnOnce(Result<ShortcutBackend, String>) + Send>;

impl GlobalHotkey {
    pub fn start(spec: HotkeySpec, _on_press: Arc<dyn Fn() + Send + Sync>, on_ready: Ready) -> Self {
        on_ready(Err(tr!("The shortcut {accelerator} is unavailable", accelerator = &spec.accelerator)));
        Self { _private: () }
    }
}
