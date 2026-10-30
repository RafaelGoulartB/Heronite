//! Reading the logs: the list of entries and the content of the selected one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::{DataPaths, Library};
use heronite_i18n::tr;
use heronite_launch::log_reader::{display_lines, list_logs, read_tail, LogEntry, LogSource};
use heronite_launch::log_writer::LogPaths;

use super::{Content, LogViewer, Scope, Start, FIRST_VIEW_BYTES, MAX_VIEW_BYTES};

/// Everything found on disk (run off the UI thread).
fn collect(data_paths: &DataPaths, scope: &Scope) -> (Vec<LogEntry>, HashMap<String, String>) {
    let paths = LogPaths::for_data(data_paths);
    let mut entries = list_logs(&paths, data_paths);
    if let Scope::Game(app) = scope {
        entries.retain(|e| e.app_name() == Some(app.as_str()));
    }
    let titles = match Library::load_with(data_paths, true) {
        Ok(lib) => lib.games.iter().map(|g| (g.app_name.clone(), g.title.clone())).collect(),
        Err(_) => HashMap::new(),
    };
    (entries, titles)
}

fn read_content(path: &Path, bytes: u64) -> Result<Content, String> {
    let tail = read_tail(path, bytes).map_err(|e| tr!("Could not read {path}: {error}", path = path.display(), error = e))?;
    let lines = display_lines(&tail.text).into_iter().map(SharedString::from).collect();
    Ok(Content { path: path.to_path_buf(), lines: Arc::new(lines), total: tail.total, skipped: tail.skipped })
}

/// Debug captures only keep the first frames, so the data is read in place.
pub(super) fn read_in_place() -> bool {
    crate::env::flag("HERONITE_DEBUG_PAGE") || crate::env::flag("HERONITE_DEBUG_DIALOG")
}

impl LogViewer {
    pub(super) fn reload_entries(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        if read_in_place() {
            let (entries, titles) = collect(&self.data_paths, &self.scope);
            self.apply_entries(entries, titles, cx);
            return;
        }
        let (data_paths, scope) = (self.data_paths.clone(), self.scope.clone());
        cx.spawn(async move |this, cx| {
            let (entries, titles) = cx.background_spawn(async move { collect(&data_paths, &scope) }).await;
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
                .position(|e| {
                    e.app_name() == Some(app.as_str())
                        && matches!(e.source, LogSource::Game { ty: heronite_launch::log_writer::GameLogType::Launch, .. })
                })
                .or_else(|| self.entries.iter().position(|e| e.app_name() == Some(app.as_str()))),
        };
        self.selected =
            by_path.or_else(|| if first_time { by_start() } else { None }).or(if self.entries.is_empty() { None } else { Some(0) });
        if by_path.is_none() {
            self.previous = false;
            self.window_bytes = FIRST_VIEW_BYTES;
        }
        self.load_content(false, cx);
        cx.notify();
    }

    pub(super) fn current_path(&self) -> Option<PathBuf> {
        let e = self.entries.get(self.selected?)?;
        if self.previous {
            e.previous.clone().or_else(|| Some(e.path.clone()))
        } else {
            Some(e.path.clone())
        }
    }

    pub(super) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.selected == Some(index) {
            return;
        }
        self.selected = Some(index);
        self.previous = false;
        self.window_bytes = FIRST_VIEW_BYTES;
        self.load_content(false, cx);
        cx.notify();
    }

    pub(super) fn set_previous(&mut self, previous: bool, cx: &mut Context<Self>) {
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

    pub(super) fn load_earlier(&mut self, cx: &mut Context<Self>) {
        self.window_bytes = (self.window_bytes * 4).min(MAX_VIEW_BYTES);
        self.load_content(true, cx);
        cx.notify();
    }

    pub(super) fn can_load_earlier(&self) -> bool {
        self.content.skipped > 0 && self.window_bytes < MAX_VIEW_BYTES
    }

    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self.content.lines.iter().map(|l| l.as_ref()).collect::<Vec<_>>().join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = true;
        cx.notify();
    }

    pub(super) fn open(&mut self, target: PathBuf, cx: &mut Context<Self>) {
        self.notice = match heronite_platform::shell::open(&target) {
            Ok(_) => None,
            Err(e) => Some(tr!("Could not open {path}: {error}", path = target.display(), error = e).into()),
        };
        cx.notify();
    }

    /// (name, detail) shown for an entry.
    pub(super) fn label(&self, e: &LogEntry) -> (String, String) {
        let title = |app: &str| self.titles.get(app).cloned().unwrap_or_else(|| app.to_string());
        let day = || {
            e.path
                .file_name()
                .map(|n| {
                    n.to_string_lossy().trim_start_matches("heronite-rs-").trim_end_matches(".log").trim_end_matches(".old").to_string()
                })
                .unwrap_or_default()
        };
        match &e.source {
            LogSource::Launcher => (tr!("Heronite (this app)").into(), day()),
            LogSource::Runner(r) => (tr!("{runner} runner", runner = r), format!("runners/{r}.log")),
            LogSource::Game { app_name, runner, ty } => (title(app_name), format!("{} - {runner}", ty.label())),
            LogSource::Legacy { app_name } => (title(app_name), tr!("Older log").into()),
        }
    }

    pub(super) fn group_of(e: &LogEntry) -> (u8, &'static str) {
        match e.source {
            LogSource::Launcher => (0, tr!("This app")),
            LogSource::Runner(_) => (2, tr!("Runners")),
            LogSource::Game { .. } | LogSource::Legacy { .. } => (3, tr!("Games")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_content;

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
