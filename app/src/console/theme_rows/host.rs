//! Running [`super`]'s rows against a view: a [`Host`] opens the keyboard,
//! the pickers and sheets in its own way (the Settings page has a layer, the
//! Collection an overlay) and keeps a [`ThemeState`]; the functions here do
//! the rest (change the prefs through `themes::update`, the theme store, the
//! export and import dialogs).

use std::path::{Path, PathBuf};

use gpui_kit::*;
use heronite_data::console_theme::{store, ConsolePrefs, ConsoleTheme};
use heronite_data::paths::AppPaths;
use heronite_i18n::tr;

use super::{activate, export_name, movable, removable, run, Arg, Command, Ctx, Data, Step, StoreOp};
use crate::console::browser::PickKind;
use crate::console::list::{Outcome, Row};
use crate::console::{hint, themes, Btn, Hint, Pad};

/// What a view that shows the theme rows keeps.
pub struct ThemeState {
    /// The prefs as of the last change.
    pub prefs: ConsolePrefs,
    pub data: Data,
    /// The last result worth telling: `(text, is an error)`.
    pub notice: Option<(String, bool)>,
    /// ↑ ↓ move the focused row (after X).
    pub moving: bool,
    /// The theme before each change, for Undo (the latest last), and the
    /// ones undone, for Redo. Cleared when another theme is put in use.
    pub undo: Vec<ConsoleTheme>,
    pub redo: Vec<ConsoleTheme>,
    /// The option changed last: more changes of the same number or colour
    /// (a drag) make one step of Undo.
    pub(super) last_change: Option<String>,
}

/// Steps of Undo kept.
const UNDO_MAX: usize = 100;

impl ThemeState {
    /// Reads `console.json` (once per run); the saved themes come with
    /// [`load_saved`].
    pub fn new(app: &AppPaths) -> Self {
        Self {
            prefs: themes::prefs(app),
            data: Data::default(),
            notice: None,
            moving: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_change: None,
        }
    }

    pub fn ctx(&self) -> Ctx<'_> {
        Ctx { prefs: &self.prefs, data: &self.data }
    }

    pub fn set_error(&mut self, e: impl std::fmt::Display) {
        self.notice = Some((format!("{e:#}"), true));
    }
}

