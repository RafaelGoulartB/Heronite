//! Settings → Backups, second card: scheduled copy of the Heroic config folder.

use chrono::{DateTime, Local, Utc};
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::settings::{self, EffectiveSettings};
use heronite_emulation::backup::run_collection_backup;

use super::SettingsPage;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card_box, ButtonKind};
use crate::ui::fields::{choose_path, meta_text, new_input, on_commit, path_field, select_field};

const INTERVALS: [(&str, &str); 3] = [("daily", "Once a day"), ("weekly", "Once a week"), ("monthly", "Once a month")];

pub struct State {
    folder: Entity<InputState>,
    interval: String,
    interval_open: bool,
    running: bool,
    message: String,
    error: String,
}

impl State {
    pub fn new(
        _ctx: &PageCtx,
        s: &EffectiveSettings,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Self {
        let folder = new_input(window, cx, &s.backup.folder, "");
        on_commit(&folder, window, cx, |d: &mut SettingsPage, value, _, cx| {
            if value != d.settings.backup.folder {
                persist(d, Some(value), None, cx);
            }
        });
        let interval = if INTERVALS.iter().any(|(v, _)| *v == s.backup.interval) { s.backup.interval.clone() } else { "weekly".into() };
        Self { folder, interval, interval_open: false, running: false, message: String::new(), error: String::new() }
    }
}

/// Local, medium date + short time (`Intl.DateTimeFormat` default look).
pub fn format_time(iso: &str) -> String {
    match DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => dt.with_timezone(&Utc).with_timezone(&Local).format("%b %-d, %Y, %-I:%M %p").to_string(),
        Err(_) => iso.to_string(),
    }
}

/// Writes folder / interval (whichever changed) through `heronite_data`.
fn persist(d: &mut SettingsPage, folder: Option<String>, interval: Option<String>, cx: &mut Context<SettingsPage>) {
    let folder = folder.unwrap_or_else(|| d.settings.backup.folder.clone());
    let interval = interval.unwrap_or_else(|| d.backup.interval.clone());
    let res = settings::save_backup(&d.ctx.heroic, &folder, &interval);
    d.saved(res, cx);
}

