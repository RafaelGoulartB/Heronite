//! Log viewer: a pane that lists the logs (this app, Heroic, runners, every
//! game) and shows one of them, plus a dialog that hosts the pane for a single
//! game ("View log" in the game's context menu). Settings → Logs embeds the
//! same pane with the full list.
//!
//! Big files are never read whole: the pane loads the last 512 KiB (a line
//! boundary away), renders only the visible lines (GPUI `list`), and "Load
//! earlier" reads more, up to [`MAX_VIEW_BYTES`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_data::{HeroicPaths, Library};
use heronite_launch::log_reader::{display_lines, list_logs, read_tail, LogEntry, LogSource};
use heronite_launch::log_writer::LogPaths;

use super::{DialogCtx, DialogEvent};
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::meta_text;

/// First read of a log, and each "Load earlier" step multiplies it by 4.
const FIRST_VIEW_BYTES: u64 = 512 * 1024;
/// The pane never holds more than this of one file (the rest: open it
/// externally).
pub const MAX_VIEW_BYTES: u64 = 32 * 1024 * 1024;

/// Which logs the pane offers.
#[derive(Clone, Debug)]
pub enum Scope {
    All,
    /// Only the logs of this app name.
    Game(String),
}

/// Which log is selected first.
#[derive(Clone, Debug)]
pub enum Start {
    /// The newest log of the scope.
    First,
    Launcher,
    Game(String),
}

#[derive(Default)]
struct Content {
    path: PathBuf,
    lines: Arc<Vec<SharedString>>,
    total: u64,
    skipped: u64,
}

pub struct LogViewer {
    heroic: HeroicPaths,
    scope: Scope,
    start: Start,
    show_list: bool,
    /// Fixed height in px; `None` follows the window.
    height: Option<f32>,
    entries: Vec<LogEntry>,
    titles: HashMap<String, String>,
    entries_loaded: bool,
    selected: Option<usize>,
    /// Showing the previous run (`.old`) of the selected log.
    previous: bool,
    window_bytes: u64,
    content: Content,
    list_state: ListState,
    generation: u64,
    busy: bool,
    error: Option<SharedString>,
    notice: Option<SharedString>,
    copied: bool,
    mono: SharedString,
}

/// A monospace family that exists on this machine (GPUI does not resolve the
/// generic `monospace`).
fn mono_family(cx: &App) -> SharedString {
    const CANDIDATES: [&str; 9] = [
        "JetBrains Mono",
        "JetBrainsMono Nerd Font",
        "Adwaita Mono",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Noto Sans Mono",
        "Nimbus Mono PS",
        "Consolas",
        "Menlo",
    ];
    let names = cx.text_system().all_font_names();
    CANDIDATES.iter().find(|c| names.iter().any(|n| n == *c)).map_or(SharedString::from("monospace"), |c| SharedString::from(*c))
}

/// Everything found on disk (run off the UI thread).
fn collect(heroic: &HeroicPaths, scope: &Scope) -> (Vec<LogEntry>, HashMap<String, String>) {
    let paths = LogPaths::for_heroic(heroic);
    let mut entries = list_logs(&paths, heroic);
    if let Scope::Game(app) = scope {
        entries.retain(|e| e.app_name() == Some(app.as_str()));
    }
    let titles = match Library::load_with(heroic, true) {
        Ok(lib) => lib.games.iter().map(|g| (g.app_name.clone(), g.title.clone())).collect(),
        Err(_) => HashMap::new(),
    };
    (entries, titles)
}

fn read_content(path: &Path, bytes: u64) -> Result<Content, String> {
    let tail = read_tail(path, bytes).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let lines = display_lines(&tail.text).into_iter().map(SharedString::from).collect();
    Ok(Content { path: path.to_path_buf(), lines: Arc::new(lines), total: tail.total, skipped: tail.skipped })
}

/// Debug captures only keep the first frames, so the data is read in place.
fn read_in_place() -> bool {
    std::env::var_os("HERONITE_DEBUG_PAGE").is_some() || std::env::var_os("HERONITE_DEBUG_DIALOG").is_some()
}

fn when(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string()
}

