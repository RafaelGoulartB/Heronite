//! "Install from script" glue of the Collection: opens the dialog (palette,
//! debug hook; from the ☰ menu it is a step of "Add game") and reports the
//! end of an install that ran while the dialog was closed. The work is in `services::script_install`.

use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{CollectionView, ToastAction};
use crate::dialogs::script_install::ScriptInstallDialog;
use crate::services::script_install::ScriptInstallEvent;

impl CollectionView {
    /// Opens the dialog: on `file`, on the install that is running, or on
    /// the file chooser.
    pub(super) fn open_install_script(&mut self, file: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        self.open_install_script_view(file, window, cx);
    }

    fn open_install_script_view(
        &mut self,
        file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ScriptInstallDialog>> {
        self.open_menu = None;
        self.context_menu = None;
        let ctx = self.dialog_ctx()?;
        let view = cx.new(|cx| ScriptInstallDialog::new(ctx, file, window, cx));
        self.open_dialog(view.clone(), cx);
        Some(view)
    }

    /// Shows the last install again (the toast's "Details"), even when it
    /// has ended.
    pub(super) fn show_script_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let view = cx.new(|cx| ScriptInstallDialog::showing_last(ctx, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn on_script_install_event(&mut self, ev: &ScriptInstallEvent, cx: &mut Context<Self>) {
        match ev {
            ScriptInstallEvent::Finished { title, error: None, .. } => {
                self.reload(cx);
                self.show_toast(tr!("{title} was installed", title = title.clone()), false, cx);
            }
            ScriptInstallEvent::Finished { title, cancelled: true, .. } => {
                self.show_toast(tr!("The install of {title} was stopped", title = title.clone()), false, cx);
            }
            ScriptInstallEvent::Finished { title, .. } => {
                self.show_toast(tr!("The install of {title} failed", title = title.clone()), true, cx);
                self.toast_action = Some(ToastAction::ScriptInstall);
            }
        }
    }

    /// `HERONITE_DEBUG_DIALOG=install-script[:<file>[:<state>]]` (states are
    /// `+`-joined, see `ScriptInstallDialog::debug_state`).
    pub(super) fn script_install_debug_dialog(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) {
        let rest = which.strip_prefix("install-script").unwrap_or("").trim_start_matches(':');
        let (file, state) = match rest.rsplit_once(':') {
            Some((f, s)) if !f.is_empty() && !s.contains('/') => (f, s),
            _ => (rest, ""),
        };
        let file = (!file.is_empty()).then(|| PathBuf::from(file));
        if let Some(view) = self.open_install_script_view(file, window, cx) {
            if !state.is_empty() {
                view.update(cx, |d, cx| d.debug_state(state, cx));
            }
        }
    }
}