fn backup_now(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let folder = d.backup.folder.read(cx).value().trim().to_string();
    if folder != d.settings.backup.folder {
        persist(d, Some(folder), None, cx);
    }
    let _ = window;
    d.backup.running = true;
    d.backup.error.clear();
    d.backup.message.clear();
    let paths = d.ctx.heroic.clone();
    cx.notify();
    cx.spawn(async move |this, cx| {
        let p = paths.clone();
        let result = cx.background_spawn(async move { run_collection_backup(&p, true, Local::now()) }).await;
        let fresh = cx.background_spawn(async move { settings::load(&paths) }).await;
        this.update(cx, |d, cx| {
            d.backup.running = false;
            if let Some(e) = result.error {
                d.backup.error = e;
            } else if let (true, Some(path)) = (result.ran, result.path) {
                d.backup.message = format!("Backup saved to {path}");
            } else {
                d.backup.message = "Backup is already up to date.".into();
            }
            d.saved(fresh, cx);
        })
        .ok();
    })
    .detach();
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let b = &d.backup;
    let folder_value = b.folder.read(cx).value().trim().to_string();
    let interval_label = INTERVALS.iter().find(|(v, _)| *v == b.interval).map(|(_, l)| *l).unwrap_or("Once a week");
    let last = match &d.settings.backup.last_backup_at {
        Some(at) => format_time(at),
        None => "Never".into(),
    };
    let last_line = match &d.settings.backup.last_backup_path {
        Some(p) => format!("Last backup: {last} · {p}"),
        None => format!("Last backup: {last}"),
    };
    let running = b.running;
    let message = b.message.clone();
    let error = b.error.clone();
    let stored_error = d.settings.backup.last_error.clone().filter(|_| message.is_empty() && error.is_empty());
    let options: Vec<(SharedString, bool)> =
        INTERVALS.iter().map(|(v, l)| ((*l).into(), *v == b.interval)).collect();

    let has = !folder_value.is_empty();
    let folder = path_field(
        "backup-folder-pick",
        Some("Backup folder"),
        &b.folder,
        has,
        false,
        cx.listener(move |d, _, window, cx| {
            if has {
                d.backup.folder.update(cx, |s, cx| s.set_value("", window, cx));
                persist(d, Some(String::new()), None, cx);
            } else {
                choose_path(window, cx, true, "Backup folder", |d: &mut SettingsPage, p, window, cx| {
                    d.backup.folder.update(cx, |s, cx| s.set_value(p.clone(), window, cx));
                    persist(d, Some(p), None, cx);
                });
            }
        }),
    );
    let interval = select_field(
        "backup-interval",
        Some("How often"),
        interval_label,
        running,
        b.interval_open,
        options,
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.backup.interval_open = !d.backup.interval_open;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.backup.interval_open = false;
            d.backup.interval = INTERVALS[i].0.to_string();
            persist(d, None, Some(INTERVALS[i].0.to_string()), cx);
        },
    );

    let setup = card_box().py(px(12.)).gap(px(12.)).child(folder).child(interval);
    let action = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .mt(px(12.))
        .child(action_button(
            "backup-now",
            ButtonKind::Primary,
            if running { "Backing up…" } else { "Backup now" },
            !running && has,
            cx.listener(|d, _, window, cx| backup_now(d, window, cx)),
        ))
        .child(meta_text(last_line));
    div()
        .flex()
        .flex_col()
        .child(caption("Heroic config backup"))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(
            "Copies Heroic config (Collection, playtime, statuses, Local games, logins) into the folder below. Installed games are not included. Cache and Wine tools are skipped. A dated folder is created when Heroic Local opens, if the schedule is due.",
        )))
        .child(setup)
        .child(action)
        .children((!message.is_empty()).then(|| div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(theme::success()).child(message)))
        .children((!error.is_empty()).then(|| div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(theme::danger_text()).child(error)))
        .children(stored_error.map(|e| div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(theme::danger_text()).child(e)))
        .into_any_element()
}

/// Console mode rows of the Collection backup (under Ludusavi's).
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    use crate::icons::Icon;
    let b = &d.backup;
    let ii = INTERVALS.iter().position(|(v, _)| *v == b.interval).unwrap_or(1);
    let last = match &d.settings.backup.last_backup_at {
        Some(at) => format!("Last backup: {}", format_time(at)),
        None => "No backup yet".into(),
    };
    let mut rows = vec![
        Row::header("Collection backup"),
        Row::text("bk-folder", "Backup folder", b.folder.read(cx).value().to_string()).icon(Icon::FolderOpen),
        Row::choice("bk-interval", "How often", INTERVALS.iter().map(|(_, l)| SharedString::from(*l)).collect(), ii),
        Row::action("bk-now", if b.running { "Backing up\u{2026}" } else { "Back up now" }).icon(Icon::Save).enabled(!b.running).desc(last),
    ];
    if !b.message.is_empty() {
        rows.push(Row::info("bk-message", b.message.clone(), ""));
    }
    if !b.error.is_empty() {
        rows.push(Row::info("bk-error", b.error.clone(), "").danger());
    }
    rows
}

pub(super) fn console_apply(d: &mut SettingsPage, outcome: crate::console::list::Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    use crate::console::list::{Outcome, Value};
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "bk-folder" => {
                let start = Some(std::path::PathBuf::from(d.backup.folder.read(cx).value().to_string()));
                d.console_browse("Backup folder", crate::console::browser::PickKind::Folder, start, window, cx, |d, path, window, cx| {
                    d.backup.folder.update(cx, |s, cx| s.set_value(path.clone(), window, cx));
                    persist(d, Some(path), None, cx);
                });
            }
            "bk-now" => backup_now(d, window, cx),
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "bk-interval" => {
            d.backup.interval = INTERVALS[index].0.to_string();
            let interval = d.backup.interval.clone();
            persist(d, None, Some(interval), cx);
        }
        _ => {}
    }
}
