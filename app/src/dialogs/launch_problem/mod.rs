//! "Launch problem": why a game closed right after starting
//! (`crate::diagnosis`), the log lines that show it, and its fixes. The fix
//! offered first applies with one click and starts the game again; a
//! command to run in a terminal is copied instead.
//!
//! The Collection opens it with the report of a run that just ended, or
//! from "Troubleshoot" with the report of the last session.

mod console;
mod render;

use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_data::DataPaths;
use heronite_launch::diagnose::Fix;

use super::DialogEvent;
use crate::diagnosis::{self, Report};

/// Output lines kept while a fix runs.
const LINES: usize = 4;

/// What the dialog asks of the Collection.
#[derive(Clone)]
pub struct Actions {
    /// Closes the dialog, shows this toast and starts the game.
    pub play: Rc<dyn Fn(String, &mut App)>,
    pub view_log: Rc<dyn Fn(&mut App)>,
    /// The store's verify of the game's files.
    pub repair: Rc<dyn Fn(&mut App)>,
    /// The game runs or is starting ("Troubleshoot" is there meanwhile).
    pub running: Rc<dyn Fn(&App) -> bool>,
}

/// A fix of the report: (finding, fix) indices.
type At = (usize, usize);

enum State {
    Idle,
    Working { at: At, lines: Vec<String>, pid: Arc<AtomicU32>, cancelled: bool },
    Failed { at: At, message: String },
}

enum Msg {
    Line(String),
    Done(Result<(), String>),
}

pub struct LaunchProblemDialog {
    paths: DataPaths,
    report: Arc<Report>,
    actions: Actions,
    state: State,
    /// The fix whose command was copied.
    copied: Option<At>,
    scroll: ScrollHandle,
    console_list: crate::console::list::ListNav,
}

impl EventEmitter<DialogEvent> for LaunchProblemDialog {}

impl LaunchProblemDialog {
    pub fn new(paths: DataPaths, report: Arc<Report>, actions: Actions) -> Self {
        Self { paths, report, actions, state: State::Idle, copied: None, scroll: ScrollHandle::new(), console_list: Default::default() }
    }

    fn fix(&self, at: At) -> Option<&Fix> {
        self.report.findings.get(at.0)?.fixes.get(at.1)
    }

    /// "Launch problem" right after a failed start, "Troubleshoot" when asked.
    fn title(&self) -> &'static str {
        match self.report.ended {
            Some(_) => heronite_i18n::tr!("Launch problem"),
            None => heronite_i18n::tr!("Troubleshoot"),
        }
    }

    fn busy(&self) -> bool {
        matches!(self.state, State::Working { .. })
    }

    /// Runs the fix at `at`: copies a command, asks the store to verify, or
    /// applies the change on a thread and starts the game once it worked.
    fn run_fix(&mut self, at: At, cx: &mut Context<Self>) {
        let Some(fix) = self.fix(at).cloned() else { return };
        if self.busy() {
            return;
        }
        match &fix {
            Fix::Terminal(cmd) => {
                cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()));
                self.copied = Some(at);
                cx.notify();
                return;
            }
            Fix::Repair => {
                diagnosis::remember_tried(&self.report.app_name, &fix);
                (self.actions.repair.clone())(cx);
                return;
            }
            _ => {}
        }
        // A fix changes the prefix or the settings under the game, and
        // starting it again would stop it instead.
        if (self.actions.running)(cx) {
            let message = heronite_i18n::tr!("{title} is running. Close the game to apply this fix.", title = self.report.title);
            self.state = State::Failed { at, message };
            cx.notify();
            return;
        }
        let pid = Arc::new(AtomicU32::new(0));
        self.state = State::Working { at, lines: Vec::new(), pid: pid.clone(), cancelled: false };
        let (tx, mut rx) = unbounded::<Msg>();
        let (paths, report, job) = (self.paths.clone(), self.report.clone(), fix.clone());
        std::thread::Builder::new()
            .name("launch-fix".into())
            .spawn(move || {
                let line = |l: String| {
                    let _ = tx.unbounded_send(Msg::Line(l));
                };
                let res = diagnosis::apply(&paths, &report, &job, &pid, &line);
                let _ = tx.unbounded_send(Msg::Done(res));
            })
            .ok();
        cx.spawn(async move |this, cx| {
            let mut done = false;
            while let Some(msg) = rx.next().await {
                done = matches!(msg, Msg::Done(_));
                let alive = this.update(cx, |this, cx| this.on_msg(msg, cx)).is_ok();
                if done || !alive {
                    break;
                }
            }
            if !done {
                this.update(cx, |this, cx| this.on_msg(Msg::Done(Err(heronite_i18n::tr!("The fix stopped unexpectedly.").into())), cx))
                    .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let State::Working { at, lines, cancelled, .. } = &mut self.state else { return };
        let at = *at;
        match msg {
            Msg::Line(l) => {
                lines.push(l);
                if lines.len() > LINES {
                    lines.remove(0);
                }
            }
            Msg::Done(_) if *cancelled => self.state = State::Idle,
            Msg::Done(Ok(())) => {
                self.state = State::Idle;
                if let Some(fix) = self.fix(at).cloned() {
                    diagnosis::remember_tried(&self.report.app_name, &fix);
                    let toast = diagnosis::text::fixed(&fix, &self.report);
                    (self.actions.play.clone())(toast, cx);
                }
            }
            Msg::Done(Err(message)) => self.state = State::Failed { at, message },
        }
        cx.notify();
    }

    /// Stops winetricks (its process group).
    fn cancel(&mut self, cx: &mut Context<Self>) {
        let State::Working { pid, cancelled, .. } = &mut self.state else { return };
        let pid = pid.load(Ordering::SeqCst);
        if pid == 0 {
            return;
        }
        *cancelled = true;
        // SAFETY: signals the process group winetricks leads (started with
        // `process_group(0)`), so its Wine children stop too.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        // A running fix keeps the dialog: closing would hide its outcome.
        if !self.busy() {
            cx.emit(DialogEvent::Close);
        }
    }

    fn protondb(&self, cx: &mut App) {
        if let Some(id) = &self.report.steam_app_id {
            cx.open_url(&format!("https://www.protondb.com/app/{id}"));
        }
    }

    /// `HERONITE_DEBUG_LAUNCH_FIX=working|failed`: the main fix as if it
    /// were running or had failed (nothing runs).
    pub fn debug_state(&mut self, which: &str) {
        let at = (0, 0);
        self.state = match which {
            "working" => State::Working {
                at,
                lines: vec!["Executing w_do_call vcrun2022".into(), "Executing wine VC_redist.x64.exe /q".into()],
                pid: Arc::new(AtomicU32::new(0)),
                cancelled: false,
            },
            "failed" => State::Failed { at, message: "vcrun2022 could not be installed: winetricks exited with exit status: 1".into() },
            _ => State::Idle,
        };
    }
}

impl Render for LaunchProblemDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        self.render_desktop(window, cx)
    }
}
