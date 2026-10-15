//! Desktop view of the Saves tab.

use std::path::Path;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_emulation::ludusavi::restore::{BackupEntry, Change, FileKind, PlannedFile, RestorePlan, RestoreTarget, SourceKind};
use heronite_i18n::{tr, trn};

use super::{BackupKey, Load, Loaded, SaveBackupsView, Stage};
use crate::icons::Icon;
use crate::pages::settings::format_time;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card_box, help_text, mini_checkbox, ButtonKind};
use crate::ui::fields::{choose_path, input_box, meta_text, select_field};
use crate::ui::{icon, tool_button};

/// `Windows` / `Linux` for the OS Ludusavi recorded.
pub(super) fn os_label(os: Option<&str>) -> Option<&'static str> {
    match os? {
        "windows" => Some("Windows"),
        "linux" => Some("Linux"),
        "mac" => Some("macOS"),
        _ => None,
    }
}

/// A backup's title (its name, else its date) and the line under it
/// (the date when named, the OS, the size, locked).
pub(super) fn backup_line(b: &BackupEntry) -> (String, String) {
    let when = format_time(&b.when);
    let mut facts: Vec<String> = Vec::new();
    let title = match &b.comment {
        Some(name) => {
            facts.push(when);
            name.clone()
        }
        None => when,
    };
    facts.extend(os_label(b.os.as_deref()).map(|os| tr!("made on {os}", os)));
    facts.extend(b.bytes.map(human_size));
    if b.locked {
        facts.push(tr!("locked").into());
    }
    (title, facts.join(" \u{b7} "))
}

pub(super) fn source_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Backups => tr!("Backup folder"),
        SourceKind::Extra => tr!("Added folder"),
        SourceKind::Picked => tr!("Picked folder"),
    }
}

/// A file path as the game sees it: `C:/users/steamuser/...` inside the
/// prefix, `<game folder>/...`, `Steam userdata/...`, else the full path.
pub(super) fn short_path(path: &str, target: &RestoreTarget) -> String {
    let p = Path::new(path);
    if let Some(prefix) = &target.prefix {
        if let Ok(rest) = p.strip_prefix(prefix.join("drive_c")) {
            return format!("C:/{}", rest.display());
        }
        if let Ok(rest) = p.strip_prefix(prefix) {
            return tr!("Prefix: {file}", file = rest.display().to_string());
        }
    }
    if let Some(rest) = target.install_dir.as_ref().and_then(|d| p.strip_prefix(d).ok()) {
        return tr!("Game folder: {file}", file = rest.display().to_string());
    }
    if let Some(rest) = target.steam_userdata.as_ref().and_then(|d| p.strip_prefix(d).ok()) {
        return tr!("Steam userdata: {file}", file = rest.display().to_string());
    }
    path.to_string()
}

/// Folders listed per group before "and N more folders".
const MAX_FOLDERS: usize = 3;

/// The files of one [`FileKind`] in a preview, summed up.
pub(super) struct Group {
    pub kind: FileKind,
    pub count: usize,
    pub bytes: u64,
    /// Every file of the group is picked.
    pub selected: bool,
    /// Files that replace a different one on disk.
    pub replaces: usize,
    /// `(folder, files)` as the game sees them, most files first.
    pub folders: Vec<(String, usize)>,
}

/// The parent folder of a [`short_path`].
fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir.to_string(),
        _ => path.to_string(),
    }
}

/// A folder shared by many games (`C:/users/steamuser/AppData/Local`,
/// `Documents/My Games`, the start of Steam's userdata), so files under it
/// are not merged into one line.
fn is_generic(parts: &[&str]) -> bool {
    let n = parts.len();
    let lower: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    match lower.first().map(String::as_str) {
        Some("c:") if lower.get(1).map(String::as_str) == Some("users") => match lower.get(3).map(String::as_str) {
            None => true,
            Some("appdata") => n <= 5,
            Some("documents") => n <= 4 || (n == 5 && lower[4] == "my games"),
            Some(_) => n <= 4,
        },
        Some(first) if first.ends_with(':') && first.len() == 2 => n <= 2,
        // An absolute Linux path: `/home/<user>/.local/share` and above.
        Some("") => n <= 5,
        // `Steam userdata: <account>` / `Game folder: ...` labels.
        _ => n <= 1,
    }
}

