//! Settings → Backups: the backup of the data folder and the app settings
//! (`heronite_backup`): where, how often, how many are kept and what goes in;
//! "Back up now" (run by `services::upkeep`, which runs the schedule too);
//! then the backups of the folder and restoring one (`restore.rs`).

mod console;
mod restore;

pub(super) use console::{console_apply, console_rows};

use chrono::{DateTime, Datelike, Local, Utc};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_backup::run::{Outcome, Skipped};
use heronite_backup::usage::Usage;
use heronite_backup::{Left, Part};
use heronite_data::maintenance::human_size;
use heronite_data::settings::{self, BackupPatch, EffectiveSettings, DEFAULT_BACKUP_KEEP};
use heronite_i18n::{n_, tr, trn};

use super::SettingsPage;
use crate::pages::{status_pill, PageCtx};
use crate::theme;
use crate::ui::controls::{action_button, caption, card_box, row, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, meta_text, new_input, on_commit, path_field, select_field};

const INTERVALS: [(&str, &str); 4] = [
    ("daily", n_!("Once a day")),
    ("weekly", n_!("Once a week")),
    ("monthly", n_!("Once a month")),
    ("never", n_!("Only when I click Back up now")),
];
/// Backups kept; 0 = all.
const KEEP: [u32; 6] = [3, 5, 10, 20, 50, 0];

pub struct State {
    folder: Entity<InputState>,
    interval_open: bool,
    keep_open: bool,
    /// Size of each part and of what is left out (measured when shown).
    usage: Option<Usage>,
    /// Whether the upkeep service was backing up at its last change.
    was_running: bool,
    restore: restore::State,
}

impl State {
    pub fn new(_ctx: &PageCtx, s: &EffectiveSettings, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let folder = new_input(window, cx, &s.backup.folder, "");
        on_commit(&folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.backup.folder {
                persist(d, BackupPatch { folder: Some(value), ..Default::default() }, cx);
            }
        });
        if let Some(upkeep) = crate::services::upkeep::get(cx) {
            cx.observe(&upkeep, |d, upkeep, cx| {
                let running = upkeep.read(cx).running();
                if d.backup.was_running && !running {
                    after_backup(d, cx);
                }
                d.backup.was_running = running;
                cx.notify();
            })
            .detach();
        }
        Self { folder, interval_open: false, keep_open: false, usage: None, was_running: false, restore: Default::default() }
    }
}

/// The section came on screen: sizes and the list of backups.
pub(super) fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let (data, app) = (d.ctx.data_paths.clone(), d.ctx.app.clone());
    cx.spawn(async move |this, cx| {
        let usage = cx.background_spawn(async move { heronite_backup::usage::measure(&data, &app) }).await;
        this.update(cx, |d, cx| {
            d.backup.usage = Some(usage);
            cx.notify();
        })
        .ok();
    })
    .detach();
    restore::reload(d, cx);
}

/// `HERONITE_DEBUG_SETTINGS_ACTION=backup-now|backup-open:<n>|backup-confirm:<n>`.
pub(super) fn debug_action(d: &mut SettingsPage, action: &str, n: usize, cx: &mut Context<SettingsPage>) {
    if action == "backup-now" {
        return back_up_now(d, cx);
    }
    let Some(info) = d.backup.restore.all().nth(n).cloned() else { return };
    restore::select(d, &info);
    if action == "backup-confirm" {
        d.backup.restore.ask_restore(info.path);
    }
    cx.notify();
}

/// Local, medium date + short time (`Intl.DateTimeFormat` default look).
pub fn format_time(iso: &str) -> String {
    match DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => {
            let local = dt.with_timezone(&Utc).with_timezone(&Local);
            match heronite_i18n::current() {
                heronite_i18n::Lang::En => local.format("%b %-d, %Y, %-I:%M %p").to_string(),
                heronite_i18n::Lang::PtBr | heronite_i18n::Lang::Es => {
                    format!(
                        "{} {} {}, {}",
                        local.day(),
                        heronite_i18n::month_short(local.month()).to_lowercase(),
                        local.year(),
                        local.format("%H:%M")
                    )
                }
            }
        }
        Err(_) => iso.to_string(),
    }
}

fn interval_label(value: &str) -> &'static str {
    INTERVALS.iter().find(|(v, _)| *v == value).map(|(_, l)| tr(l)).unwrap_or(tr!("Once a week"))
}

