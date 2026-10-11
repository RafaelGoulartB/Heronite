//! Folder / file picker for console mode (the desktop uses the portal file
//! chooser, which a controller cannot drive). A opens a folder or picks a
//! file, X uses the current folder, Y jumps home, B goes up (cancels at the
//! top). Folders are read off the UI thread.

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::kit::{self, u};
use super::list::{ListNav, Outcome, Row};
use super::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::theme;

#[derive(Clone, Debug, PartialEq)]
pub enum PickKind {
    Folder,
    /// Files with one of these extensions (lower case, no dot); empty = any.
    File(Vec<String>),
}

pub enum BrowserEvent {
    Picked(PathBuf),
    Cancel,
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
}

pub struct Browser {
    title: SharedString,
    kind: PickKind,
    dir: PathBuf,
    entries: Vec<Entry>,
    loading: bool,
    error: Option<String>,
    nav: ListNav,
}

impl EventEmitter<BrowserEvent> for Browser {}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn read_dir(dir: &Path, kind: &PickKind) -> Result<Vec<Entry>, String> {
    let mut out: Vec<Entry> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let dir = path.is_dir();
            let keep = dir
                || match kind {
                    PickKind::Folder => false,
                    PickKind::File(exts) => {
                        exts.is_empty()
                            || path.extension().and_then(|x| x.to_str()).is_some_and(|x| exts.iter().any(|w| w.eq_ignore_ascii_case(x)))
                    }
                };
            keep.then_some(Entry { name, path, dir })
        })
        .collect();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

impl Browser {
    /// Starts at `start` (or its closest existing parent, or home).
    pub fn new(title: impl Into<SharedString>, kind: PickKind, start: Option<&Path>, cx: &mut Context<Self>) -> Self {
        let mut dir = start.map(Path::to_path_buf).unwrap_or_else(home);
        while !dir.is_dir() {
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => {
                    dir = home();
                    break;
                }
            }
        }
        let mut this = Self { title: title.into(), kind, dir: dir.clone(), entries: Vec::new(), loading: true, error: None, nav: ListNav::default() };
        this.open(dir, cx);
        this
    }

    fn open(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.dir = dir.clone();
        self.loading = true;
        self.nav.reset();
        let kind = self.kind.clone();
        cx.spawn(async move |this, cx| {
            let d = dir.clone();
            let read = cx.background_spawn(async move { read_dir(&d, &kind) }).await;
            this.update(cx, |this, cx| {
                if this.dir != dir {
                    return;
                }
                this.loading = false;
                match read {
                    Ok(list) => {
                        this.entries = list;
                        this.error = None;
                    }
                    Err(e) => {
                        this.entries.clear();
                        this.error = Some(e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.kind == PickKind::Folder {
            rows.push(Row::action("use", "Use this folder").icon(Icon::Check));
        }
        for (i, e) in self.entries.iter().enumerate() {
            rows.push(Row::action(SharedString::from(format!("e{i}")), e.name.clone()).icon(if e.dir { Icon::Folder } else { Icon::Description }));
        }
        rows
    }

    fn up(&mut self, cx: &mut Context<Self>) {
        match self.dir.parent() {
            Some(p) => self.open(p.to_path_buf(), cx),
            None => cx.emit(BrowserEvent::Cancel),
        }
    }
}

impl PadTarget for Browser {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        match pad {
            Pad::B => self.up(cx),
            Pad::Y => self.open(home(), cx),
            Pad::X if self.kind == PickKind::Folder => cx.emit(BrowserEvent::Picked(self.dir.clone())),
            Pad::Start => cx.emit(BrowserEvent::Cancel),
            _ => {
                let rows = self.rows();
                match self.nav.on_pad(pad, &rows) {
                    Outcome::Activate(id) if id.as_ref() == "use" => cx.emit(BrowserEvent::Picked(self.dir.clone())),
                    Outcome::Activate(id) => {
                        let i: usize = id.trim_start_matches('e').parse().unwrap_or(usize::MAX);
                        if let Some(e) = self.entries.get(i).cloned() {
                            if e.dir {
                                self.open(e.path, cx);
                            } else {
                                cx.emit(BrowserEvent::Picked(e.path));
                            }
                        }
                    }
                    Outcome::Ignored => return false,
                    _ => {}
                }
            }
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        let mut h = vec![hint(Btn::A, "Open")];
        if self.kind == PickKind::Folder {
            h.push(hint(Btn::X, "Use this folder"));
        }
        h.extend([hint(Btn::Y, "Home"), hint(Btn::B, "Up"), hint(Btn::Start, "Cancel")]);
        h
    }
}

impl Render for Browser {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        let body: AnyElement = if self.loading {
            kit::body_text("Reading\u{2026}").px(u(22.)).into_any_element()
        } else if let Some(err) = &self.error {
            kit::body_text(format!("Cannot open this folder: {err}")).px(u(22.)).into_any_element()
        } else {
            self.nav.render("console-browser", &rows, true).flex_1().min_h_0().into_any_element()
        };
        div().absolute().inset_0().child(kit::scrim()).child(
            div().absolute().inset_0().flex().justify_center().py(u(40.)).child(kit::panel_in(
                "browser-panel",
                div()
                    .w(u(860.))
                    .flex()
                    .flex_col()
                    .gap(u(12.))
                    .p(u(24.))
                    .rounded(u(24.))
                    .bg(theme::console_sidebar())
                    .border_1()
                    .border_color(theme::white_a(0.1))
                    .child(kit::title(self.title.clone(), 24.))
                    .child(div().text_size(u(14.)).text_color(theme::text_muted()).truncate().child(self.dir.display().to_string()))
                    .shadow(kit::shadow(0.5, 18., 48.))
                    .child(div().flex().flex_col().flex_1().min_h_0().child(body)),
            )),
        )
    }
}
