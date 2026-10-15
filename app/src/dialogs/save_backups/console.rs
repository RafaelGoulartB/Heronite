//! Console mode of the Backups tab: the same steps as rows. Folders are the
//! ones of Settings → Backups (picking another one needs desktop mode).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::ludusavi::restore::FileKind;
use heronite_i18n::{tr, trn};

use super::render::{backup_line, group_facts, group_help, group_title, groups, overwrite_line, short_path, source_name, target_line};
use super::{Load, SaveBackupsView, Stage};
use crate::console::list::{Outcome, Row, Value};
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::pages::settings::format_time;

impl SaveBackupsView {
    fn console_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some((text, error)) = &self.notice {
            let r = Row::info("sb-notice", text.clone(), "");
            rows.push(if *error { r.danger() } else { r });
        }
        let l = match &self.load {
            Load::Idle | Load::Loading => {
                rows.push(Row::info("sb-loading", tr!("Looking for backups…"), ""));
                return rows;
            }
            Load::Failed(e) => {
                rows.push(Row::info("sb-error", e.clone(), "").danger());
                rows.push(Row::action("sb-retry", tr!("Try again")).icon(Icon::Refresh));
                return rows;
            }
            Load::Ready(l) => l,
        };
        let (line, error) = target_line(&l.target, self.installed);
        let target_row = Row::info("sb-target", line, "");
        rows.push(if error { target_row.danger() } else { target_row });
        let busy = self.busy();

        match &self.stage {
            Stage::List | Stage::Planning => {
                if !l.sources.is_empty() {
                    let options = l
                        .sources
                        .iter()
                        .map(|s| {
                            SharedString::from(format!(
                                "{} \u{b7} {}",
                                s.source.root.display(),
                                trn!(s.backups.len(), "{n} backup", "{n} backups")
                            ))
                        })
                        .collect();
                    let desc = l.sources.get(self.source).map(|s| source_name(s.source.kind)).unwrap_or_default();
                    rows.push(Row::choice("sb-folder", tr!("Backup folder"), options, self.source).desc(desc).enabled(!busy));
                }
                rows.push(Row::header(tr!("Backups")));
                match l.sources.get(self.source) {
                    None => rows.push(Row::info("sb-none", tr!("No backup folder yet. Set one in Settings → Backups."), "")),
                    Some(s) if s.backups.is_empty() => {
                        let text = s.error.clone().unwrap_or_else(|| tr!("No backup of this game in this folder.").into());
                        rows.push(Row::info("sb-none", text, ""));
                    }
                    Some(s) => {
                        let can = !busy && !l.target.missing_prefix();
                        for (i, b) in s.backups.iter().enumerate() {
                            let (when, facts) = backup_line(b);
                            let mut r = Row::action(SharedString::from(format!("sb-b-{i}")), when).icon(Icon::Replay).enabled(can);
                            let desc = if facts.is_empty() { tr!("A restores it").to_string() } else { facts };
                            r = r.desc(desc);
                            rows.push(r);
                        }
                        rows.push(
                            Row::toggle("sb-lock", tr!("Lock the newest backup"), s.backups[0].locked)
                                .enabled(!busy)
                                .desc(tr!("Locked backups are never removed")),
                        );
                    }
                }
                rows.push(Row::header(""));
                if matches!(self.stage, Stage::Planning) {
                    rows.push(Row::info("sb-planning", tr!("Reading the backup…"), ""));
                }
                rows.push(
                    Row::action("sb-backup-now", if self.working { tr!("Working…") } else { tr!("Back up now") })
                        .icon(Icon::Save)
                        .enabled(!busy),
                );
                if let Some(u) = &l.undo {
                    rows.push(
                        Row::action("sb-undo", tr!("Undo the last restore"))
                            .icon(Icon::History)
                            .enabled(!busy)
                            .desc(tr!("Last restore: {when}. A copy of what it replaced is kept.", when = format_time(&u.created_at))),
                    );
                }
                rows.push(Row::action("sb-refresh", tr!("Refresh")).icon(Icon::Refresh).enabled(!busy));
            }
            Stage::Plan(plan) => {
                rows.push(Row::header(tr!("Files to restore")));
                for g in groups(plan, &l.target) {
                    let mut desc = group_facts(&g);
                    if let Some((dir, _)) = g.folders.first() {
                        desc = format!("{desc} \u{b7} {dir}");
                    }
                    if let Some(help) = group_help(g.kind) {
                        desc = format!("{desc} \u{2014} {help}");
                    }
                    rows.push(
                        Row::toggle(SharedString::from(format!("sb-g-{}", g.kind as u8)), group_title(g.kind), g.selected)
                            .enabled(g.kind != FileKind::Unmapped)
                            .desc(desc),
                    );
                }
                rows.push(Row::header(""));
                let selected = plan.selected().count();
                rows.push(
                    Row::action("sb-confirm", trn!(selected, "Restore {n} file", "Restore {n} files"))
                        .icon(Icon::Replay)
                        .enabled(selected > 0)
                        .desc(overwrite_line(plan)),
                );
                rows.push(Row::action("sb-back", tr!("Back")).icon(Icon::ChevronLeft));
            }
            Stage::Restoring => rows.push(Row::info("sb-restoring", tr!("Restoring the save…"), "")),
            Stage::Done(outcome) => {
                rows.push(Row::info("sb-written", trn!(outcome.written, "Restored {n} file.", "Restored {n} files."), ""));
                if outcome.unchanged > 0 {
                    rows.push(Row::info("sb-same", trn!(outcome.unchanged, "{n} was already the same.", "{n} were already the same."), ""));
                }
                for (i, (path, error)) in outcome.failed.iter().enumerate() {
                    rows.push(Row::info(SharedString::from(format!("sb-fail-{i}")), short_path(path, &l.target), error.clone()).danger());
                }
                rows.push(Row::action("sb-undo", tr!("Undo")).icon(Icon::History).enabled(outcome.undo.is_some() && !busy));
                rows.push(Row::action("sb-back", tr!("Done")).icon(Icon::Check));
            }
        }
        rows
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows();
        self.console_list.render("console-save-backups", &rows, true).size_full().into_any_element()
    }

    pub fn console_reset(&mut self) {
        self.console_list.reset();
    }

    /// A pad press for the tab; `false` when it did nothing.
    pub fn on_pad(&mut self, pad: Pad, cx: &mut Context<Self>) -> bool {
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "sb-folder" => self.pick_source(index, cx),
            Outcome::Changed(id, Value::Toggle(_)) => match id.as_ref() {
                "sb-lock" => self.toggle_lock(0, cx),
                id => {
                    let kind = id.strip_prefix("sb-g-").and_then(|n| n.parse::<u8>().ok()).and_then(|n| {
                        [FileKind::Save, FileKind::Registry, FileKind::Outside, FileKind::Unmapped].into_iter().find(|k| *k as u8 == n)
                    });
                    if let Some(kind) = kind {
                        self.toggle_kind(kind, cx);
                    }
                }
            },
            Outcome::Activate(id) => match id.as_ref() {
                "sb-retry" | "sb-refresh" => self.reload(cx),
                "sb-backup-now" => self.backup_now(cx),
                "sb-undo" => self.undo_last(cx),
                "sb-confirm" => self.confirm_restore(cx),
                "sb-back" => self.back_to_list(cx),
                id => {
                    if let Some(i) = id.strip_prefix("sb-b-").and_then(|n| n.parse::<usize>().ok()) {
                        self.pick_backup(i, cx);
                        self.start_restore(cx);
                    }
                }
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    pub fn hints(&self) -> Vec<Hint> {
        vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change"))]
    }
}
