//! Settings → Backups, the restore card under Ludusavi's: the folders a
//! game's Saves tab restores from, how many backups Ludusavi keeps, and
//! its list of games.

use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_emulation::ludusavi::restore::{restore_sources, update_manifest, SourceKind, Tool};
use heronite_emulation::ludusavi::{LudusaviEnv, SystemRunner};
use heronite_emulation::settings_ext::LudusaviSettingsPatch;
use heronite_i18n::{tr, trn};
use heronite_launch::SystemProbe;

use super::saves::save;
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

fn source_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Backups => tr!("Backup folder"),
        SourceKind::LudusaviRestore => tr!("Ludusavi's restore folder"),
        SourceKind::Extra | SourceKind::Picked => tr!("Added folder"),
    }
}

pub(super) fn add_folder(d: &mut SettingsPage, path: String, cx: &mut Context<SettingsPage>) {
    let mut paths = d.settings.ludusavi.restore_paths.clone();
    paths.push(path);
    save(d, LudusaviSettingsPatch { restore_paths: Some(paths), ..Default::default() }, cx);
}

fn remove_folder(d: &mut SettingsPage, path: &str, cx: &mut Context<SettingsPage>) {
    let paths = d.settings.ludusavi.restore_paths.iter().filter(|p| *p != path).cloned().collect();
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
                Tool::resolve(&env).and_then(|tool| update_manifest(&env, &tool))
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

/// The folders listed, as `(kind, path)`.
fn folders(d: &SettingsPage) -> Vec<(SourceKind, PathBuf)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let detected = d.saves.detected().cloned();
    restore_sources(&d.settings.ludusavi, detected.as_ref(), &home, None).into_iter().map(|s| (s.kind, s.root)).collect()
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let l = d.settings.ludusavi.clone();
    let st = &d.saves_restore;

    let mut list = card_box().py(px(6.));
    let folders = folders(d);
    if folders.is_empty() {
        list = list.child(div().py(px(10.)).child(meta_text(tr!("No folder yet. Set the backup folder above or add one."))));
    }
    for (i, (kind, path)) in folders.into_iter().enumerate() {
        let text = path.to_string_lossy().into_owned();
        let removable = kind == SourceKind::Extra;
        let remove = removable.then(|| {
            let p = text.clone();
            button_sm(SharedString::from(format!("restore-folder-remove-{i}")), ButtonKind::Ghost, tr!("Remove"))
                .on_click(cx.listener(move |d, _, _, cx| remove_folder(d, &p, cx)))
        });
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(8.))
                .when(i > 0, |r| r.border_t_1().border_color(theme::row_separator()))
                .child(icon(Icon::Folder, px(16.), theme::text_muted()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(text))
                        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(source_label(kind))),
                )
                .children(remove),
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

    let full = select_field(
        "ludusavi-full-limit",
        Some(tr!("Full backups kept per game")),
        full_label(l.full_limit),
        false,
        st.full_open,
        FULL.iter().map(|v| (full_label(*v).into(), *v == l.full_limit)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.saves_restore.full_open = !d.saves_restore.full_open;
            d.saves_restore.differential_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.saves_restore.full_open = false;
            save(d, LudusaviSettingsPatch { full_limit: Some(FULL[i]), ..Default::default() }, cx);
        },
    );
    let differential = select_field(
        "ludusavi-differential-limit",
        Some(tr!("Differential backups")),
        differential_label(l.differential_limit),
        false,
        st.differential_open,
        DIFFERENTIAL.iter().map(|v| (differential_label(*v).into(), *v == l.differential_limit)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.saves_restore.differential_open = !d.saves_restore.differential_open;
            d.saves_restore.full_open = false;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.saves_restore.differential_open = false;
            save(d, LudusaviSettingsPatch { differential_limit: Some(DIFFERENTIAL[i]), ..Default::default() }, cx);
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

    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Restoring saves")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "A game's settings have a Saves tab that puts a backup back into the game's prefix as it is now, even one made in another Steam library or on Windows. It lists the backups in these folders."
        ))))
        .child(list)
        .child(div().mt(px(10.)).flex().child(add))
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
    for (i, (kind, path)) in folders(d).into_iter().enumerate() {
        let row = Row::info(SharedString::from(format!("rs-folder-{i}")), path.to_string_lossy().into_owned(), source_label(kind));
        rows.push(if kind == SourceKind::Extra {
            Row::action(SharedString::from(format!("rs-remove-{i}")), path.to_string_lossy().into_owned())
                .icon(Icon::DeleteOutline)
                .desc(tr!("Added folder. A removes it from the list."))
        } else {
            row
        });
    }
    rows.push(Row::action("rs-add", tr!("Add folder…")).icon(Icon::Add));
    let fi = FULL.iter().position(|v| *v == l.full_limit).unwrap_or(0);
    rows.push(Row::choice("rs-full", tr!("Full backups kept per game"), FULL.iter().map(|v| full_label(*v).into()).collect(), fi));
    let di = DIFFERENTIAL.iter().position(|v| *v == l.differential_limit).unwrap_or(0);
    rows.push(Row::choice(
        "rs-differential",
        tr!("Differential backups"),
        DIFFERENTIAL.iter().map(|v| differential_label(*v).into()).collect(),
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
        Outcome::Activate(id) if id.starts_with("rs-remove-") => {
            let Some(i) = id.strip_prefix("rs-remove-").and_then(|n| n.parse::<usize>().ok()) else { return true };
            if let Some((_, path)) = folders(d).into_iter().nth(i) {
                remove_folder(d, &path.to_string_lossy(), cx);
            }
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "rs-full" => {
            save(d, LudusaviSettingsPatch { full_limit: Some(FULL[*index]), ..Default::default() }, cx)
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "rs-differential" => {
            save(d, LudusaviSettingsPatch { differential_limit: Some(DIFFERENTIAL[*index]), ..Default::default() }, cx)
        }
        _ => return false,
    }
    true
}
