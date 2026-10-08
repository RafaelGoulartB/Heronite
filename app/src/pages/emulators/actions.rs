//! Background actions of the Emulators page: duplicate an emulator and scan
//! ROM folders. They run off the UI thread and finish with a notice.

use gpui_kit::prelude::*;
use heronite_emulation::{duplicate_user_emulator, run_rom_scanners, AutoScanReport};

use super::{scan_notice, EmulatorsPage, Notice};
use crate::pages::PageEvent;

impl EmulatorsPage {
    pub(super) fn duplicate(&mut self, id: &str, cx: &mut Context<Self>) {
        let (paths, id) = (self.ctx.heroic.clone(), id.to_string());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { duplicate_user_emulator(&paths, &id) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(e) => this.notice = Some(Notice { lines: vec![format!("Created {}.", e.name)], error: false }),
                    Err(e) => this.notice = Some(Notice { lines: vec![format!("{e:#}")], error: true }),
                }
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Scans the given saved folders now (even those with "scan on refresh"
    /// off) and imports the games not imported yet.
    pub(super) fn scan(&mut self, ids: Vec<String>, label: &'static str, cx: &mut Context<Self>) {
        if self.busy.is_some() || ids.is_empty() {
            return;
        }
        self.busy = Some(label.into());
        self.notice = None;
        let paths = self.ctx.heroic.clone();
        cx.spawn(async move |this, cx| {
            let report = cx
                .background_spawn(async move {
                    let mut total = AutoScanReport::default();
                    for id in ids {
                        match run_rom_scanners(&paths, Some(&id)) {
                            Ok(r) => {
                                total.scanned += r.scanned;
                                total.imported += r.imported;
                                total.errors.extend(r.errors);
                            }
                            Err(e) => total.errors.push(format!("{e:#}")),
                        }
                    }
                    total
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                let notice = scan_notice(&report);
                if report.imported > 0 {
                    this.toast(format!("Added {} new game(s)", report.imported), false, cx);
                    cx.emit(PageEvent::LibraryChanged);
                }
                this.notice = Some(notice);
                this.reload(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn scan_all(&mut self, cx: &mut Context<Self>) {
        let ids = self.summaries.iter().flat_map(|s| s.scanners.iter().map(|x| x.id.clone())).collect();
        self.scan(ids, "Scanning all ROM folders\u{2026}", cx);
    }
}