fn keep_label(n: u32) -> String {
    match n {
        0 => tr!("All of them").into(),
        n => trn!(n, "The last backup", "The last {n} backups"),
    }
}

pub(super) fn part_label(part: Part) -> &'static str {
    match part {
        Part::Collection => tr!("Collection"),
        Part::Settings => tr!("App settings"),
        Part::Art => tr!("Your artwork"),
        Part::Logins => tr!("Store sign-ins"),
    }
}

fn part_desc(part: Part) -> &'static str {
    match part {
        Part::Collection => tr!("Games, statuses, play sessions, journal, ratings, game settings and the GOG, Amazon and Steam libraries."),
        Part::Settings => tr!("View presets, appearance, overlay and screenshot options."),
        Part::Art => tr!("Covers and backgrounds you picked."),
        Part::Logins => tr!("Epic, GOG and Amazon Games sign-ins. Only your user can open the backups."),
    }
}

fn left_label(left: Left) -> &'static str {
    match left {
        Left::Tools => tr!("Wine and Proton builds"),
        Left::Images => tr!("image caches"),
        Left::Caches => tr!("store caches"),
        Left::Logs => tr!("logs"),
        Left::Temp => tr!("temporary files"),
        Left::Sandbox => tr!("the sandbox's home folders"),
    }
}

/// Writes the backup settings through `heronite_data`.
fn persist(d: &mut SettingsPage, patch: BackupPatch, cx: &mut Context<SettingsPage>) {
    let folder_changed = patch.folder.as_ref().is_some_and(|f| f.trim() != d.settings.backup.folder);
    let res = settings::save_backup_settings(&d.ctx.data_paths, patch);
    d.saved(res, cx);
    if folder_changed {
        restore::reload(d, cx);
    }
}

fn set_part(d: &mut SettingsPage, part: Part, on: bool, cx: &mut Context<SettingsPage>) {
    let mut exclude: Vec<String> = d.settings.backup.exclude.iter().filter(|e| *e != part.key()).cloned().collect();
    if !on {
        exclude.push(part.key().into());
    }
    persist(d, BackupPatch { exclude: Some(exclude), ..Default::default() }, cx);
}

fn included(d: &SettingsPage, part: Part) -> bool {
    !d.settings.backup.exclude.iter().any(|e| e == part.key())
}

fn back_up_now(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let folder = d.backup.folder.read(cx).value().trim().to_string();
    if folder != d.settings.backup.folder {
        persist(d, BackupPatch { folder: Some(folder), ..Default::default() }, cx);
    }
    if let Some(upkeep) = crate::services::upkeep::get(cx) {
        upkeep.update(cx, |u, cx| u.back_up_now(cx));
    }
}

