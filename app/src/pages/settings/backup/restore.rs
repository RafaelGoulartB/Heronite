//! Settings → Backups, the restore card: the backups of the folder (and of
//! one opened from elsewhere), what each holds, putting one back and
//! removing one.
//!
//! Restoring checks the backup, backs up what is there now and restarts the
//! app, which copies the files before it reads anything
//! (`heronite_backup::restore`). Refused while a game runs.

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_backup::snapshot::{self, Info};
use heronite_backup::{Part, Reason};
use heronite_data::maintenance::human_size;
use heronite_i18n::{tr, trn};

use super::{format_time, part_label, SettingsPage};
use crate::icons::Icon;
use crate::pages::{caption_with, empty_row, icon_action, list_item, status_pill, tag_pill};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, card_box, checkbox, ButtonKind};
use crate::ui::fields::{choose_path, meta_text};
use crate::ui::icon;

#[derive(Default)]
pub struct State {
    /// The backups of the backup folder, newest first (`None` = not read yet).
    list: Option<Vec<Info>>,
    /// Disk space they take together.
    folder_bytes: Option<u64>,
    /// Backups opened from another folder, and that folder.
    other: Option<(PathBuf, Vec<Info>)>,
    /// The backup whose details are open.
    open: Option<PathBuf>,
    /// The parts of the open backup to restore.
    parts: Vec<Part>,
    confirm: Option<Confirm>,
    busy: bool,
    /// Last result (`true` = error).
    notice: Option<(String, bool)>,
}

#[derive(Clone, PartialEq)]
pub(super) enum Confirm {
    Restore(PathBuf),
    Delete(PathBuf),
}

impl State {
    pub(super) fn all(&self) -> impl Iterator<Item = &Info> {
        self.other.iter().flat_map(|(_, l)| l.iter()).chain(self.list.iter().flatten())
    }

    pub(super) fn busy(&self) -> bool {
        self.busy
    }

    pub(super) fn confirm(&self) -> Option<&Confirm> {
        self.confirm.as_ref()
    }

    pub(super) fn ask_restore(&mut self, path: PathBuf) {
        self.confirm = Some(Confirm::Restore(path));
    }
}

