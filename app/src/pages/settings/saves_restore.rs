//! Settings → Saves, the restore card under Ludusavi's: the folders a
//! game's Saves tab restores from, how many backups Ludusavi keeps, and
//! its list of games.
//!
//! The folders are the backup folder (where new backups go, set in the
//! card above or by Ludusavi's own settings) and only the ones added here.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::ludusavi::restore::{restore_sources, update_manifest, SourceKind, Tool};
use heronite_emulation::ludusavi::{LudusaviEnv, SystemRunner};
use heronite_emulation::settings_ext::LudusaviSettingsPatch;
use heronite_i18n::{tr, trn};
use heronite_launch::SystemProbe;

use super::saves::{save, set_backup_folder};
use super::SettingsPage;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card_box, ButtonKind};
use crate::ui::fields::{choose_path, meta_text, select_field};
use crate::ui::icon;

/// Full backups kept per game; `None` = Ludusavi's own setting.
const FULL: [Option<u8>; 7] = [None, Some(1), Some(2), Some(3), Some(5), Some(10), Some(20)];
/// Differential backups per full one.
const DIFFERENTIAL: [Option<u8>; 6] = [None, Some(0), Some(1), Some(3), Some(5), Some(10)];

#[derive(Default)]
pub struct State {
    full_open: bool,
    differential_open: bool,
    manifest_busy: bool,
    /// Outcome of the last game list update (`Err` = error text).
    manifest: Option<Result<(), String>>,
    /// The folders listed and what they came from. Finding them can read
    /// Ludusavi's `config.yaml`, so not on every frame.
    folders: RefCell<Option<(FoldersKey, Listed)>>,
}

/// The folder list and Ludusavi's restore folder, offered when not in it.
#[derive(Clone, Default)]
struct Listed {
    folders: Vec<Folder>,
    suggestion: Option<PathBuf>,
}

/// Everything the folder list depends on.
#[derive(Clone, PartialEq)]
struct FoldersKey {
    use_installed_config: bool,
    backup_path: String,
    restore_paths: Vec<String>,
    detected: Option<(Option<String>, Option<String>, Option<String>)>,
}

/// A folder backups are restored from.
#[derive(Clone)]
struct Folder {
    kind: SourceKind,
    path: PathBuf,
    /// The entries of Settings' extra folders it stands for (one can match
    /// the backup folder and only show as that): Remove takes them out.
    extra: Vec<String>,
}

/// The presets, with the value in use added when it is not one of them
/// (set by hand), so it shows and stays picked.
fn options(presets: &[Option<u8>], current: Option<u8>) -> Vec<Option<u8>> {
    let mut all = presets.to_vec();
    if !all.contains(&current) {
        all.push(current);
        all.sort_by_key(|v| v.map(|n| n as i32 + 1).unwrap_or(0));
    }
    all
}

fn full_label(v: Option<u8>) -> String {
    match v {
        None => tr!("Ludusavi's setting").into(),
        Some(n) => trn!(n, "{n} backup", "{n} backups"),
    }
}

fn differential_label(v: Option<u8>) -> String {
    match v {
        None => tr!("Ludusavi's setting").into(),
        Some(0) => tr!("None (full backups only)").into(),
        Some(n) => tr!("{n} per full backup", n),
    }
}

/// Where the backup folder comes from, and whether it can be changed here.
fn backup_origin(d: &SettingsPage) -> (&'static str, bool) {
    let l = &d.settings.ludusavi;
    let config_known = d.saves.detected().is_some_and(|x| x.config_path.is_some());
    if l.use_installed_config && config_known {
        (tr!("New backups go here. It comes from Ludusavi's own settings."), false)
    } else if l.backup_path.trim().is_empty() {
        (tr!("New backups go here: Ludusavi's backup folder."), true)
    } else {
        (tr!("New backups go here. Also set in Backup location & format above."), true)
    }
}

fn label(d: &SettingsPage, kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Backups => backup_origin(d).0,
        SourceKind::Extra | SourceKind::Picked => tr!("Added folder: only read to restore."),
    }
}

/// Adds a folder to restore from (a folder already listed is left alone).
pub(super) fn add_folder(d: &mut SettingsPage, path: String, cx: &mut Context<SettingsPage>) {
    let home = std::env::var("HOME").unwrap_or_default();
    let at = normalized(&path, &home);
    if folders(d).folders.iter().any(|f| normalized(&f.path.to_string_lossy(), &home) == at) {
        return;
    }
    let mut paths = d.settings.ludusavi.restore_paths.clone();
    paths.push(path);
    save(d, LudusaviSettingsPatch { restore_paths: Some(paths), ..Default::default() }, cx);
}