/// A backup ended: the "last backup" line and the list change.
fn after_backup(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let paths = d.ctx.data_paths.clone();
    cx.spawn(async move |this, cx| {
        let fresh = cx.background_spawn(async move { settings::load(&paths) }).await;
        this.update(cx, |d, cx| {
            if let Ok(s) = fresh {
                d.settings = s;
            }
            restore::reload(d, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn running(cx: &App) -> bool {
    crate::services::upkeep::get(cx).is_some_and(|u| u.read(cx).running())
}

/// The outcome of the last backup this process made, in words (`true` = error).
fn outcome_line(cx: &App) -> Option<(String, bool)> {
    let upkeep = crate::services::upkeep::get(cx)?;
    let o: Outcome = upkeep.read(cx).last()?.clone();
    if let Some(e) = o.error {
        return Some((e, true));
    }
    if o.skipped == Some(Skipped::AlreadyRunning) {
        return Some((tr!("A backup is already being made.").into(), false));
    }
    let (path, summary) = (o.path?, o.summary?);
    let mut text = tr!("Backup saved to {path}. It took {size} of new space.", path = path.display(), size = human_size(summary.new_bytes));
    if o.pruned > 0 {
        text.push(' ');
        text.push_str(&trn!(o.pruned, "The oldest backup was removed.", "The {n} oldest backups were removed."));
    }
    Some((text, false))
}

fn last_line(d: &SettingsPage) -> String {
    let last = match &d.settings.backup.last_backup_at {
        Some(at) => format_time(at),
        None => tr!("Never").into(),
    };
    tr!("Last backup: {last}", last)
}

fn render_setup(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let b = &d.backup;
    let s = &d.settings.backup;
    let has = !b.folder.read(cx).value().trim().is_empty();
    let folder = path_field(
        "backup-folder-pick",
        Some(tr!("Backup folder")),
        &b.folder,
        has,
        false,
        cx.listener(move |d, _, window, cx| {
            if has {
                d.backup.folder.update(cx, |s, cx| s.set_value("", window, cx));
                persist(d, BackupPatch { folder: Some(String::new()), ..Default::default() }, cx);
            } else {
                choose_path(window, cx, true, tr!("Backup folder"), |d: &mut SettingsPage, p, window, cx| {
                    d.backup.folder.update(cx, |s, cx| s.set_value(p.clone(), window, cx));
                    persist(d, BackupPatch { folder: Some(p), ..Default::default() }, cx);
                });
            }
        }),
    );
    let interval = select_field(
        "backup-interval",
        Some(tr!("How often")),
        interval_label(&s.interval),
        false,
        b.interval_open,
        INTERVALS.iter().map(|(v, l)| (tr(l).into(), *v == s.interval)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.backup.interval_open = !d.backup.interval_open;
            d.backup.keep_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.backup.interval_open = false;
            persist(d, BackupPatch { interval: Some(INTERVALS[i].0.into()), ..Default::default() }, cx);
        },
    );
    let keep_now = s.keep.unwrap_or(DEFAULT_BACKUP_KEEP);
    let keep = select_field(
        "backup-keep",
        Some(tr!("Backups kept")),
        keep_label(keep_now),
        false,
        b.keep_open,
        KEEP.iter().map(|n| (keep_label(*n).into(), *n == keep_now)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.backup.keep_open = !d.backup.keep_open;
            d.backup.interval_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.backup.keep_open = false;
            persist(d, BackupPatch { keep: Some(KEEP[i]), ..Default::default() }, cx);
        },
    );
    card_box().py(px(12.)).gap(px(12.)).child(folder).child(interval).child(keep)
}

fn render_parts(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let usage = d.backup.usage.clone();
    let desc = |part: Part| match &usage {
        Some(u) => format!("{} \u{b7} {}", part_desc(part), human_size(u.part_bytes(part))),
        None => part_desc(part).to_string(),
    };
    let collection = row(part_label(Part::Collection), Some(&desc(Part::Collection)), status_pill(tr!("Always"), theme::accent()));
    let mut card = card_box().child(collection);
    for part in [Part::Settings, Part::Art, Part::Logins] {
        let on = included(d, part);
        card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(
            toggle_row(SharedString::from(format!("backup-part-{}", part.key())), part_label(part), Some(&desc(part)), on)
                .on_click(cx.listener(move |d, _, _, cx| set_part(d, part, !on, cx))),
        );
    }
    let left = usage.map(|u| {
        let items: Vec<String> = Left::ALL
            .iter()
            .filter_map(|l| u.left.get(l).filter(|b| **b > 0).map(|b| format!("{} {}", left_label(*l), human_size(*b))))
            .collect();
        tr!("Left out, since they are downloaded or rebuilt again: {list}.", list = items.join(", "))
    });
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("What goes in a backup")))
        .child(card)
        .children(left.map(|text| div().mt(px(8.)).mx(px(6.)).child(meta_text(text))))
        .child(
            div()
                .mt(px(6.))
                .mx(px(6.))
                .child(meta_text(tr!("Game saves are backed up by Ludusavi (Settings → Saves). Installed games are not backed up."))),
        )
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let setup = render_setup(d, cx);
    let parts = render_parts(d, cx);
    let busy = running(cx);
    let has = !d.backup.folder.read(cx).value().trim().is_empty();
    let stored_error = d.settings.backup.last_error.clone();
    let outcome = outcome_line(cx).or(stored_error.map(|e| (e, true)));
    let action = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .mt(px(12.))
        .child(action_button(
            "backup-now",
            ButtonKind::Primary,
            if busy { tr!("Backing up…") } else { tr!("Back up now") },
            !busy && has,
            cx.listener(|d, _, _, cx| back_up_now(d, cx)),
        ))
        .child(meta_text(last_line(d)));
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Data folder backup")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Each backup is a dated folder you can open and browse. Files that did not change are shared with the previous backup, so a new one only takes the space of what changed. Scheduled backups run while Heronite is open, never during a game."
        ))))
        .child(setup)
        .child(action)
        .children(outcome.map(|(text, error)| {
            div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
        }))
        .child(parts)
        .child(restore::render(d, window, cx))
        .into_any_element()
}
