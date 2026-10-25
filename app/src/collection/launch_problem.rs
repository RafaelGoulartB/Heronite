//! A game that closed right after starting: the Collection diagnoses the
//! run (`crate::diagnosis`, off the UI thread) and opens the "Launch
//! problem" dialog when it found something, or offers it in a toast while
//! another dialog is open. "Troubleshoot launch" opens it for the last
//! launch on demand.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use heronite_launch::ExitInfo;

use super::{CollectionView, ToastAction};
use crate::diagnosis::{Ended, Report};
use crate::dialogs::launch_problem::{Actions, LaunchProblemDialog};

/// How long the toast that offers the dialog stays.
const TOAST_SECS: u64 = 12;

impl CollectionView {
    /// `app_name` ended `elapsed` after starting, on its own: tells why
    /// when the analysis knows (`log`: another log to read, debug hooks).
    pub(super) fn diagnose_exit(&mut self, app_name: &str, ended: Ended, log: Option<PathBuf>, cx: &mut Context<Self>) {
        self.diagnose_exit_with(app_name, ended, log, false, cx);
    }

    /// [`Self::diagnose_exit`]; `toast`: offer the dialog in a toast even
    /// when nothing else is open.
    fn diagnose_exit_with(&mut self, app_name: &str, ended: Ended, log: Option<PathBuf>, toast: bool, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.games.iter().find(|g| g.app_name == app_name)).cloned() else { return };
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let g = game.clone();
            let report = cx.background_spawn(async move { crate::diagnosis::gather(&paths, &g, Some(ended), log.as_deref()) }).await;
            this.update(cx, |this, cx| {
                match report {
                    Some(r) if r.worth_telling() => this.show_launch_problem(Arc::new(r), toast, cx),
                    Some(_) => {}
                    // Not one the analysis reads (Steam starts it): the plain toast.
                    None => {
                        if let Some(exit) = ended.exit.filter(|_| ended.failed()) {
                            this.show_failed_launch_toast(&game.title, &game.key, exit, cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Troubleshoot launch": the report of the game's last launch.
    pub(super) fn troubleshoot_launch(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let report = cx.background_spawn(async move { crate::diagnosis::gather(&paths, &game, None, None) }).await;
            this.update(cx, |this, cx| match report {
                Some(r) => this.open_launch_problem(Arc::new(r), cx),
                None => this.show_toast(heronite_i18n::tr!("Steam starts this game: its launch problems show in Steam."), false, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Opens the dialog at once unless another dialog is open (or `toast`),
    /// where a toast offers it instead.
    fn show_launch_problem(&mut self, report: Arc<Report>, toast: bool, cx: &mut Context<Self>) {
        if self.dialog.is_none() && !toast {
            return self.open_launch_problem(report, cx);
        }
        let text = crate::diagnosis::text::summary(&report);
        self.show_toast_for(text, true, Duration::from_secs(TOAST_SECS), cx);
        self.toast_action = Some(ToastAction::LaunchProblem(report));
    }

    pub(super) fn open_launch_problem(&mut self, report: Arc<Report>, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let key = report.key.clone();
        let play = {
            let (weak, key) = (weak.clone(), key.clone());
            Rc::new(move |toast: String, cx: &mut App| {
                weak.update(cx, |this, cx| {
                    this.dialog = None;
                    this.dialog_sub = None;
                    this.show_toast(toast, false, cx);
                    this.primary_action(&key, cx);
                    this.reload(cx);
                    cx.notify();
                })
                .ok();
            })
        };
        let view_log = {
            let (weak, key) = (weak.clone(), key.clone());
            Rc::new(move |cx: &mut App| {
                weak.update(cx, |this, cx| this.open_game_log(&key, cx)).ok();
            })
        };
        let repair = Rc::new(move |cx: &mut App| {
            weak.update(cx, |this, cx| {
                this.dialog = None;
                this.dialog_sub = None;
                this.epic.pending = Some(super::epic::Request::Verify(key.clone()));
                cx.notify();
            })
            .ok();
        });
        let actions = Actions { play, view_log, repair };
        let paths = self.data_paths.clone();
        let view = cx.new(|_| LaunchProblemDialog::new(paths, report, actions));
        if let Some(which) = crate::env::value("HERONITE_DEBUG_LAUNCH_FIX") {
            view.update(cx, |d, _| d.debug_state(&which));
        }
        self.open_dialog(view, cx);
    }

    /// A game that died right after starting, when the analysis cannot tell
    /// why: says so, and offers its log.
    pub(super) fn show_failed_launch_toast(&mut self, title: &str, key: &str, exit: ExitInfo, cx: &mut Context<Self>) {
        self.show_toast(crate::runtime::failed_launch_message(title, exit), true, cx);
        self.toast_action = Some(ToastAction::ViewLog(key.to_string()));
    }

    /// `HERONITE_DEBUG_DIALOG=launch-problem:<log>[@<exit code>]`: the
    /// selected game as if it had just ended 2 s after starting with that
    /// code (53 unless given), its run read from `<log>`; nothing is
    /// launched. `launch-problem-toast:` offers it in the toast instead.
    pub(super) fn debug_launch_problem(&mut self, spec: &str, toast: bool, cx: &mut Context<Self>) {
        let Some(app) = self.selected.as_ref().and_then(|k| self.lib.as_ref()?.get(k)).map(|g| g.app_name.clone()) else { return };
        let (log, code) = match spec.rsplit_once('@') {
            Some((log, code)) if code.parse::<i32>().is_ok() => (log, code.parse().unwrap_or(53)),
            _ => (spec, 53),
        };
        let ended = Ended { elapsed: Duration::from_secs(2), exit: Some(ExitInfo { code: Some(code), signal: None }) };
        self.diagnose_exit_with(&app, ended, Some(PathBuf::from(log)), toast, cx);
    }
}
