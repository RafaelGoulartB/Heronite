//! Emulators page: the one place to manage emulators. Lists the configured
//! emulators with their health, the games using them and their ROM folders;
//! adds (catalog or custom), edits, duplicates and removes them; detects
//! emulators on this PC or in a folder; manages the ROM folders and scans them.
//!
//! Data: `local_library/emulators.json` and `rom-scanners.json` through
//! `heronite_emulation`. All reads and scans run off the UI thread.
//!
//! Debug: `HERONITE_DEBUG_EMU_PAGE=add|add:<definition>|edit|edit-profile|detect|folder|folder-edit|
//! remove|remove-folder|notice|pick-rom|launch-state` opens that state (use a fixture dir); see `debug.rs`.

mod actions;
mod console;
mod debug;
mod dialogs;
mod palette;
mod render;

use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::manager::{data_stamp, overview, EmulatorSummary};
use heronite_emulation::{AutoScanReport, SystemEnv};
use heronite_i18n::{tr, trn};

use super::{content_width, page_close, sheet, PageCtx, PageEvent};
use crate::ui::controls::{button, ButtonKind};
use render::render_empty;

/// A message under the toolbar (result of a scan, an add, a removal...).
#[derive(Clone)]
struct Notice {
    lines: Vec<String>,
    error: bool,
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
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        };
        this.reload(cx);
        // The Collection refresh and the scan dialogs write the same files:
        // pick the changes up (a few stat calls every 2 seconds).
        let paths = this.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let p = paths.clone();
            let stamp = cx.background_spawn(async move { data_stamp(&p) }).await;
            let Ok(changed) = this.update(cx, |this, _| this.loaded && this.stamp != stamp) else { break };
            if changed && this.update(cx, |this, cx| this.reload(cx)).is_err() {
                break;
            }
        })
        .detach();
        this
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
        let folders = self.total_folders();

        let mut column = div().flex().flex_col().w_full().gap(px(14.)).child(self.render_toolbar(folders, cx));
        column = column.children(self.render_busy()).children(self.render_notice(cx));
        if self.loaded && self.summaries.is_empty() {
            column = column.child(render_empty());
        }
        // One card per emulator, in as many columns as fit (at most 3, and no
        // more than there are cards); the column grows with them.
        const CARD_MIN: f32 = 600.;
        const GAP: f32 = 14.;
        let fit = ((content_width(window) + GAP) / (CARD_MIN + GAP)).floor() as usize;
        let cols = fit.clamp(1, 3).min(self.summaries.len().max(1));
        let max_w = match cols {
            1 => 1000.,
            2 => 1640.,
            _ => 2280.,
        };
        let mut grid = div().grid().grid_cols(cols as u16).gap(px(GAP)).w_full();
        for s in self.summaries.clone() {
            grid = grid.child(self.render_card(&s, cx));
        }
        column = column.child(grid);

        let lead = if self.loaded {
            format!(
                "{} \u{b7} {}. {}",
                trn!(self.summaries.len(), "{n} emulator", "{n} emulators"),
                trn!(folders, "{n} ROM folder", "{n} ROM folders"),
                tr!("Folders set to scan on refresh add new games whenever the Collection refreshes.")
            )
        } else {
            tr!("Loading\u{2026}").to_string()
        };
        let add = button("emus-add", ButtonKind::Primary, tr!("Add emulator"))
            .on_click(cx.listener(|this, _, window, cx| this.open_add_emulator(window, cx)))
            .into_any_element();
        let close = page_close(cx);
        div()
            .size_full()
            .relative()
            .child(sheet("page-emulators", tr!("Emulators"), Some(lead.into()), Some(add), max_w, window, close, column))
            .children(self.dialog.clone())
            .into_any_element()
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