impl LogViewer {
    pub fn new(heroic: HeroicPaths, scope: Scope, start: Start, show_list: bool, height: Option<f32>, cx: &mut Context<Self>) -> Self {
        let list_state = ListState::new(0, ListAlignment::Top, px(400.));
        let weak = cx.entity().downgrade();
        list_state.set_scroll_handler(move |_, _, cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |_, cx| cx.notify());
            }
        });
        let mut this = Self {
            heroic,
            scope,
            start,
            show_list,
            height,
            entries: Vec::new(),
            titles: HashMap::new(),
            entries_loaded: false,
            selected: None,
            previous: false,
            window_bytes: FIRST_VIEW_BYTES,
            content: Content::default(),
            list_state,
            generation: 0,
            busy: false,
            error: None,
            notice: None,
            copied: false,
            mono: mono_family(cx),
        };
        this.reload_entries(cx);
        // Debug: `HERONITE_DEBUG_LOG_EARLIER=<n>` presses "Load earlier" n times.
        if let Some(n) = std::env::var("HERONITE_DEBUG_LOG_EARLIER").ok().and_then(|v| v.parse::<u32>().ok()).filter(|_| read_in_place()) {
            for _ in 0..n {
                this.load_earlier(cx);
            }
        }
        this
    }

    /// Re-reads which logs exist and the selected one.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload_entries(cx);
    }

    fn reload_entries(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        if read_in_place() {
            let (entries, titles) = collect(&self.heroic, &self.scope);
            self.apply_entries(entries, titles, cx);
            return;
        }
        let (heroic, scope) = (self.heroic.clone(), self.scope.clone());
        cx.spawn(async move |this, cx| {
            let (entries, titles) = cx.background_spawn(async move { collect(&heroic, &scope) }).await;
            this.update(cx, |this, cx| this.apply_entries(entries, titles, cx)).ok();
        })
        .detach();
    }

    fn apply_entries(&mut self, entries: Vec<LogEntry>, titles: HashMap<String, String>, cx: &mut Context<Self>) {
        let keep = self.selected.and_then(|i| self.entries.get(i)).map(|e| e.path.clone());
        self.entries = entries;
        self.titles = titles;
        let first_time = !self.entries_loaded;
        self.entries_loaded = true;
        let by_path = keep.and_then(|p| self.entries.iter().position(|e| e.path == p));
        let by_start = || match &self.start {
            Start::First => None,
            Start::Launcher => self.entries.iter().position(|e| e.source == LogSource::Launcher),
            Start::Game(app) => self
                .entries
                .iter()
                .position(|e| e.app_name() == Some(app.as_str()) && matches!(e.source, LogSource::Game { ty: heronite_launch::log_writer::GameLogType::Launch, .. }))
                .or_else(|| self.entries.iter().position(|e| e.app_name() == Some(app.as_str()))),
        };
        self.selected = by_path.or_else(|| if first_time { by_start() } else { None }).or(if self.entries.is_empty() { None } else { Some(0) });
        if by_path.is_none() {
            self.previous = false;
            self.window_bytes = FIRST_VIEW_BYTES;
        }
        self.load_content(false, cx);
        cx.notify();
    }

    fn current_path(&self) -> Option<PathBuf> {
        let e = self.entries.get(self.selected?)?;
        if self.previous {
            e.previous.clone().or_else(|| Some(e.path.clone()))
        } else {
            Some(e.path.clone())
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.selected == Some(index) {
            return;
        }
        self.selected = Some(index);
        self.previous = false;
        self.window_bytes = FIRST_VIEW_BYTES;
        self.load_content(false, cx);
        cx.notify();
    }

    fn set_previous(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.previous == previous {
            return;
        }
        self.previous = previous;
        self.window_bytes = FIRST_VIEW_BYTES;
        self.load_content(false, cx);
        cx.notify();
    }

    /// Reads the end of the selected log. `earlier`: the window just grew, so
    /// keep the lines on screen where they are.
    fn load_content(&mut self, earlier: bool, cx: &mut Context<Self>) {
        self.copied = false;
        let Some(path) = self.current_path() else {
            self.content = Content::default();
            self.list_state.reset(0);
            self.busy = false;
            self.error = None;
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        let bytes = self.window_bytes;
        if read_in_place() {
            let result = read_content(&path, bytes);
            self.apply_content(result, earlier, cx);
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { read_content(&path, bytes) }).await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.apply_content(result, earlier, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_content(&mut self, result: Result<Content, String>, earlier: bool, _cx: &mut Context<Self>) {
        self.busy = false;
        match result {
            Ok(content) => {
                self.error = None;
                let top = self.list_state.logical_scroll_top();
                let added = content.lines.len().saturating_sub(self.content.lines.len());
                let same_file = self.content.path == content.path;
                self.list_state.reset(content.lines.len());
                if earlier && same_file {
                    self.list_state.scroll_to(ListOffset { item_ix: top.item_ix + added, offset_in_item: top.offset_in_item });
                } else {
                    // A log is read from its end.
                    self.list_state.scroll_to_end();
                }
                self.content = content;
            }
            Err(e) => {
                self.error = Some(e.into());
                self.content = Content::default();
                self.list_state.reset(0);
            }
        }
    }

    fn load_earlier(&mut self, cx: &mut Context<Self>) {
        self.window_bytes = (self.window_bytes * 4).min(MAX_VIEW_BYTES);
        self.load_content(true, cx);
        cx.notify();
    }

    fn can_load_earlier(&self) -> bool {
        self.content.skipped > 0 && self.window_bytes < MAX_VIEW_BYTES
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self.content.lines.iter().map(|l| l.as_ref()).collect::<Vec<_>>().join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = true;
        cx.notify();
    }

    fn open(&mut self, target: PathBuf, cx: &mut Context<Self>) {
        self.notice = match Command::new("xdg-open").arg(&target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(_) => None,
            Err(e) => Some(format!("Could not open {}: {e}", target.display()).into()),
        };
        cx.notify();
    }

    /// (name, detail) shown for an entry.
    fn label(&self, e: &LogEntry) -> (String, String) {
        let title = |app: &str| self.titles.get(app).cloned().unwrap_or_else(|| app.to_string());
        let day = || {
            e.path
                .file_name()
                .map(|n| n.to_string_lossy().trim_start_matches("heronite-rs-").trim_end_matches(".log").trim_end_matches(".old").to_string())
                .unwrap_or_default()
        };
        match &e.source {
            LogSource::Launcher => ("Heronite (this app)".into(), day()),
            LogSource::Heroic => ("Heroic / Heronite".into(), "heroic.log".into()),
            LogSource::Runner(r) => (format!("{r} runner"), format!("runners/{r}.log")),
            LogSource::Game { app_name, runner, ty } => (title(app_name), format!("{} - {runner}", ty.label())),
            LogSource::Legacy { app_name } => (title(app_name), "Older Heroic log".into()),
        }
    }

    fn group_of(e: &LogEntry) -> (u8, &'static str) {
        match e.source {
            LogSource::Launcher => (0, "This app"),
            LogSource::Heroic => (1, "Heroic / Heronite"),
            LogSource::Runner(_) => (2, "Runners"),
            LogSource::Game { .. } | LogSource::Legacy { .. } => (3, "Games"),
        }
    }
}

/// Colour of a log line by its level.
fn line_color(line: &str) -> Hsla {
    if line.starts_with("===") {
        theme::white_a(0.45)
    } else if line.contains("[ERROR]") {
        rgb(0xe58a93).into()
    } else if line.contains("[WARNING]") {
        rgb(0xebcb8b).into()
    } else {
        rgb(0xd8dee9).into()
    }
}

impl LogViewer {
    fn render_list(&self, height: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut col = div().flex().flex_col().gap(px(2.)).p(px(6.));
        let mut last_group = None;
        if self.entries.is_empty() {
            col = col.child(div().p(px(8.)).child(meta_text(if self.entries_loaded { "No logs yet." } else { "Looking for logs..." })));
        }
        for (i, e) in self.entries.iter().enumerate() {
            let (g, header) = Self::group_of(e);
            if last_group != Some(g) {
                last_group = Some(g);
                col = col.child(
                    div()
                        .px(px(8.))
                        .pt(px(if i == 0 { 2. } else { 10. }))
                        .pb(px(2.))
                        .text_size(px(12.))
                        .font_weight(theme::W_BOLD)
                        .text_color(theme::white_a(0.55))
                        .child(header.to_uppercase()),
                );
            }
            let active = self.selected == Some(i);
            let (name, detail) = self.label(e);
            col = col.child(
                div()
                    .id(SharedString::from(format!("log-entry-{i}")))
                    .flex()
                    .flex_col()
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(theme::white_a(0.1)))
                    .hover(|d| d.bg(theme::white_a(0.08)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(i, cx)))
                    .child(
                        div()
                            .text_size(px(14.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(if active { theme::accent() } else { rgb(0xcaf3fd) })
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(theme::white_a(0.55))
                            .child(format!("{detail} - {}", human_size(e.size))),
                    ),
            );
        }
        div()
            .id("log-entries")
            .w(px(240.))
            .flex_none()
            .h(px(height))
            .overflow_y_scroll()
            .rounded(px(4.))
            .bg(theme::white_a(0.04))
            .border_1()
            .border_color(theme::white_a(0.08))
            .child(col)
    }

    fn render_pane(&mut self, cx: &mut Context<Self>) -> Div {
        let entry = self.selected.and_then(|i| self.entries.get(i)).cloned();
        let (name, _) = entry.as_ref().map(|e| self.label(e)).unwrap_or_default();
        let path = self.current_path();
        let has_previous = entry.as_ref().is_some_and(|e| e.previous.is_some());
        let showing_previous = self.previous && has_previous;

        // Title, run switch and actions.
        let mut bar = div().flex().flex_wrap().items_center().gap(px(8.));
        bar = bar.child(
            div()
                .flex_1()
                .min_w(px(120.))
                .font_family(theme::FONT_HEADING)
                .text_size(px(17.))
                .font_weight(theme::W_BOLD)
                .text_color(white())
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(if name.is_empty() { "Logs".to_string() } else { name }),
        );
        if has_previous {
            bar = bar
                .child(action_button(
                    "log-latest",
                    if showing_previous { ButtonKind::Outline } else { ButtonKind::Tertiary },
                    "Latest",
                    true,
                    cx.listener(|this, _, _, cx| this.set_previous(false, cx)),
                ))
                .child(action_button(
                    "log-previous",
                    if showing_previous { ButtonKind::Tertiary } else { ButtonKind::Outline },
                    "Previous run",
                    true,
                    cx.listener(|this, _, _, cx| this.set_previous(true, cx)),
                ));
        }
        let has_path = path.is_some();
        let (p1, p2) = (path.clone(), path.clone());
        bar = bar
            .child(action_button("log-refresh", ButtonKind::Outline, "Refresh", true, cx.listener(|this, _, _, cx| this.refresh(cx))))
            .child(action_button(
                "log-copy",
                ButtonKind::Outline,
                if self.copied { "Copied!" } else { "Copy" },
                has_path && !self.content.lines.is_empty(),
                cx.listener(|this, _, _, cx| this.copy(cx)),
            ))
            .child(action_button(
                "log-folder",
                ButtonKind::Outline,
                "Open folder",
                has_path,
                cx.listener(move |this, _, _, cx| {
                    if let Some(dir) = p1.as_deref().and_then(Path::parent) {
                        this.open(dir.to_path_buf(), cx);
                    }
                }),
            ))
            .child(action_button(
                "log-open",
                ButtonKind::Outline,
                "Open file",
                has_path,
                cx.listener(move |this, _, _, cx| {
                    if let Some(file) = p2.clone() {
                        this.open(file, cx);
                    }
                }),
            ));

        // Where it is and how much of it is shown.
        let c = &self.content;
        let mut info = String::new();
        if let Some(p) = &path {
            info.push_str(&p.display().to_string());
        }
        if !c.lines.is_empty() || c.total > 0 {
            info.push_str(&format!("  -  {}", human_size(c.total)));
            if c.skipped > 0 {
                info.push_str(&format!(", showing the last {}", human_size(c.total - c.skipped)));
            }
            if let Some(t) = entry.as_ref().and_then(|e| e.modified) {
                if !showing_previous {
                    info.push_str(&format!("  -  {}", when(t)));
                }
            }
        }

        // The log itself: only the visible lines are laid out.
        let lines = c.lines.clone();
        let mono = self.mono.clone();
        let body = if let Some(err) = self.error.clone() {
            div().p(px(12.)).text_color(theme::danger()).child(err).into_any_element()
        } else if lines.is_empty() {
            let text = if self.busy {
                "Reading..."
            } else if self.selected.is_none() {
                "Nothing to show. Logs appear here after a game is launched."
            } else {
                "This log is empty."
            };
            div().p(px(12.)).child(meta_text(text)).into_any_element()
        } else {
            list(self.list_state.clone(), move |ix, _, _| {
                let line = lines[ix].clone();
                let color = line_color(&line);
                div()
                    .w_full()
                    .px(px(10.))
                    .font_family(mono.clone())
                    .text_size(px(12.5))
                    .line_height(px(17.))
                    .text_color(color)
                    .min_h(px(17.))
                    .child(line)
                    .into_any_element()
            })
            .size_full()
            .into_any_element()
        };
        let scrollbar = crate::ui::scrollbar_draggable(
            "log-scrollbar",
            self.list_state.scroll_px_offset_for_scrollbar().y,
            self.list_state.max_offset_for_scrollbar().y,
            self.list_state.viewport_bounds().size.height,
            {
                let (state, weak) = (self.list_state.clone(), cx.entity().downgrade());
                move |distance, _, cx| {
                    state.set_offset_from_scrollbar(point(px(0.), px(-distance)));
                    weak.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            Some(Rc::new({
                let state = self.list_state.clone();
                move |started| {
                    if started {
                        state.scrollbar_drag_started();
                    } else {
                        state.scrollbar_drag_ended();
                    }
                }
            })),
        );
        let viewer = div()
            .relative()
            .flex_1()
            .min_h_0()
            .rounded(px(4.))
            .bg(rgb(0x14161b))
            .border_1()
            .border_color(theme::white_a(0.08))
            .overflow_hidden()
            .child(body)
            .children(scrollbar);

        let mut footer = div().flex().items_center().gap(px(12.)).min_h(px(22.));
        if self.can_load_earlier() {
            footer = footer.child(action_button("log-earlier", ButtonKind::Outline, "Load earlier", true, cx.listener(|this, _, _, cx| this.load_earlier(cx))));
        } else if c.skipped > 0 {
            footer = footer.child(meta_text("Only the end of this big log is shown here. Use Open file to see all of it."));
        }
        if let Some(n) = self.notice.clone() {
            footer = footer.child(div().text_size(px(13.)).text_color(theme::danger()).child(n));
        }

        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(bar)
            .child(div().text_size(px(12.)).text_color(theme::white_a(0.55)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(info))
            .child(viewer)
            .child(footer)
    }
}

impl Render for LogViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height.unwrap_or_else(|| (f32::from(window.viewport_size().height) - 230.).clamp(320., 1100.));
        let mut row = div().flex().gap(px(12.)).w_full().h(px(height));
        if self.show_list {
            row = row.child(self.render_list(height, cx));
        }
        row.child(self.render_pane(cx))
    }
}

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

/// "View log" of one game.
pub struct LogViewerDialog {
    title: String,
    viewer: Entity<LogViewer>,
}

impl EventEmitter<DialogEvent> for LogViewerDialog {}

impl LogViewerDialog {
    pub fn new(ctx: DialogCtx, app_name: String, title: String, cx: &mut Context<Self>) -> Self {
        let heroic = ctx.heroic.clone();
        let start = Start::Game(app_name.clone());
        let viewer = cx.new(|cx| LogViewer::new(heroic, Scope::Game(app_name), start, true, Some(500.), cx));
        Self { title, viewer }
    }
}

impl Render for LogViewerDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "log-viewer".into(), title: format!("Logs - {}", self.title).into(), width: 1040., height: Some(520.) },
            self.viewer.clone().into_any_element(),
            None,
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui prelude has its own `test` attribute.
    use super::{line_color, read_content};
    use crate::theme;

    #[test]
    fn lines_get_a_colour_by_level() {
        assert_ne!(line_color("(10:00:00) [ERROR]:   boom"), line_color("plain"));
        assert_ne!(line_color("(10:00:00) [WARNING]: hmm"), line_color("plain"));
        assert_eq!(line_color("(10:00:00) [INFO]:    ok"), line_color("plain"));
        assert_eq!(line_color("============= End of log ============="), theme::white_a(0.45));
    }

    #[test]
    fn content_reads_the_end_and_cleans_lines() {
        let dir = std::env::temp_dir().join(format!("heronite-logviewer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.log");
        let mut text = String::new();
        for i in 0..50_000 {
            text.push_str(&format!("line {i}\n"));
        }
        text.push_str("\x1b[31mred\x1b[0m\n");
        std::fs::write(&p, &text).unwrap();
        let c = read_content(&p, 4096).unwrap();
        assert_eq!(c.lines.last().map(|l| l.to_string()).as_deref(), Some("red"));
        assert!(c.skipped > 0 && c.lines.len() < 1000);
        assert!(read_content(&dir.join("missing.log"), 10).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
