//! Emulators page: the one place to manage emulators. Lists the configured
//! emulators with their health, the games using them and their ROM folders;
//! adds (catalog or custom), edits, duplicates and removes them; detects
//! emulators on this PC or in a folder; manages the ROM folders and scans them.
//!
//! Data: `local_library/emulators.json` and `rom-scanners.json` through
//! `heronite_emulation`. All reads and scans run off the UI thread.
//!
//! Debug: `HERONITE_DEBUG_EMU_PAGE=show[:<id>]|show-menu|add-menu|add|add:<definition>|edit|edit-profile|detect|folder|folder-edit|
//! remove|remove-folder|notice|pick-rom|launch-state` opens that state (use a fixture dir); see `debug.rs`.

mod actions;
mod console;
mod debug;
mod dialogs;
mod icon;
mod palette;
mod render;

use std::time::SystemTime;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::manager::{data_stamp, overview, EmulatorSummary};
use heronite_emulation::{AutoScanReport, SystemEnv};
use heronite_i18n::{tr, trn};

use super::{PageCtx, PageEvent};

/// A message under the toolbar (result of a scan, an add, a removal...).
#[derive(Clone)]
struct Notice {
    lines: Vec<String>,
    error: bool,
}

/// The menu open on the page.
#[derive(Clone, PartialEq, Eq)]
enum Menu {
    /// The ways to add an emulator, under "Add emulator".
    Add,
    /// Duplicate / Remove of the emulator shown.
    Emulator,
    /// Edit / Remove of this ROM folder.
    Folder(String),
}

pub struct EmulatorsPage {
    ctx: PageCtx,
    summaries: Vec<EmulatorSummary>,
    loaded: bool,
    stamp: Vec<Option<SystemTime>>,
    /// What is running in the background ("Scanning ROM folders...").
    busy: Option<SharedString>,
    notice: Option<Notice>,
    dialog: Option<AnyView>,
    dialog_sub: Option<Subscription>,
    /// Shown as the notice when the open confirmation succeeds.
    confirm_notice: Option<String>,
    /// Pending `HERONITE_DEBUG_EMU_PAGE` request, run by the first render after the load.
    debug: Option<String>,
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
    /// Palette command waiting for the first load (see `palette.rs`).
    palette_pending: Option<String>,
    /// Looks for changes made by others while the page is on screen.
    watch: Option<Task<()>>,
    /// The emulator shown (its id); `None` for "All emulators".
    selected: Option<String>,
    menu: Option<Menu>,
    /// Filters the list once it is long.
    search: Entity<InputState>,
    query: String,
}

impl EventEmitter<PageEvent> for EmulatorsPage {}

/// The notice for a finished scan of ROM folders.
fn scan_notice(report: &AutoScanReport) -> Notice {
    let mut lines = vec![tr!(
        "Scanned {folders}: added {games}.",
        folders = trn!(report.scanned, "{n} folder", "{n} folders"),
        games = trn!(report.imported, "{n} new game", "{n} new games")
    )];
    lines.extend(report.errors.iter().cloned());
    Notice { lines, error: !report.errors.is_empty() && report.imported == 0 }
}

impl EmulatorsPage {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = crate::ui::fields::new_input(window, cx, "", tr!("Search\u{2026}"));
        cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = state.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        })
        .detach();
        let this = Self {
            ctx,
            summaries: vec![],
            loaded: false,
            stamp: vec![],
            busy: None,
            notice: None,
            dialog: None,
            dialog_sub: None,
            confirm_notice: None,
            debug: crate::env::value("HERONITE_DEBUG_EMU_PAGE"),
            console_list: Default::default(),
            layer: Default::default(),
            palette_pending: None,
            watch: None,
            selected: None,
            menu: None,
            search,
            query: String::new(),
        };
        this.reload(cx);
        this
    }

    /// The Collection shows or hides the page. The Collection refresh and the
    /// scan dialogs write the same files: while the page is on screen, pick the
    /// changes up (a few stat calls every 2 seconds). Shown again, it looks at
    /// once, so what changed meanwhile appears without waiting.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if !shown {
            self.watch = None;
            return;
        }
        if self.watch.is_some() {
            return;
        }
        let paths = self.ctx.data_paths.clone();
        self.watch = Some(cx.spawn(async move |this, cx| loop {
            let p = paths.clone();
            let stamp = cx.background_spawn(async move { data_stamp(&p) }).await;
            let Ok(changed) = this.update(cx, |this, _| this.loaded && this.stamp != stamp) else { break };
            if changed && this.update(cx, |this, cx| this.reload(cx)).is_err() {
                break;
            }
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
        }));
    }

    fn reload(&self, cx: &mut Context<Self>) {
        let paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let (res, stamp) = cx
                .background_spawn(async move {
                    let stamp = data_stamp(&paths);
                    (overview(&paths, &SystemEnv), stamp)
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(s) => {
                        this.summaries = s;
                        this.stamp = stamp;
                        this.loaded = true;
                    }
                    Err(e) => this.notice = Some(Notice { lines: vec![format!("{e:#}")], error: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toast(&self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        cx.emit(PageEvent::Toast(text.into(), error));
    }

    fn emulators(&self) -> Vec<heronite_launch::emulators::UserEmulator> {
        self.summaries.iter().map(|s| s.emulator.clone()).collect()
    }

    fn total_folders(&self) -> usize {
        self.summaries.iter().map(|s| s.scanners.len()).sum()
    }

    /// Shows `id`'s emulator, or "All emulators" for `None`.
    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected = id;
        self.menu = None;
        cx.notify();
    }

    fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu.as_ref() == Some(&menu) { None } else { Some(menu) };
        cx.notify();
    }

    /// The emulator shown, while it is still there.
    fn selected_summary(&self) -> Option<EmulatorSummary> {
        let id = self.selected.as_deref()?;
        self.summaries.iter().find(|s| s.emulator.id == id).cloned()
    }
}

impl Render for EmulatorsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.debug.is_some() && self.loaded {
            self.run_debug(window, cx);
        }
        if self.loaded {
            if let Some(id) = self.palette_pending.take() {
                self.run_command(&id, window, cx);
            }
        }
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let page = self.render_desktop(window, cx);
        div().size_full().relative().child(page).children(self.dialog.clone()).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::scan_notice;
    use heronite_emulation::AutoScanReport;

    #[test]
    fn scan_notices() {
        let n = scan_notice(&AutoScanReport { scanned: 2, imported: 3, errors: vec![] });
        assert_eq!(n.lines, ["Scanned 2 folders: added 3 new games."]);
        assert!(!n.error);
        let n = scan_notice(&AutoScanReport { scanned: 1, imported: 0, errors: vec!["Gone: Folder does not exist: /x".into()] });
        assert!(n.error);
        assert_eq!(n.lines.len(), 2);
    }
}