/// Folders of `paths` with their file count, most files first. Folders
/// whose common parent is specific to the game become one line
/// (`C:/users/steamuser/AppData/Local/LunaAbyss/Saved`, not one line per
/// subfolder).
fn folders(paths: &[String]) -> Vec<(String, usize)> {
    let mut out: Vec<(Vec<String>, usize)> = Vec::new();
    for p in paths {
        let dir: Vec<String> = parent_of(p).split('/').map(str::to_string).collect();
        match out.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, n)) => *n += 1,
            None => out.push((dir, 1)),
        }
    }
    'merge: loop {
        for i in 0..out.len() {
            for j in i + 1..out.len() {
                let same = out[i].0.iter().zip(&out[j].0).take_while(|(a, b)| a == b).count();
                let common: Vec<String> = out[i].0[..same].to_vec();
                let parts: Vec<&str> = common.iter().map(String::as_str).collect();
                if same > 0 && !is_generic(&parts) {
                    let n = out[j].1;
                    out.remove(j);
                    out[i] = (common, out[i].1 + n);
                    continue 'merge;
                }
            }
        }
        break;
    }
    let mut out: Vec<(String, usize)> = out.into_iter().map(|(d, n)| (d.join("/"), n)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The groups of a preview: saves first, empty kinds left out.
pub(super) fn groups(plan: &RestorePlan, target: &RestoreTarget) -> Vec<Group> {
    [FileKind::Save, FileKind::Registry, FileKind::Outside, FileKind::Unmapped]
        .into_iter()
        .filter_map(|kind| {
            let files: Vec<&PlannedFile> = plan.files.iter().filter(|f| f.kind == kind).collect();
            (!files.is_empty()).then(|| {
                let paths: Vec<String> = files.iter().map(|f| short_path(&f.target, target)).collect();
                Group {
                    kind,
                    count: files.len(),
                    bytes: files.iter().map(|f| f.bytes).sum(),
                    selected: files.iter().all(|f| f.selected),
                    replaces: files.iter().filter(|f| f.change == Change::Different).count(),
                    folders: folders(&paths),
                }
            })
        })
        .collect()
}

pub(super) fn group_title(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Save => tr!("Game saves"),
        FileKind::Registry => tr!("Wine registry"),
        FileKind::Outside => tr!("Outside the game's prefix"),
        FileKind::Unmapped => tr!("No place on Linux"),
    }
}

/// `35 files · 2.4 MB · 3 replace files on this PC`.
pub(super) fn group_facts(g: &Group) -> String {
    let mut facts = vec![trn!(g.count, "{n} file", "{n} files"), human_size(g.bytes)];
    if g.replaces > 0 {
        facts.push(trn!(g.replaces, "{n} replaces a file on this PC", "{n} replace files on this PC"));
    }
    facts.join(" \u{b7} ")
}

pub(super) fn group_help(kind: FileKind) -> Option<&'static str> {
    match kind {
        FileKind::Save => None,
        FileKind::Registry => Some(tr!(
            "The whole registry of the prefix: restoring it also rolls back the settings of everything else installed there. Only pick it if the game keeps its save in the registry."
        )),
        FileKind::Outside => {
            Some(tr!("Copies from another prefix or Steam library on this PC. They are written back where they were, not into the game."))
        }
        FileKind::Unmapped => Some(tr!("Windows paths on another drive. They cannot be restored here.")),
    }
}

/// What the restore overwrites, for the line under the groups.
pub(super) fn overwrite_line(plan: &RestorePlan) -> String {
    let overwrites = plan.overwrites();
    if overwrites > 0 {
        trn!(
            overwrites,
            "{n} file on this PC will be replaced. A copy is kept so you can undo.",
            "{n} files on this PC will be replaced. A copy is kept so you can undo."
        )
    } else {
        tr!("No file on this PC is replaced.").into()
    }
}