pub trait Host: Sized + 'static {
    fn app(&self) -> AppPaths;
    /// `None` when the view is not showing the options any more (a job of
    /// the theme store may finish after it closed).
    fn theme_state(&mut self) -> Option<&mut ThemeState>;
    fn open_osk(
        &mut self,
        title: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    );
    fn open_browser(
        &mut self,
        title: &str,
        kind: PickKind,
        start: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        picked: impl Fn(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    );
    /// A sheet of action rows; `chosen` gets the id of the one activated.
    fn open_sheet(
        &mut self,
        title: &str,
        message: Option<String>,
        rows: Vec<Row>,
        window: &mut Window,
        cx: &mut Context<Self>,
        chosen: impl Fn(&mut Self, &str, &mut Window, &mut Context<Self>) + 'static,
    );
    /// `openIn` or `autoConsole` changed: the running app follows (the
    /// controller watcher starts or stops now, not at the next start).
    fn start_options_changed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

/// Reads the saved themes off the UI thread.
pub fn load_saved<H: Host>(h: &mut H, cx: &mut Context<H>) {
    let app = h.app();
    cx.spawn(async move |this, cx| {
        let list = cx.background_spawn(async move { store::list(&app) }).await;
        this.update(cx, |h, cx| {
            if let Some(st) = h.theme_state() {
                st.data.saved = list;
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Applies `cmd` through `themes::update` (the screen behind follows at
/// once); whether anything changed. The startup options go through
/// `console::startup`, which mirrors them for the next start.
pub fn change<H: Host>(h: &mut H, cmd: &Command, cx: &mut Context<H>) -> bool {
    let app = h.app();
    let Some(st) = h.theme_state() else { return false };
    let mut probe = st.prefs.clone();
    if !run(&mut probe, cmd, &st.data) {
        return false;
    }
    st.prefs = match cmd.id.as_str() {
        "startup.open_in" => {
            crate::console::startup::set_open_in(&app, probe.open_in);
            themes::prefs(&app)
        }
        "startup.auto_console" => {
            crate::console::startup::set_auto_console(&app, probe.auto_console);
            themes::prefs(&app)
        }
        _ => {
            st.remember(cmd);
            let data = &st.data;
            themes::update(&app, |p| {
                run(p, cmd, data);
            })
        }
    };
    st.notice = None;
    cx.notify();
    true
}

impl ThemeState {
    /// Empties Undo and Redo (another theme is in use, or it was saved).
    pub fn forget_steps(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_change = None;
    }

    /// The next change is a step of its own, even of the option changed last
    /// (a ↺ after a drag).
    pub fn end_step(&mut self) {
        self.last_change = None;
    }

    /// Keeps the theme as it is before `cmd` changes it, for Undo.
    fn remember(&mut self, cmd: &Command) {
        if matches!(cmd.id.as_str(), "theme.use" | "theme.reset") {
            self.forget_steps();
            return;
        }
        let drag = matches!(cmd.arg, Arg::Text(_) | Arg::Value(crate::console::list::Value::Number { .. }));
        let same = drag && self.last_change.as_deref() == Some(cmd.id.as_str());
        self.last_change = Some(cmd.id.clone());
        if same {
            return;
        }
        self.undo.push(self.prefs.theme.clone());
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

/// Puts back the theme as it was before the last change (`back`), or the
/// change undone last.
pub fn undo<H: Host>(h: &mut H, back: bool, cx: &mut Context<H>) {
    let app = h.app();
    let Some(st) = h.theme_state() else { return };
    let (from, to) = if back { (&mut st.undo, &mut st.redo) } else { (&mut st.redo, &mut st.undo) };
    let Some(mut theme) = from.pop() else { return };
    // The id says which saved theme Save writes: it stays the current one.
    theme.id = st.prefs.theme.id.clone();
    to.push(st.prefs.theme.clone());
    st.last_change = None;
    st.prefs = themes::update(&app, |p| p.theme = theme);
    st.notice = None;
    cx.notify();
}

/// What came out of a row list: a changed value or an activated row.
pub fn on_outcome<H: Host>(h: &mut H, outcome: Outcome, window: &mut Window, cx: &mut Context<H>) {
    match outcome {
        Outcome::Changed(id, value) => {
            if change(h, &Command::changed(&id, value), cx) && id.starts_with("startup.") {
                h.start_options_changed(window, cx);
            }
        }
        Outcome::Activate(id) => {
            let Some(step) = h.theme_state().map(|st| activate(&id, &st.ctx())) else { return };
            do_step(h, step, window, cx);
        }
        _ => {}
    }
}

fn do_step<H: Host>(h: &mut H, step: Step, window: &mut Window, cx: &mut Context<H>) {
    match step {
        Step::Nothing => {}
        Step::Run(cmd) => {
            change(h, &cmd, cx);
        }
        Step::Store(op) => store_op(h, op, window, cx),
        Step::Text { id, title, initial } => {
            h.open_osk(&title, &initial, window, cx, move |h, text, window, cx| text_done(h, &id, text, window, cx));
        }
        Step::File { id, title, start } => {
            h.open_browser(
                &title,
                PickKind::File(vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()]),
                start,
                window,
                cx,
                move |h, path, _, cx| {
                    change(h, &Command::new(id.clone(), Arg::Text(path.display().to_string())), cx);
                },
            );
        }
        Step::Pick { id, title, rows } => {
            h.open_sheet(&title, None, rows, window, cx, move |h, picked, _, cx| {
                change(h, &Command::new(id.clone(), Arg::Pick(picked.to_string())), cx);
            });
        }
        Step::Confirm { title, message, yes, then } => {
            let rows = vec![Row::action("no", tr!("Cancel")), Row::action("yes", yes).danger()];
            h.open_sheet(&title, Some(message), rows, window, cx, move |h, picked, window, cx| {
                if picked == "yes" {
                    store_op(h, then.clone(), window, cx);
                }
            });
        }
    }
}

/// A text typed for the row `id`.
pub fn text_done<H: Host>(h: &mut H, id: &str, text: String, window: &mut Window, cx: &mut Context<H>) {
    match id {
        "theme.save-as" => {
            let name = text.trim().to_string();
            if !name.is_empty() {
                store_op(h, StoreOp::SaveAs { name, author: String::new(), description: String::new(), version: String::new() }, window, cx);
            }
        }
        "theme.rename" => {
            let name = text.trim().to_string();
            if !name.is_empty() {
                store_op(h, StoreOp::Rename(name), window, cx);
            }
        }
        _ => {
            let colour = id.starts_with("colors.") || id == "interface.accent_hex";
            if colour && super::normalize_colour(&text).is_none() {
                if let Some(st) = h.theme_state() {
                    st.notice = Some((tr!("That is not a colour: write it as #rrggbb.").into(), true));
                }
                cx.notify();
                return;
            }
            change(h, &Command::new(id, Arg::Text(text)), cx);
        }
    }
}

fn fail<H: Host>(h: &mut H, e: anyhow::Error, cx: &mut Context<H>) {
    if let Some(st) = h.theme_state() {
        st.set_error(e);
    }
    cx.notify();
}

/// Makes `theme` the one in use, based on itself, and refreshes the list.
fn adopt<H: Host>(h: &mut H, theme: ConsoleTheme, list: Vec<ConsoleTheme>, note: String, cx: &mut Context<H>) {
    let app = h.app();
    let id = theme.id.clone();
    let next = themes::apply(&app, theme, Some(&id));
    if let Some(st) = h.theme_state() {
        st.prefs = next;
        st.data.saved = list;
        st.notice = Some((note, false));
        // A step from before would bring back another theme's id or picture.
        st.forget_steps();
    }
    cx.notify();
}

/// Runs a theme operation off the UI thread; errors become the notice.
pub fn store_op<H: Host>(h: &mut H, op: StoreOp, window: &mut Window, cx: &mut Context<H>) {
    let app = h.app();
    let Some(current) = h.theme_state().map(|st| st.prefs.theme.clone()) else { return };
    match op {
        StoreOp::Save => save_job(h, app, current.clone(), current, tr!("Saved.").into(), cx),
        StoreOp::SaveAs { name, author, description, version } => {
            let mut t = current.clone();
            t.name = name;
            t.author = author;
            t.description = description;
            t.version = if version.is_empty() { "1".into() } else { version };
            t.id.clear();
            save_job(h, app, current, t, tr!("Saved as a new theme.").into(), cx);
        }
        StoreOp::Rename(name) => rename_job(h, app, name, cx),
        StoreOp::SaveDetails { name, author } => {
            let mut t = current.clone();
            t.name = name;
            t.author = author;
            save_job(h, app, current, t, tr!("Saved.").into(), cx);
        }
        StoreOp::Delete => {
            let id = h.theme_state().map(|st| st.prefs.based_on.clone()).unwrap_or_default();
            delete_job(h, app, id, cx);
        }
        StoreOp::DeleteSaved(id) => delete_job(h, app, id, cx),
        StoreOp::Export => export(h, current, window, cx),
        StoreOp::Import => import(h, window, cx),
    }
}

/// Deletes the saved theme `id`; the theme in use stops being based on it
/// (it stays on screen, as "Customised").
fn delete_job<H: Host>(_h: &mut H, app: AppPaths, id: String, cx: &mut Context<H>) {
    cx.spawn(async move |this, cx| {
        let res = cx
            .background_spawn({
                let (app, id) = (app.clone(), id.clone());
                async move { store::delete(&app, &id).map(|()| store::list(&app)) }
            })
            .await;
        this.update(cx, |h, cx| match res {
            Ok(list) => {
                let app = h.app();
                let in_use = h.theme_state().is_some_and(|st| st.prefs.based_on == id);
                let prefs = in_use.then(|| themes::update(&app, |p| p.based_on.clear()));
                if let Some(st) = h.theme_state() {
                    st.data.saved = list;
                    if let Some(p) = prefs {
                        st.prefs = p;
                        st.forget_steps();
                    }
                    st.notice = Some((tr!("Deleted.").into(), false));
                }
                cx.notify();
            }
            Err(e) => {
                fail(h, e, cx);
            }
        })
        .ok();
    })
    .detach();
}

/// Renames the saved theme in use on disk and in use, leaving its other
/// unsaved changes as they are.
fn rename_job<H: Host>(h: &mut H, app: AppPaths, name: String, cx: &mut Context<H>) {
    let Some(saved) = h.theme_state().and_then(|st| super::based_on_saved(&st.prefs, &st.data.saved).cloned()) else { return };
    cx.spawn(async move |this, cx| {
        let res = cx
            .background_spawn({
                let app = app.clone();
                async move {
                    let mut t = saved;
                    t.name = name;
                    store::save(&app, &t).map(|t| (t, store::list(&app)))
                }
            })
            .await;
        this.update(cx, |h, cx| match res {
            Ok((t, list)) => {
                let prefs = themes::update(&app, |p| p.theme.name = t.name.clone());
                if let Some(st) = h.theme_state() {
                    st.prefs = prefs;
                    st.data.saved = list;
                    st.notice = Some((tr!("Renamed.").into(), false));
                }
                cx.notify();
            }
            Err(e) => fail(h, e, cx),
        })
        .ok();
    })
    .detach();
}

/// Saves `theme` (made from `start`, the theme in use when the save began)
/// and makes it the one in use. A change made while the file was written is
/// kept: only what the save gave the theme (its id, details and picture's
/// path) is put on it.
fn save_job<H: Host>(_h: &mut H, app: AppPaths, start: ConsoleTheme, theme: ConsoleTheme, done: String, cx: &mut Context<H>) {
    cx.spawn(async move |this, cx| {
        let res = cx.background_spawn(async move { store::save(&app, &theme).map(|t| (t, store::list(&app))) }).await;
        this.update(cx, |h, cx| match res {
            Ok((t, list)) if h.theme_state().is_none_or(|st| st.prefs.theme == start) => adopt(h, t, list, done, cx),
            Ok((t, list)) => {
                let app = h.app();
                let next = themes::update(&app, |p| {
                    if p.theme.background.image == start.background.image {
                        p.theme.background.image = t.background.image.clone();
                    }
                    p.theme.id = t.id.clone();
                    p.theme.name = t.name.clone();
                    p.theme.author = t.author.clone();
                    p.theme.description = t.description.clone();
                    p.theme.version = t.version.clone();
                    p.based_on = t.id.clone();
                });
                if let Some(st) = h.theme_state() {
                    st.prefs = next;
                    st.data.saved = list;
                    st.notice = Some((done, false));
                    st.forget_steps();
                }
                cx.notify();
            }
            Err(e) => {
                fail(h, e, cx);
            }
        })
        .ok();
    })
    .detach();
}

fn downloads() -> PathBuf {
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."))
}

fn write_export<H: Host>(h: &mut H, theme: &ConsoleTheme, to: &Path, cx: &mut Context<H>) {
    let Some(st) = h.theme_state() else { return };
    st.notice = Some(match store::export(theme, to) {
        Ok(()) => (tr!("Exported to {path}", path = to.display()), false),
        Err(e) => (format!("{e:#}"), true),
    });
    cx.notify();
}

fn export<H: Host>(h: &mut H, theme: ConsoleTheme, window: &mut Window, cx: &mut Context<H>) {
    let name = export_name(&theme);
    if crate::console::active() {
        h.open_browser(tr!("Export to this folder (X)"), PickKind::Folder, Some(downloads()), window, cx, move |h, dir, _, cx| {
            write_export(h, &theme, &dir.join(&name), cx);
        });
        return;
    }
    let rx = cx.prompt_for_new_path(&downloads(), Some(&name));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(path))) = rx.await else { return };
        this.update(cx, |h, cx| write_export(h, &theme, &path, cx)).ok();
    })
    .detach();
}

fn import<H: Host>(h: &mut H, window: &mut Window, cx: &mut Context<H>) {
    if crate::console::active() {
        h.open_browser(tr!("Import a theme"), PickKind::File(vec!["json".into()]), Some(downloads()), window, cx, |h, path, _, cx| {
            import_from(h, path, cx);
        });
        return;
    }
    let rx =
        cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        this.update(cx, |h, cx| import_from(h, path, cx)).ok();
    })
    .detach();
}

fn import_from<H: Host>(h: &mut H, path: PathBuf, cx: &mut Context<H>) {
    let app = h.app();
    cx.spawn(async move |this, cx| {
        // A theme that is already saved keeps its copy: this one gets a new id.
        let res = cx.background_spawn(async move { store::import(&app, &path).map(|t| (t, store::list(&app))) }).await;
        this.update(cx, |h, cx| match res {
            // Unsaved changes on screen are never replaced: the theme waits in the list.
            Ok((t, list)) if h.theme_state().is_some_and(|st| super::is_edited(&st.prefs, &st.data.saved)) => {
                if let Some(st) = h.theme_state() {
                    st.data.saved = list;
                    st.notice = Some((tr!("“{name}” is with your themes.", name = t.name), false));
                }
                cx.notify();
            }
            Ok((t, list)) => adopt(h, t, list, tr!("Imported.").into(), cx),
            Err(e) => {
                fail(h, e, cx);
            }
        })
        .ok();
    })
    .detach();
}

/// How a press that only some rows take went.
pub enum Extra {
    /// Not for this layer: the list handles it.
    Pass,
    Handled,
    /// Apply this; when it changed something, move the focus by the number
    /// of rows.
    Run(Command, i32),
}

/// X starts moving a shelf or menu item (↑ ↓ move it, A / B / X end),
/// LT removes a shelf.
pub fn extra_pad(st: &mut ThemeState, pad: Pad, focused: Option<&str>) -> Extra {
    if st.moving {
        let Some(id) = focused.filter(|id| movable(id)) else {
            st.moving = false;
            return Extra::Handled;
        };
        return match pad {
            Pad::Up => Extra::Run(Command::new(id, Arg::Move(-1)), -1),
            Pad::Down => Extra::Run(Command::new(id, Arg::Move(1)), 1),
            Pad::A | Pad::B | Pad::X => {
                st.moving = false;
                Extra::Handled
            }
            _ => Extra::Handled,
        };
    }
    match (pad, focused) {
        (Pad::X, Some(id)) if movable(id) => {
            st.moving = true;
            Extra::Handled
        }
        (Pad::LT, Some(id)) if removable(id) => Extra::Run(Command::new(id, Arg::Remove), 0),
        _ => Extra::Pass,
    }
}

/// Hints that depend on the focused row.
pub fn extra_hints(st: &ThemeState, focused: Option<&str>) -> Vec<Hint> {
    if st.moving {
        return vec![hint(Btn::A, tr!("Done"))];
    }
    let mut h = Vec::new();
    if let Some(id) = focused {
        if movable(id) {
            h.push(hint(Btn::X, tr!("Move")));
        }
        if removable(id) {
            h.push(hint(Btn::LT, tr!("Remove")));
        }
    }
    h
}

/// A button of an editor: the action row `id`.
pub fn run_press<H: Host>(h: &mut H, id: &str, cx: &mut Context<H>) {
    change(h, &Command::new(id, Arg::Press), cx);
}

/// A button of an editor: moves the shelf or menu item `id`.
pub fn run_move<H: Host>(h: &mut H, id: &str, dir: i32, cx: &mut Context<H>) {
    change(h, &Command::new(id, Arg::Move(dir)), cx);
}