/// Reads the backup folder again.
pub(super) fn reload(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let folder = d.settings.backup.folder.trim().to_string();
    if folder.is_empty() {
        d.backup.restore.list = Some(Vec::new());
        d.backup.restore.folder_bytes = None;
        cx.notify();
        return;
    }
    cx.spawn(async move |this, cx| {
        let (list, bytes) = cx
            .background_spawn(async move {
                let path = PathBuf::from(&folder);
                (snapshot::list(&path), snapshot::folder_usage(&path))
            })
            .await;
        this.update(cx, |d, cx| {
            let r = &mut d.backup.restore;
            if r.open.as_ref().is_some_and(|p| !list.iter().any(|i| &i.path == p) && r.other.is_none()) {
                r.open = None;
            }
            r.list = Some(list);
            r.folder_bytes = Some(bytes);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Opens a backup, or a folder of backups, from anywhere (another drive,
/// a copy from another machine).
fn open_other(window: &mut Window, cx: &mut Context<SettingsPage>) {
    choose_path(window, cx, true, tr!("A backup, or a folder of backups"), |_: &mut SettingsPage, picked, _, cx| {
        load_other(PathBuf::from(picked), cx)
    });
}

fn load_other(path: PathBuf, cx: &mut Context<SettingsPage>) {
    cx.spawn(async move |this, cx| {
        let p = path.clone();
        let found = cx
            .background_spawn(async move {
                match snapshot::read_info(&p) {
                    Some(one) => vec![one],
                    None => snapshot::list(&p),
                }
            })
            .await;
        this.update(cx, |d, cx| {
            let r = &mut d.backup.restore;
            if found.is_empty() {
                r.notice = Some((tr!("No Heronite backup in {path}.", path = path.display()), true));
            } else {
                r.notice = None;
                let first = found[0].clone();
                r.other = Some((path, found));
                select(d, &first);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub(super) fn select(d: &mut SettingsPage, info: &Info) {
    let r = &mut d.backup.restore;
    if r.open.as_ref() == Some(&info.path) {
        r.open = None;
    } else {
        r.open = Some(info.path.clone());
        r.parts = info.parts.clone();
    }
    r.confirm = None;
}

fn toggle_part(d: &mut SettingsPage, part: Part) {
    let parts = &mut d.backup.restore.parts;
    if let Some(i) = parts.iter().position(|p| *p == part) {
        parts.remove(i);
    } else {
        parts.push(part);
        parts.sort();
    }
}

/// The running app's own binary: the AppImage when started from one.
fn own_executable() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()).map(PathBuf::from).or_else(|| std::env::current_exe().ok())
}

/// Checks the backup, backs up what is there now, then restarts into the restore.
pub(super) fn restore(d: &mut SettingsPage, path: PathBuf, parts: Vec<Part>, cx: &mut Context<SettingsPage>) {
    let r = &mut d.backup.restore;
    r.confirm = None;
    if let Some(why) = crate::system::restart_blocker(cx) {
        r.notice = Some((tr!("Heronite has to restart to restore, and cannot now: {why}.", why), true));
        cx.notify();
        return;
    }
    let Some(exe) = own_executable() else { return };
    r.busy = true;
    r.notice = Some((tr!("Checking the backup and backing up what you have now…").into(), false));
    cx.notify();
    // What is there now goes next to the other backups; with no usable
    // backup folder, next to the backup being restored.
    let configured = PathBuf::from(d.settings.backup.folder.trim());
    let safety = if configured.is_dir() { configured } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
    let (data, app) = (d.ctx.data_paths.clone(), d.ctx.app.clone());
    cx.spawn(async move |this, cx| {
        let res = cx.background_spawn(async move { heronite_backup::restore::prepare(&data, &app, &path, &parts, &safety) }).await;
        this.update(cx, |d, cx| {
            d.backup.restore.busy = false;
            match res {
                Ok(_) => {
                    if let Some(why) = crate::system::restart_blocker(cx) {
                        heronite_backup::restore::cancel(&d.ctx.app);
                        d.backup.restore.notice = Some((tr!("Heronite has to restart to restore, and cannot now: {why}.", why), true));
                    } else {
                        d.backup.restore.notice = Some((tr!("Restarting…").into(), false));
                        crate::system::restart_into(exe, cx);
                    }
                }
                Err(e) => d.backup.restore.notice = Some((heronite_backup::describe(&e), true)),
            }
            reload(d, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub(super) fn delete(d: &mut SettingsPage, path: PathBuf, cx: &mut Context<SettingsPage>) {
    let r = &mut d.backup.restore;
    r.confirm = None;
    r.busy = true;
    cx.notify();
    cx.spawn(async move |this, cx| {
        let p = path.clone();
        let res = cx.background_spawn(async move { snapshot::delete(&p) }).await;
        this.update(cx, |d, cx| {
            let r = &mut d.backup.restore;
            r.busy = false;
            match res {
                Ok(()) => {
                    if r.open.as_ref() == Some(&path) {
                        r.open = None;
                    }
                    if let Some((_, list)) = &mut r.other {
                        list.retain(|i| i.path != path);
                    }
                    r.notice = None;
                }
                Err(e) => r.notice = Some((format!("{e:#}"), true)),
            }
            reload(d, cx);
        })
        .ok();
    })
    .detach();
}

pub(super) fn reason_label(info: &Info) -> &'static str {
    match info.reason {
        Reason::Scheduled => tr!("Scheduled"),
        Reason::BeforeRestore => tr!("Before a restore"),
        Reason::Manual => tr!("Manual"),
    }
}

/// "199 games · 190 sessions · 57 MB (2 MB new)".
pub(super) fn summary_line(info: &Info) -> String {
    let s = &info.summary;
    let mut parts = vec![trn!(s.games, "{n} game", "{n} games"), trn!(s.sessions, "{n} session", "{n} sessions")];
    if s.journal > 0 {
        parts.push(trn!(s.journal, "{n} note", "{n} notes"));
    }
    parts.push(tr!("{size} ({new} new)", size = human_size(s.bytes), new = human_size(s.new_bytes)));
    parts.join(" \u{b7} ")
}

fn render_details(d: &SettingsPage, info: &Info, cx: &mut Context<SettingsPage>) -> Div {
    let r = &d.backup.restore;
    let mut parts = div().flex().flex_col();
    for part in info.parts.iter().copied() {
        let on = r.parts.contains(&part);
        parts = parts.child(checkbox(SharedString::from(format!("restore-part-{}", part.key())), on, part_label(part)).on_click(
            cx.listener(move |d, _, _, cx| {
                toggle_part(d, part);
                cx.notify();
            }),
        ));
    }
    let path = info.path.clone();
    let chosen = r.parts.clone();
    let confirming = r.confirm == Some(Confirm::Restore(path.clone()));
    let footer: AnyElement = if confirming {
        let names: Vec<&str> = chosen.iter().map(|p| part_label(*p)).collect();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(meta_text(tr!(
                "Heronite backs up what you have now, then restarts and puts back {parts} as they were on {when}. Anything of those added since then goes.",
                parts = names.join(", "),
                when = format_time(&info.created_at)
            )))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(action_button(
                        "restore-cancel",
                        ButtonKind::Ghost,
                        tr!("Cancel"),
                        true,
                        cx.listener(|d, _, _, cx| {
                            d.backup.restore.confirm = None;
                            cx.notify();
                        }),
                    ))
                    .child(action_button(
                        "restore-yes",
                        ButtonKind::Danger,
                        tr!("Restore and restart"),
                        true,
                        cx.listener(move |d, _, _, cx| restore(d, path.clone(), chosen.clone(), cx)),
                    )),
            )
            .into_any_element()
    } else {
        let p = path.clone();
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(action_button(
                "restore-start",
                ButtonKind::Primary,
                tr!("Restore…"),
                !r.busy && !r.parts.is_empty(),
                cx.listener(move |d, _, _, cx| {
                    d.backup.restore.confirm = Some(Confirm::Restore(p.clone()));
                    cx.notify();
                }),
            ))
            .child(button_sm("restore-open-folder", ButtonKind::Ghost, tr!("Open folder")).on_click(move |_, _, _| {
                if let Err(e) = super::super::open_folder(&path) {
                    log::error!("could not open {}: {e}", path.display());
                }
            }))
            .into_any_element()
    };
    div().flex().flex_col().gap(px(8.)).pb(px(12.)).pl(px(30.)).child(meta_text(tr!("Restore:"))).child(parts).child(footer)
}

fn render_row(d: &SettingsPage, info: &Info, i: usize, cx: &mut Context<SettingsPage>) -> Div {
    let r = &d.backup.restore;
    let open = r.open.as_ref() == Some(&info.path);
    let path = info.path.clone();
    let deleting = r.confirm == Some(Confirm::Delete(path.clone()));
    let pill = if info.reason == Reason::BeforeRestore {
        status_pill(reason_label(info), theme::status_color("on-hold"))
    } else {
        tag_pill(reason_label(info))
    };
    let content = div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(format_time(&info.created_at)))
                .child(pill),
        )
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(summary_line(info)));
    let trailing: AnyElement = if deleting {
        let p = path.clone();
        div()
            .flex()
            .gap(px(6.))
            .child(button_sm(SharedString::from(format!("backup-del-no-{i}")), ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(
                |d, _, _, cx| {
                    d.backup.restore.confirm = None;
                    cx.notify();
                },
            )))
            .child(
                button_sm(SharedString::from(format!("backup-del-yes-{i}")), ButtonKind::Danger, tr!("Delete"))
                    .on_click(cx.listener(move |d, _, _, cx| delete(d, p.clone(), cx))),
            )
            .into_any_element()
    } else {
        let p = path.clone();
        let enabled = !r.busy;
        icon_action(SharedString::from(format!("backup-del-{i}")), Icon::DeleteOutline, enabled)
            .when(enabled, |b| {
                b.on_click(cx.listener(move |d, _, _, cx| {
                    cx.stop_propagation();
                    d.backup.restore.confirm = Some(Confirm::Delete(p.clone()));
                    cx.notify();
                }))
            })
            .into_any_element()
    };
    let clicked = info.clone();
    let head = list_item(
        Some(icon(if open { Icon::FaChevronUp } else { Icon::FaChevronDown }, px(14.), theme::text_muted()).into_any_element()),
        content,
        trailing,
    )
    .id(SharedString::from(format!("backup-row-{i}")))
    .cursor_pointer()
    .on_click(cx.listener(move |d, _, _, cx| {
        select(d, &clicked);
        cx.notify();
    }));
    div()
        .flex()
        .flex_col()
        .when(i > 0, |r| r.border_t_1().border_color(theme::row_separator()))
        .child(head)
        .children(open.then(|| render_details(d, info, cx)))
}

pub(super) fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let r = &d.backup.restore;
    let open_other = button_sm("backup-open-other", ButtonKind::Ghost, tr!("Open a backup…"))
        .on_click(cx.listener(|_, _, window, cx| open_other(window, cx)));
    let mut col =
        div().flex().flex_col().child(caption_with(tr!("Restore"), open_other)).child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Pick a backup to see what it holds and put all or part of it back. What you have now is backed up first."
        ))));
    if let Some((folder, list)) = &r.other {
        let mut card = card_box();
        for (i, info) in list.iter().enumerate() {
            card = card.child(render_row(d, info, 1000 + i, cx));
        }
        col = col
            .child(div().mx(px(6.)).mb(px(6.)).child(meta_text(tr!("From {path}", path = folder.display()))))
            .child(card)
            .child(div().h(px(12.)));
    }
    let mut card = card_box();
    match &r.list {
        _ if d.settings.backup.folder.trim().is_empty() => {
            card = card.child(empty_row(tr!("Choose a backup folder above, or open a backup from another folder.")))
        }
        None => card = card.child(empty_row(tr!("Reading the backup folder…"))),
        Some(list) if list.is_empty() => card = card.child(empty_row(tr!("No backups in this folder yet."))),
        Some(list) => {
            for (i, info) in list.iter().enumerate() {
                card = card.child(render_row(d, info, i, cx));
            }
        }
    }
    col = col.child(card);
    if let (Some(list), Some(bytes)) = (&r.list, r.folder_bytes) {
        if !list.is_empty() {
            let text = trn!(list.len(), "{n} backup, {size} on disk.", "{n} backups, {size} on disk.", size = human_size(bytes));
            col = col.child(div().mt(px(8.)).mx(px(6.)).child(meta_text(text)));
        }
    }
    if let Some((text, error)) = r.notice.clone() {
        col = col.child(
            div()
                .mt(px(10.))
                .mx(px(6.))
                .text_size(px(13.))
                .text_color(if error { theme::danger_text() } else { theme::text_muted() })
                .child(text),
        );
    }
    col
}