/// Where the restore writes, as a sentence.
pub(super) fn target_line(t: &RestoreTarget, installed: bool) -> (String, bool) {
    if t.missing_prefix() && !installed {
        return (
            tr!("The game is not installed. You can see, lock and take its backups here; to restore one, install the game and start it once.")
                .into(),
            false,
        );
    }
    if t.missing_prefix() {
        return (tr!("The game's prefix does not exist yet. Start the game once, then restore.").into(), true);
    }
    match (&t.prefix, &t.user) {
        (Some(p), Some(u)) => (tr!("Restores into {prefix} (Wine user {user}).", prefix = p.display().to_string(), user = u), false),
        (Some(p), None) => (tr!("Restores into {prefix}.", prefix = p.display().to_string()), false),
        _ => (tr!("Native game: saves go back to the folders they were backed up from.").into(), false),
    }
}

fn notice(text: String, error: bool) -> Div {
    div().mt(px(10.)).mx(px(6.)).text_size(px(13.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
}

impl SaveBackupsView {
    pub(super) fn render_desktop(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body: AnyElement = match &self.load {
            Load::Idle | Load::Loading => card_box().py(px(12.)).child(help_text(tr!("Looking for backups…"))).into_any_element(),
            Load::Failed(e) => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(card_box().py(px(12.)).child(div().text_size(px(13.)).text_color(theme::danger_text()).child(e.clone())))
                .child(div().child(action_button(
                    "sb-retry",
                    ButtonKind::Glass,
                    tr!("Try again"),
                    true,
                    cx.listener(|v, _, _, cx| v.reload(cx)),
                )))
                .into_any_element(),
            Load::Ready(_) => match &self.stage {
                Stage::List | Stage::Planning => self.render_list(cx),
                Stage::Plan(_) => self.render_plan(cx),
                Stage::Restoring => card_box().py(px(12.)).child(help_text(tr!("Restoring the save…"))).into_any_element(),
                Stage::Done(_) => self.render_done(cx),
            },
        };
        div().flex().flex_col().child(body).children(self.notice.clone().map(|(text, error)| notice(text, error))).into_any_element()
    }

    fn render_target(&self, l: &Loaded) -> Div {
        let (line, error) = target_line(&l.target, self.installed);
        let steam = self.steam_app_id.is_some() && l.target.prefix.is_some();
        card_box()
            .py(px(12.))
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(if error { theme::danger_text() } else { theme::text_primary() })
                    .child(line),
            )
            .children(steam.then(|| {
                help_text(tr!(
                    "With Steam Cloud on, Steam may ask which saves to keep the next time the game starts: pick the ones on this PC."
                ))
            }))
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(l) = self.loaded() else { return div().into_any_element() };
        let planning = matches!(self.stage, Stage::Planning);
        let busy = self.busy();
        let target = self.render_target(l);

        let options: Vec<(SharedString, bool)> = l
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let count = trn!(s.backups.len(), "{n} backup", "{n} backups");
                (format!("{} \u{b7} {count}", s.source.root.display()).into(), i == self.source)
            })
            .chain(std::iter::once((tr!("Other folder…").into(), false)))
            .collect();
        let n_sources = l.sources.len();
        let current = l.sources.get(self.source);
        let value = current.map(|s| s.source.root.display().to_string()).unwrap_or_else(|| tr!("No folder").into());
        let folder = select_field(
            "sb-folder",
            Some(tr!("Backup folder")),
            value,
            busy,
            self.source_open,
            options,
            cx,
            |v: &mut Self, _, cx| {
                v.source_open = !v.source_open;
                cx.notify();
            },
            move |v: &mut Self, i, window, cx| {
                if i < n_sources {
                    v.pick_source(i, cx);
                } else {
                    v.source_open = false;
                    choose_path(window, cx, true, tr!("Folder with Ludusavi backups"), |v: &mut Self, p, _, cx| v.use_folder(p, cx));
                }
            },
        );

        let mut list = card_box().py(px(6.));
        match current {
            None => list = list.child(div().py(px(10.)).child(meta_text(tr!("No backup folder yet. Set one in Settings → Backups.")))),
            Some(s) if s.backups.is_empty() => {
                let text = s.error.clone().unwrap_or_else(|| tr!("No backup of this game in this folder.").into());
                list = list.child(div().py(px(10.)).child(meta_text(text)));
            }
            Some(s) => {
                let picked = self.selected_key();
                for (i, b) in s.backups.iter().enumerate() {
                    let key = BackupKey::of(s, b);
                    let selected = picked.as_ref() == Some(&key);
                    list = list.child(self.backup_row(i, key, b, selected, busy, cx));
                }
            }
        }
        let has_backup = current.is_some_and(|s| !s.backups.is_empty());
        let can_restore = has_backup && !busy && !l.target.missing_prefix();
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(12.))
            .child(action_button(
                "sb-restore",
                ButtonKind::Primary,
                if planning { tr!("Reading the backup…") } else { tr!("Restore…") },
                can_restore,
                cx.listener(|v, _, _, cx| v.start_restore(cx)),
            ))
            .child(action_button(
                "sb-backup-now",
                ButtonKind::Glass,
                if self.working { tr!("Working…") } else { tr!("Back up now") },
                !busy,
                cx.listener(|v, _, _, cx| v.backup_now(cx)),
            ))
            .child(action_button("sb-refresh", ButtonKind::Ghost, tr!("Refresh"), !busy, cx.listener(|v, _, _, cx| v.reload(cx))));

        let undo = l.undo.as_ref().map(|u| {
            card_box().mt(px(12.)).py(px(10.)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon(Icon::History, px(16.), theme::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(tr!("Last restore: {when}. A copy of what it replaced is kept.", when = format_time(&u.created_at))),
                    )
                    .child(action_button("sb-undo", ButtonKind::Glass, tr!("Undo"), !busy, cx.listener(|v, _, _, cx| v.undo_last(cx)))),
            )
        });

        div()
            .flex()
            .flex_col()
            .child(target)
            .child(div().mt(px(12.)).child(folder))
            .child(caption(tr!("Backups")))
            .child(list)
            .child(actions)
            .children(undo)
            .into_any_element()
    }

    /// `i` only names the elements; every action goes by `key`.
    fn backup_row(&self, i: usize, key: BackupKey, b: &BackupEntry, selected: bool, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let (title, facts) = backup_line(b);
        let dot = div()
            .flex_none()
            .size(px(16.))
            .rounded_full()
            .border_2()
            .border_color(if selected { theme::accent().into() } else { theme::white_a(0.3) })
            .when(selected, |d| d.child(div().m(px(3.)).size(px(6.)).rounded_full().bg(theme::accent())));
        let row = div()
            .id(SharedString::from(format!("sb-backup-{i}")))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(8.))
            .py(px(8.))
            .my(px(2.))
            .rounded(px(10.))
            .when(selected, |d| d.bg(theme::white_a(0.06)))
            .child(dot);
        let small = |id: &str, kind, label: &'static str| button_sm(SharedString::from(format!("sb-{id}-{i}")), kind, label);

        if let Some((_, input, _)) = self.renaming.as_ref().filter(|(r, _, _)| *r == key) {
            return row
                .child(div().flex_1().min_w_0().child(input_box(input, false)))
                .child(small("name-save", ButtonKind::Primary, tr!("Save")).on_click(cx.listener(|v, _, _, cx| v.save_rename(cx))))
                .child(small("name-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|v, _, _, cx| v.cancel_rename(cx))))
                .into_any_element();
        }
        if self.confirm_delete.as_ref() == Some(&key) {
            return row
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(theme::text_primary())
                        .child(tr!("Delete \u{201c}{name}\u{201d}? This cannot be undone.", name = title)),
                )
                .child(small("delete-yes", ButtonKind::Danger, tr!("Delete")).on_click(cx.listener(|v, _, _, cx| v.delete_confirmed(cx))))
                .child(small("delete-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|v, _, _, cx| v.cancel_delete(cx))))
                .into_any_element();
        }
        let k = key.clone();
        let lock = small("lock", ButtonKind::Ghost, if b.locked { tr!("Unlock") } else { tr!("Lock") })
            .when(!busy, |b| b.on_click(cx.listener(move |v, _, _, cx| v.toggle_lock(k.clone(), cx))));
        let k = key.clone();
        let rename = tool_button(SharedString::from(format!("sb-rename-{i}")), Icon::Edit, false)
            .when(!busy, |b| b.on_click(cx.listener(move |v, _, window, cx| v.start_rename(k.clone(), window, cx))));
        let k = key.clone();
        let delete = tool_button(SharedString::from(format!("sb-delete-{i}")), Icon::DeleteOutline, false)
            .when(!busy, |b| b.on_click(cx.listener(move |v, _, _, cx| v.ask_delete(k.clone(), cx))));
        row.cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.04)))
            .on_click(cx.listener(move |v, _, _, cx| v.pick_backup(key.clone(), cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(title))
                    .children((!facts.is_empty()).then(|| div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(facts))),
            )
            .children(b.locked.then(|| icon(Icon::Flag, px(14.), theme::text_muted())))
            .child(rename)
            .child(lock)
            .child(delete)
            .into_any_element()
    }

    fn group_card(&self, g: &Group, cx: &mut Context<Self>) -> AnyElement {
        let enabled = g.kind != FileKind::Unmapped;
        let kind = g.kind;
        let id = SharedString::from(format!("sb-group-{}", g.kind as u8));
        let check = mini_checkbox(SharedString::from(format!("{id}-check")), g.selected, !enabled);
        let mut folders = div().flex().flex_col().gap(px(3.)).mt(px(8.));
        for (dir, n) in g.folders.iter().take(MAX_FOLDERS) {
            folders = folders.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(Icon::Folder, px(14.), theme::text_dim()))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(dir.clone()))
                    .child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(trn!(*n, "{n} file", "{n} files"))),
            );
        }
        if g.folders.len() > MAX_FOLDERS {
            let more = g.folders.len() - MAX_FOLDERS;
            folders = folders.child(meta_text(trn!(more, "and {n} more folder", "and {n} more folders")));
        }
        card_box()
            .py(px(12.))
            .child(
                div()
                    .id(id)
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .when(enabled, |d| d.cursor_pointer().on_click(cx.listener(move |v, _, _, cx| v.toggle_kind(kind, cx))))
                    .child(check)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(group_title(g.kind)))
                            .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(group_facts(g)))
                            .children(group_help(g.kind).map(|h| div().mt(px(4.)).child(meta_text(h)))),
                    ),
            )
            .child(div().pl(px(40.)).child(folders))
            .into_any_element()
    }

    fn render_plan(&self, cx: &mut Context<Self>) -> AnyElement {
        let (Stage::Plan(preview), Some(l)) = (&self.stage, self.loaded()) else { return div().into_any_element() };
        let (plan, groups) = (&preview.plan, &preview.groups);
        let when = l
            .sources
            .get(self.source)
            .and_then(|s| s.backups.iter().find(|b| b.id == plan.request.backup_id))
            .map(|b| format_time(&b.when))
            .unwrap_or_else(|| plan.request.backup_id.clone());
        let selected = plan.selected().count();

        let mut col = div().flex().flex_col().gap(px(10.));
        if !groups.iter().any(|g| g.kind == FileKind::Save) {
            col =
                col.child(card_box().py(px(12.)).child(meta_text(tr!("Nothing in this backup belongs in the game's prefix or folders."))));
        }
        for g in groups {
            col = col.child(self.group_card(g, cx));
        }
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(14.))
            .child(action_button("sb-plan-back", ButtonKind::Ghost, tr!("Back"), true, cx.listener(|v, _, _, cx| v.back_to_list(cx))))
            .child(action_button(
                "sb-plan-restore",
                ButtonKind::Primary,
                trn!(selected, "Restore {n} file", "Restore {n} files"),
                selected > 0,
                cx.listener(|v, _, _, cx| v.confirm_restore(cx)),
            ));

        div()
            .flex()
            .flex_col()
            .child(self.render_target(l))
            .child(caption(tr!("Restore the backup of {when}", when)))
            .child(col)
            .child(div().mt(px(10.)).mx(px(6.)).child(meta_text(overwrite_line(plan))))
            .child(actions)
            .into_any_element()
    }

    fn render_done(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Stage::Done(outcome) = &self.stage else { return div().into_any_element() };
        let target = self.loaded().map(|l| l.target.clone()).unwrap_or_default();
        let mut lines: Vec<String> = Vec::new();
        lines.push(trn!(outcome.written, "Restored {n} file.", "Restored {n} files."));
        if outcome.unchanged > 0 {
            lines.push(trn!(outcome.unchanged, "{n} was already the same.", "{n} were already the same."));
        }
        let failed = outcome.failed.clone();
        let can_undo = outcome.undo.is_some() && !self.busy();
        let mut card = card_box().py(px(12.)).gap(px(6.));
        for line in lines {
            card = card.child(div().text_size(px(14.)).text_color(theme::text_primary()).child(line));
        }
        if !failed.is_empty() {
            card = card.child(div().mt(px(4.)).text_size(px(13.)).text_color(theme::danger_text()).child(trn!(
                failed.len(),
                "{n} file could not be written:",
                "{n} files could not be written:"
            )));
            for (path, error) in failed {
                card = card.child(meta_text(format!("{} \u{2014} {error}", short_path(&path, &target))));
            }
        }
        div()
            .flex()
            .flex_col()
            .child(card)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(12.))
                    .child(action_button(
                        "sb-done-undo",
                        ButtonKind::Glass,
                        tr!("Undo"),
                        can_undo,
                        cx.listener(|v, _, _, cx| v.undo_last(cx)),
                    ))
                    .child(action_button(
                        "sb-done-ok",
                        ButtonKind::Primary,
                        tr!("Done"),
                        true,
                        cx.listener(|v, _, _, cx| v.back_to_list(cx)),
                    )),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::folders;

    #[test]
    fn folders_merge_under_the_game_folder() {
        let base = "C:/users/steamuser/AppData/Local/LunaAbyss/Saved";
        let mut paths: Vec<String> = (0..30).map(|i| format!("{base}/Config/WindowsNoEditor/f{i}.ini")).collect();
        paths.push(format!("{base}/SaveGames/slot1.sav"));
        paths.push("C:/users/steamuser/Documents/Other/a.sav".into());
        paths.push("C:/users/steamuser/Documents/Other/b.sav".into());
        assert_eq!(folders(&paths), [(base.to_string(), 31), ("C:/users/steamuser/Documents/Other".to_string(), 2)]);
        assert_eq!(folders(&["user.reg".to_string()]), [("user.reg".to_string(), 1)]);
        // Steam userdata merges at the game's folder; two games in AppData stay apart.
        let ud = "Steam userdata: 314512607/1933000/remote";
        let paths = vec![
            format!("{ud}/profiles/123/a.sav"),
            format!("{ud}/profiles/456/b.sav"),
            format!("{ud}/settings.cfg"),
            "C:/users/steamuser/AppData/Local/A/x.ini".to_string(),
            "C:/users/steamuser/AppData/Local/B/y.ini".to_string(),
            "C:/users/steamuser/Documents/My Games/C/z.sav".to_string(),
            "C:/users/steamuser/Documents/My Games/D/w.sav".to_string(),
        ];
        assert_eq!(
            folders(&paths),
            [
                (ud.to_string(), 3),
                ("C:/users/steamuser/AppData/Local/A".to_string(), 1),
                ("C:/users/steamuser/AppData/Local/B".to_string(), 1),
                ("C:/users/steamuser/Documents/My Games/C".to_string(), 1),
                ("C:/users/steamuser/Documents/My Games/D".to_string(), 1),
            ]
        );
    }
}
