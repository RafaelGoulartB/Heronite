//! Settings tab: Backup. Reference: `docs/reference/dialogs/settings-backup.png`.
//! Placeholder until the backend crate lands; replace `State` and `render`.

use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;

use super::SettingsDialog;
use crate::dialogs::DialogCtx;

pub struct State {}

impl State {
    pub fn new(_ctx: &DialogCtx, _settings: &EffectiveSettings, _window: &mut Window, _cx: &mut Context<SettingsDialog>) -> Self {
        Self {}
    }
}

pub fn render(_d: &mut SettingsDialog, _window: &mut Window, _cx: &mut Context<SettingsDialog>) -> AnyElement {
    super::pending("Backup settings")
}