fn open(path: &Path) {
    if let Err(e) = super::open_folder(path) {
        log::error!("could not open {}: {e}", path.display());
    }
}

/// Takes the given entries (as saved, `~/…` and all) out of the list.
fn remove_folder(d: &mut SettingsPage, entries: &[String], cx: &mut Context<SettingsPage>) {
    let paths = d.settings.ludusavi.restore_paths.iter().filter(|p| !entries.contains(p)).cloned().collect();
    save(d, LudusaviSettingsPatch { restore_paths: Some(paths), ..Default::default() }, cx);
}

pub(super) fn update_game_list(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.saves_restore.manifest_busy {
        return;
    }
    d.saves_restore.manifest_busy = true;
    d.saves_restore.manifest = None;
    let paths = d.ctx.data_paths.clone();
    cx.notify();
    cx.spawn(async move |this, cx| {
        let result = cx
            .background_spawn(async move {
                let runner = SystemRunner::default();
                let env = LudusaviEnv { paths: &paths, runner: &runner, probe: &SystemProbe };
                Tool::resolve(&env).map_err(String::from).and_then(|tool| update_manifest(&env, &tool))
            })
            .await;
        this.update(cx, |d, cx| {
            d.saves_restore.manifest_busy = false;
            d.saves_restore.manifest = Some(result);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// `~/` expanded and the trailing `/` dropped, to compare folders.
fn normalized(path: &str, home: &str) -> PathBuf {
    let p = path.trim();
    let p = match p.strip_prefix("~/") {
        Some(rest) => Path::new(home).join(rest),
        None if p == "~" => PathBuf::from(home),
        None => PathBuf::from(p),
    };
    PathBuf::from(p.to_string_lossy().trim_end_matches('/'))
}

/// The folders listed, read again only when the settings or Ludusavi's
/// detection change.
fn folders(d: &SettingsPage) -> Listed {
    let l = &d.settings.ludusavi;
    let detected = d.saves.detected();
    let key = FoldersKey {
        use_installed_config: l.use_installed_config,
        backup_path: l.backup_path.clone(),
        restore_paths: l.restore_paths.clone(),
        detected: detected.map(|d| (d.config_path.clone(), d.backup_path.clone(), d.restore_path.clone())),
    };
    let mut cache = d.saves_restore.folders.borrow_mut();
    if let Some((_, list)) = cache.as_ref().filter(|(k, _)| *k == key) {
        return list.clone();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let folders: Vec<Folder> = restore_sources(l, detected, &home, None)
        .into_iter()
        .map(|s| {
            let at = normalized(&s.root.to_string_lossy(), &home);
            let extra = l.restore_paths.iter().filter(|p| normalized(p, &home) == at).cloned().collect();
            Folder { kind: s.kind, path: s.root, extra }
        })
        .collect();
    let suggestion = detected
        .and_then(|d| d.restore_path.as_deref())
        .filter(|p| !p.trim().is_empty())
        .map(|p| normalized(p, &home))
        .filter(|p| !folders.iter().any(|f| normalized(&f.path.to_string_lossy(), &home) == *p));
    let listed = Listed { folders, suggestion };
    *cache = Some((key, listed.clone()));
    listed
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let l = d.settings.ludusavi.clone();
    let st = &d.saves_restore;

    let Listed { folders, suggestion } = folders(d);
    let can_change = backup_origin(d).1;
    let mut list = card_box().py(px(6.));
    if folders.is_empty() {
        list = list.child(div().py(px(10.)).child(meta_text(tr!("No folder yet. Set the backup folder above or add one."))));
    }
    for (i, f) in folders.into_iter().enumerate() {
        let label = label(d, f.kind);
        let text = f.path.to_string_lossy().into_owned();
        let path = f.path.clone();
        let open_button = button_sm(SharedString::from(format!("restore-folder-open-{i}")), ButtonKind::Ghost, tr!("Open"))
            .on_click(move |_, _, _| open(&path));
        // The backup folder is changed, an added one removed.
        let manage = match f.kind {
            SourceKind::Backups => can_change.then(|| {
                button_sm(SharedString::from(format!("restore-folder-change-{i}")), ButtonKind::Ghost, tr!("Change…")).on_click(
                    cx.listener(|_, _, window, cx| {
                        choose_path(window, cx, true, tr!("Save backup folder"), |d: &mut SettingsPage, p, window, cx| {
                            set_backup_folder(d, p, window, cx)
                        });
                    }),
                )
            }),
            _ => {
                let extra = f.extra.clone();
                (!extra.is_empty()).then(|| {
                    button_sm(SharedString::from(format!("restore-folder-remove-{i}")), ButtonKind::Ghost, tr!("Remove"))
                        .on_click(cx.listener(move |d, _, _, cx| remove_folder(d, &extra, cx)))
                })
            }
        };
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(8.))
                .when(i > 0, |r| r.border_t_1().border_color(theme::row_separator()))
                .child(icon(if f.kind == SourceKind::Backups { Icon::Save } else { Icon::Folder }, px(16.), theme::text_muted()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(text))
                        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(label)),
                )
                .child(open_button)
                .children(manage),
        );
    }
    let add = action_button(
        "restore-folder-add",
        ButtonKind::Glass,
        tr!("Add folder…"),
        true,
        cx.listener(|_, _, window, cx| {
            choose_path(window, cx, true, tr!("Folder with Ludusavi backups"), |d: &mut SettingsPage, p, _, cx| add_folder(d, p, cx));
        }),
    );
    let suggestion =
        suggestion.map(|path| {
            let text = path.to_string_lossy().into_owned();
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .mt(px(10.))
                .mx(px(6.))
                .child(div().flex_1().min_w_0().child(meta_text(tr!(
                    "Ludusavi's own settings restore from {path}. Add it to see its backups too.",
                    path = text.clone()
                ))))
                .child(
                    button_sm("restore-folder-suggested", ButtonKind::Glass, tr!("Add"))
                        .on_click(cx.listener(move |d, _, _, cx| add_folder(d, text.clone(), cx))),
                )
        });

    let full_options = options(&FULL, l.full_limit);
    let differential_options = options(&DIFFERENTIAL, l.differential_limit);
    let full = select_field(
        "ludusavi-full-limit",
        Some(tr!("Full backups kept per game")),
        full_label(l.full_limit),
        false,
        st.full_open,
        full_options.iter().map(|v| (full_label(*v).into(), *v == l.full_limit)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.saves_restore.full_open = !d.saves_restore.full_open;
            d.saves_restore.differential_open = false;
            cx.notify();
        },
        move |d: &mut SettingsPage, i, _, cx| {
            d.saves_restore.full_open = false;
            if let Some(v) = full_options.get(i) {
                save(d, LudusaviSettingsPatch { full_limit: Some(*v), ..Default::default() }, cx);
            }
        },
    );
    let differential = select_field(
        "ludusavi-differential-limit",
        Some(tr!("Differential backups")),
        differential_label(l.differential_limit),
        false,
        st.differential_open,
        differential_options.iter().map(|v| (differential_label(*v).into(), *v == l.differential_limit)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.saves_restore.differential_open = !d.saves_restore.differential_open;
            d.saves_restore.full_open = false;
            cx.notify();
        },
        move |d: &mut SettingsPage, i, _, cx| {
            d.saves_restore.differential_open = false;
            if let Some(v) = differential_options.get(i) {
                save(d, LudusaviSettingsPatch { differential_limit: Some(*v), ..Default::default() }, cx);
            }
        },
    );

    let manifest_line = match &st.manifest {
        _ if st.manifest_busy => Some((tr!("Updating Ludusavi's game list…").to_string(), false)),
        Some(Ok(())) => Some((tr!("Ludusavi's game list is up to date.").to_string(), false)),
        Some(Err(e)) => Some((e.clone(), true)),
        None => None,
    };
    let manifest = div()
        .flex()
        .items_center()
        .gap(px(12.))
        .mt(px(12.))
        .child(action_button(
            "ludusavi-manifest-update",
            ButtonKind::Glass,
            tr!("Update game list"),
            !st.manifest_busy,
            cx.listener(|d, _, _, cx| update_game_list(d, cx)),
        ))
        .children(manifest_line.map(|(text, error)| {
            div().text_size(px(13.)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
        }));

    let about = if cfg!(windows) {
        tr!(
            "The Saves tab of each game lists the backups in these folders: the one new backups go to, and only the folders you add. It puts a backup back where the game keeps its saves on this PC, even one made by another Windows user, in another Steam library or on Linux."
        )
    } else {
        tr!(
            "The Saves tab of each game lists the backups in these folders: the one new backups go to, and only the folders you add. It puts a backup back into the game's prefix as it is now, even one made in another Steam library or on Windows."
        )
    };
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Restoring saves")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(about)))
        .child(list)
        .child(div().mt(px(10.)).flex().child(add))
        .children(suggestion)
        .child(caption(tr!("Backups kept")))
        .child(card_box().py(px(12.)).gap(px(12.)).child(full).child(differential))
        .child(div().mt(px(8.)).mx(px(6.)).child(meta_text(tr!(
            "Ludusavi removes the oldest backups past these numbers. Locked backups are never removed: lock one in the game's Saves tab. Differential backups only store what changed since the last full one."
        ))))
        .child(manifest)
        .child(div().mt(px(6.)).mx(px(6.)).child(meta_text(tr!(
            "Ludusavi's list of games and where they keep saves. It updates by itself once a day; update it now when a new game is not found."
        ))))
        .into_any_element()
}

/// Console mode rows, after Ludusavi's.
pub(super) fn console_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let l = &d.settings.ludusavi;
    let st = &d.saves_restore;
    let mut rows = vec![Row::header(tr!("Restoring saves"))];
    let Listed { folders, suggestion } = folders(d);
    let can_change = backup_origin(d).1;
    for (i, f) in folders.into_iter().enumerate() {
        let text = f.path.to_string_lossy().into_owned();
        rows.push(match f.kind {
            SourceKind::Backups if can_change => {
                Row::action("rs-change", text).icon(Icon::Save).desc(format!("{} {}", label(d, f.kind), tr!("A changes it.")))
            }
            SourceKind::Backups => Row::info(SharedString::from(format!("rs-folder-{i}")), text, label(d, f.kind)),
            _ if f.extra.is_empty() => Row::info(SharedString::from(format!("rs-folder-{i}")), text, label(d, f.kind)),
            _ => Row::action(SharedString::from(format!("rs-remove-{i}")), text)
                .icon(Icon::DeleteOutline)
                .desc(tr!("Added folder. A removes it from the list.")),
        });
    }
    rows.push(Row::action("rs-add", tr!("Add folder…")).icon(Icon::Add));
    if let Some(path) = suggestion {
        rows.push(
            Row::action("rs-suggested", tr!("Add Ludusavi's restore folder")).icon(Icon::Add).desc(path.to_string_lossy().into_owned()),
        );
    }
    let full = options(&FULL, l.full_limit);
    let fi = full.iter().position(|v| *v == l.full_limit).unwrap_or(0);
    rows.push(Row::choice("rs-full", tr!("Full backups kept per game"), full.iter().map(|v| full_label(*v).into()).collect(), fi));
    let differential = options(&DIFFERENTIAL, l.differential_limit);
    let di = differential.iter().position(|v| *v == l.differential_limit).unwrap_or(0);
    rows.push(Row::choice(
        "rs-differential",
        tr!("Differential backups"),
        differential.iter().map(|v| differential_label(*v).into()).collect(),
        di,
    ));
    let desc = match &st.manifest {
        Some(Ok(())) => tr!("Ludusavi's game list is up to date.").to_string(),
        Some(Err(e)) => e.clone(),
        None => tr!("Updates by itself once a day").to_string(),
    };
    rows.push(
        Row::action("rs-manifest", if st.manifest_busy { tr!("Updating Ludusavi's game list…") } else { tr!("Update game list") })
            .icon(Icon::Refresh)
            .enabled(!st.manifest_busy)
            .desc(desc),
    );
    rows
}

/// Handles a console row of [`console_rows`]; `false` when it is not one.
pub(super) fn console_apply(
    d: &mut SettingsPage,
    outcome: &crate::console::list::Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> bool {
    use crate::console::browser::PickKind;
    use crate::console::list::{Outcome, Value};
    match outcome {
        Outcome::Activate(id) if id.as_ref() == "rs-add" => {
            d.console_browse(tr!("Folder with Ludusavi backups"), PickKind::Folder, None, window, cx, |d, path, _, cx| {
                add_folder(d, path, cx)
            });
        }
        Outcome::Activate(id) if id.as_ref() == "rs-manifest" => update_game_list(d, cx),
        Outcome::Activate(id) if id.as_ref() == "rs-change" => {
            d.console_browse(tr!("Save backup folder"), PickKind::Folder, None, window, cx, |d, path, window, cx| {
                set_backup_folder(d, path, window, cx)
            });
        }
        Outcome::Activate(id) if id.as_ref() == "rs-suggested" => {
            if let Some(path) = folders(d).suggestion {
                add_folder(d, path.to_string_lossy().into_owned(), cx);
            }
        }
        Outcome::Activate(id) if id.starts_with("rs-remove-") => {
            let Some(i) = id.strip_prefix("rs-remove-").and_then(|n| n.parse::<usize>().ok()) else { return true };
            if let Some(f) = folders(d).folders.into_iter().nth(i) {
                remove_folder(d, &f.extra, cx);
            }
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "rs-full" => {
            if let Some(v) = options(&FULL, d.settings.ludusavi.full_limit).get(*index) {
                save(d, LudusaviSettingsPatch { full_limit: Some(*v), ..Default::default() }, cx)
            }
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "rs-differential" => {
            if let Some(v) = options(&DIFFERENTIAL, d.settings.ludusavi.differential_limit).get(*index) {
                save(d, LudusaviSettingsPatch { differential_limit: Some(*v), ..Default::default() }, cx)
            }
        }
        _ => return false,
    }
    true
}
