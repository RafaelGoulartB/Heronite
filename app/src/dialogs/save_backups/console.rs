//! Console mode of the Saves tab: the same steps as rows. Folders are the
//! ones of Settings → Backups (picking another one needs desktop mode).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::ludusavi::restore::FileKind;
use heronite_i18n::{tr, trn};

use super::render::{backup_line, group_facts, group_help, group_title, overwrite_line, short_path, source_name, target_line};
use super::{BackupKey, Load, SaveBackupsView, Stage};
use crate::console::list::{Outcome, Row, Value};
use crate::console::osk::{Osk, OskEvent};
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
                        if let Some(b) = self
                            .console_backup
                            .as_ref()
                            .filter(|k| k.root == s.source.root)
                            .and_then(|k| s.backups.iter().find(|b| k.is(b)))
                        {
                            // One backup's actions.
                            let (title, facts) = backup_line(b);
                            rows.push(Row::info("sb-m-title", title, "").desc(facts));
                            rows.push(
                                Row::action("sb-m-restore", tr!("Restore…"))
                                    .icon(Icon::Replay)
                                    .enabled(!busy && !l.target.missing_prefix()),
                            );
                            rows.push(
                                Row::text("sb-m-name", tr!("Name"), b.comment.clone().unwrap_or_default()).icon(Icon::Edit).enabled(!busy),
                            );
                            rows.push(
                                Row::toggle("sb-m-lock", tr!("Lock"), b.locked)
                                    .enabled(!busy)
                                    .desc(tr!("Locked backups are never removed")),
                            );
                            rows.push(if self.confirm_delete == self.console_backup {
                                Row::action("sb-m-delete-yes", tr!("Press A again to delete it. This cannot be undone."))
                                    .icon(Icon::DeleteForever)
                                    .danger()
                            } else {
                                Row::action("sb-m-delete", tr!("Delete")).icon(Icon::DeleteOutline).enabled(!busy)
                            });
                            rows.push(Row::action("sb-m-back", tr!("Back")).icon(Icon::ChevronLeft));
                            return rows;
                        }
                        for (i, b) in s.backups.iter().enumerate() {
                            let (title, facts) = backup_line(b);
                            rows.push(Row::action(SharedString::from(format!("sb-b-{i}")), title).icon(Icon::History).desc(facts));
                        }
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
            Stage::Plan(preview) => {
                let plan = &preview.plan;
                rows.push(Row::header(tr!("Files to restore")));
                for g in &preview.groups {
                    let mut desc = group_facts(g);
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
        let active = !self.layer.is_open();
        div()
            .size_full()
            .child(self.console_list.render("console-save-backups", &rows, active).size_full())
            .children(self.layer.view())
            .into_any_element()
    }

    fn console_open_backup(&mut self, key: Option<BackupKey>) {
        self.console_backup = key;
        self.confirm_delete = None;
        self.console_list.reset();
    }

    /// The on-screen keyboard for the name of the backup shown.
    fn console_edit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.console_backup.clone() else { return };
        let current = self.backup_name(&key);
        let osk = cx.new(|_| Osk::new(tr!("Name this backup"), &current));
        let sub = cx.subscribe_in(&osk, window, move |v, _, ev: &OskEvent, _, cx| {
            v.layer.close();
            if let OskEvent::Done(text) = ev {
                v.set_backup_name(key.clone(), text.trim().to_string(), cx);
            }
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    pub fn console_reset(&mut self) {
        self.console_list.reset();
    }

    /// Whether the tab takes `pad` before the dialog does: everything while
    /// the keyboard is open, B while a backup's actions are shown.
    pub fn captures(&self, pad: Pad) -> bool {
        self.layer.is_open() || (pad == Pad::B && self.console_backup.is_some())
    }

    /// A pad press for the tab; `false` when it did nothing.
    pub fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if pad == Pad::B && self.console_backup.is_some() {
            self.console_open_backup(None);
            cx.notify();
            return true;
        }
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "sb-folder" => self.pick_source(index, cx),
            Outcome::Changed(id, Value::Toggle(_)) => match id.as_ref() {
                "sb-m-lock" => {
                    if let Some(key) = self.console_backup.clone() {
                        self.toggle_lock(key, cx);
                    }
                }
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
                "sb-m-back" => self.console_open_backup(None),
                "sb-m-restore" => {
                    if let Some(key) = self.console_backup.clone() {
                        self.console_open_backup(None);
                        self.pick_backup(key, cx);
                        self.start_restore(cx);
                    }
                }
                "sb-m-name" => self.console_edit_name(window, cx),
                "sb-m-delete" => self.confirm_delete = self.console_backup.clone(),
                "sb-m-delete-yes" => {
                    self.delete_confirmed(cx);
                    self.console_list.reset();
                }
                id => {
                    // The row number is read against the rows just built, then
                    // the backup is kept by key.
                    let key = id.strip_prefix("sb-b-").and_then(|n| n.parse::<usize>().ok()).and_then(|i| {
                        let src = self.current_source()?;
                        Some(BackupKey::of(src, src.backups.get(i)?))
                    });
                    if let Some(key) = key {
                        self.pick_backup(key.clone(), cx);
                        self.console_open_backup(Some(key));
                    }
                }
            },
            Outcome::Ignored => return false,
            _ => {}
        }
        cx.notify();
        true
    }

    pub fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        vec![hint(Btn::A, tr!("Select")), hint(Btn::LeftRight, tr!("Change"))]
    }
}
