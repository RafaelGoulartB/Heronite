//! Log viewer: a pane that lists the logs (this app, Heroic, runners, every
//! game) and shows one of them, plus a dialog that hosts the pane for a single
//! game ("View log" in the game's context menu). Settings → Logs embeds the
//! same pane with the full list.
//!
//! Big files are never read whole: the pane loads the last 512 KiB (a line
//! boundary away), renders only the visible lines (GPUI `list`), and "Load
//! earlier" reads more, up to [`MAX_VIEW_BYTES`].

mod load;
mod render;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::HeroicPaths;
use heronite_launch::log_reader::LogEntry;

use super::{DialogCtx, DialogEvent};
use crate::ui::dialog::{dialog, DialogProps};
use load::read_in_place;

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
        if let Some(n) = crate::env::parse::<u32>("HERONITE_DEBUG_LOG_EARLIER").filter(|_| read_in_place()) {
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
