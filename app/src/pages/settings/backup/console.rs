//! Console mode rows of Settings → Backups (under Ludusavi's). A backup is
//! restored whole: A on it asks, A again restores.

use gpui_kit::*;
use heronite_backup::Part;
use heronite_data::settings::{BackupPatch, DEFAULT_BACKUP_KEEP};
use heronite_i18n::tr;

use super::restore::{self, Confirm};
use super::{format_time, interval_label, keep_label, part_label, persist, set_part, SettingsPage, INTERVALS, KEEP};
use crate::console::list::Row;
use crate::icons::Icon;

pub(in crate::pages::settings) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<Row> {
    let b = &d.backup;
    let s = &d.settings.backup;
    let ii = INTERVALS.iter().position(|(v, _)| *v == s.interval).unwrap_or(1);
    let keep = s.keep.unwrap_or(DEFAULT_BACKUP_KEEP);
    let ki = KEEP.iter().position(|n| *n == keep).unwrap_or(2);
    let running = super::running(cx);
    let mut rows = vec![
        Row::header(tr!("Data folder backup")),
        Row::text("bk-folder", tr!("Backup folder"), b.folder.read(cx).value().to_string()).icon(Icon::FolderOpen),
        Row::choice("bk-interval", tr!("How often"), INTERVALS.iter().map(|(v, _)| SharedString::from(interval_label(v))).collect(), ii),
        Row::choice("bk-keep", tr!("Backups kept"), KEEP.iter().map(|n| SharedString::from(keep_label(*n))).collect(), ki),
    ];
    for part in [Part::Settings, Part::Art, Part::Logins] {
        rows.push(Row::toggle(format!("bk-part-{}", part.key()), part_label(part), super::included(d, part)));
    }
    rows.push(
        Row::action("bk-now", if running { tr!("Backing up\u{2026}") } else { tr!("Back up now") })
            .icon(Icon::Save)
            .enabled(!running && !s.folder.trim().is_empty())
            .desc(super::last_line(d)),
    );
    if let Some((text, error)) = super::outcome_line(cx) {
        let row = Row::info("bk-message", text, "");
        rows.push(if error { row.danger() } else { row });
    }

    rows.push(Row::header(tr!("Restore")));
    let r = &b.restore;
    for (i, info) in r.all().enumerate().take(20) {
        let asking = r.confirm() == Some(&Confirm::Restore(info.path.clone()));
        let desc = if asking {
            tr!("A again backs up what you have now, restarts and puts this backup back whole.").to_string()
        } else {
            format!("{} \u{b7} {}", restore::reason_label(info), restore::summary_line(info))
        };
        let row = Row::action(format!("bk-restore-{i}"), format_time(&info.created_at)).icon(Icon::History).enabled(!r.busy()).desc(desc);
        rows.push(if asking { row.danger() } else { row });
    }
    if r.all().next().is_none() {
        rows.push(Row::info("bk-none", tr!("No backups in this folder yet."), ""));
    }
    rows
}

pub(in crate::pages::settings) fn console_apply(
    d: &mut SettingsPage,
    outcome: crate::console::list::Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) {
    use crate::console::list::{Outcome, Value};
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "bk-folder" => {
                let start = Some(std::path::PathBuf::from(d.backup.folder.read(cx).value().to_string()));
                d.console_browse(
                    tr!("Backup folder"),
                    crate::console::browser::PickKind::Folder,
                    start,
                    window,
                    cx,
                    |d, path, window, cx| {
                        d.backup.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                        persist(d, BackupPatch { folder: Some(path), ..Default::default() }, cx);
                    },
                );
            }
            "bk-now" => super::back_up_now(d, cx),
            other => {
                let Some(i) = other.strip_prefix("bk-restore-").and_then(|n| n.parse::<usize>().ok()) else { return };
                let Some(info) = d.backup.restore.all().nth(i).cloned() else { return };
                if d.backup.restore.confirm() == Some(&Confirm::Restore(info.path.clone())) {
                    restore::restore(d, info.path.clone(), info.parts.clone(), cx);
                } else {
                    restore::select(d, &info);
                    d.backup.restore.ask_restore(info.path);
                }
            }
        },
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "bk-interval" => {
            persist(d, BackupPatch { interval: Some(INTERVALS[index].0.into()), ..Default::default() }, cx);
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "bk-keep" => {
            persist(d, BackupPatch { keep: Some(KEEP[index]), ..Default::default() }, cx);
        }
        Outcome::Changed(id, Value::Toggle(on)) => {
            if let Some(part) = id.strip_prefix("bk-part-").and_then(Part::parse) {
                set_part(d, part, on, cx);
            }
        }
        _ => {}
    }
}
