//! Settings → Cloud, "Restore from the cloud" (folded at the end of the
//! page): a new or formatted PC connects and brings everything back. What
//! the account holds is listed when asked; one PC is picked (the one of the
//! newest backup first); then its Heronite backup, its game saves (into
//! Ludusavi's backup folder only) and the screenshots come back one by one,
//! or all at once.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_i18n::tr;
use heronite_sync::cloud::{CloudBackup, InCloud};

use super::features::with_service;
use crate::pages::settings::SettingsPage;
use crate::services::sync::{Bring, Step};
use crate::services::widgets::progress_bar;
use crate::theme;
use crate::ui::controls::{action_button, button_sm, row, ButtonKind};
use crate::ui::fields::{meta_text, select_field};

/// When a backup was made, in words.
fn made(b: &CloudBackup) -> String {
    b.made()
        .and_then(|t| t.and_local_timezone(chrono::Local).single())
        .map(|t| crate::pages::settings::format_time(&t.to_rfc3339()))
        .unwrap_or_else(|| b.name.clone())
}

fn separator() -> Div {
    div().h(px(1.)).w_full().bg(theme::row_separator())
}

/// The PC picked (`d.cloud.restore_pc`), else the one of the newest backup.
fn picked(d: &SettingsPage, listed: &InCloud) -> Option<String> {
    let pcs = listed.pcs();
    d.cloud.restore_pc.clone().filter(|pc| pcs.contains(pc)).or_else(|| pcs.first().cloned())
}

/// What runs now, with its progress and Stop.
fn progress(step: Step, percent: Option<u8>, cx: &mut Context<SettingsPage>) -> Div {
    let what = match step {
        Step::Backup => tr!("Bringing the backup…"),
        Step::Saves => tr!("Bringing the game saves…"),
        Step::Captures => tr!("Bringing the screenshots…"),
    };
    let label = match percent {
        Some(n) => format!("{what} {n}%"),
        None => what.to_owned(),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .py(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(label))
                .child(
                    button_sm("cloud-bring-stop", ButtonKind::Ghost, tr!("Stop"))
                        .on_click(cx.listener(|_, _, _, cx| with_service(cx, |s, cx| s.stop_bringing(cx)))),
                ),
        )
        .child(progress_bar(percent.map(f32::from), 6., theme::accent()))
}

fn listed_body(d: &mut SettingsPage, listed: &InCloud, busy: bool, cx: &mut Context<SettingsPage>) -> Div {
    let pcs = listed.pcs();
    let Some(pc) = picked(d, listed) else {
        return div().py(px(10.)).child(meta_text(tr!("Nothing in the cloud yet.")));
    };
    let select = select_field(
        "cloud-restore-pc",
        Some(tr!("Bring back from")),
        pc.clone(),
        busy,
        d.cloud.restore_pc_open,
        pcs.iter().map(|p| (SharedString::from(p.clone()), *p == pc)).collect(),
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.cloud.restore_pc_open = !d.cloud.restore_pc_open;
            cx.notify();
        },
        {
            let pcs = pcs.clone();
            move |d: &mut SettingsPage, i, _, cx| {
                d.cloud.restore_pc_open = false;
                d.cloud.restore_pc = pcs.get(i).cloned();
                cx.notify();
            }
        },
    );
    let newest = listed.backups.iter().find(|b| b.pc == pc).cloned();
    let has_saves = listed.saves.contains(&pc);
    let bring = |id: &'static str, label: &'static str, enabled: bool, what: Bring, cx: &mut Context<SettingsPage>| {
        button_sm(id, ButtonKind::Glass, label).when(enabled && !busy, |b| {
            b.on_click(cx.listener(move |_, _, _, cx| {
                let what = what.clone();
                with_service(cx, |s, cx| s.bring(what, cx))
            }))
        })
    };
    let backup_row = match &newest {
        Some(b) => row(
            tr!("Heronite backup"),
            Some(&format!("{} \u{b7} {}", made(b), human_size(b.bytes))),
            bring("cloud-bring-backup", tr!("Restore"), true, Bring::Backup(b.clone()), cx),
        ),
        None => row(tr!("Heronite backup"), Some(tr!("None from this PC")), div()),
    };
    let saves_row = row(
        tr!("Game saves"),
        Some(if has_saves {
            tr!("Go to Ludusavi's backup folder only: your games' own save folders are not touched. Restore a game from its Saves tab when you want it back.")
        } else {
            tr!("None from this PC")
        }),
        bring("cloud-bring-saves", tr!("Bring here"), has_saves, Bring::Saves(pc.clone()), cx),
    );
    let captures_row = row(
        tr!("Screenshots"),
        Some(if listed.captures {
            tr!("Of every PC, into the screenshots folder; the ones already here stay.")
        } else {
            tr!("None in the cloud")
        }),
        bring("cloud-bring-captures", tr!("Bring here"), listed.captures, Bring::Captures, cx),
    );
    let everything = action_button(
        "cloud-bring-everything",
        ButtonKind::Primary,
        tr!("Restore everything"),
        !busy,
        cx.listener(move |_, _, _, cx| {
            let what = Bring::Everything(pc.clone());
            with_service(cx, |s, cx| s.bring(what, cx))
        }),
    );
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().pb(px(6.)).child(select))
        .child(backup_row)
        .child(separator())
        .child(saves_row)
        .child(separator())
        .child(captures_row)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .pt(px(10.))
                .child(everything)
                .child(meta_text(tr!("The saves and screenshots first, then the backup: restoring it restarts Heronite."))),
        )
        .children(render_all_backups(listed, busy, cx))
}

/// Older backups, folded: each can be restored too.
fn render_all_backups(listed: &InCloud, busy: bool, cx: &mut Context<SettingsPage>) -> Option<Div> {
    if listed.backups.len() < 2 {
        return None;
    }
    let mut col = div()
        .flex()
        .flex_col()
        .pt(px(10.))
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Every backup in the cloud")));
    for (i, b) in listed.backups.iter().enumerate() {
        let what = Bring::Backup(b.clone());
        let button = button_sm(SharedString::from(format!("cloud-restore-{i}")), ButtonKind::Ghost, tr!("Restore")).when(!busy, |btn| {
            btn.on_click(cx.listener(move |_, _, _, cx| {
                let what = what.clone();
                with_service(cx, |s, cx| s.bring(what, cx))
            }))
        });
        col = col.child(row(made(b), Some(&format!("{} \u{b7} {}", b.pc, human_size(b.bytes))), button));
    }
    Some(col)
}

pub(super) fn render(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let svc = crate::services::sync::get(cx);
    let (listing, listed, notice) = svc
        .as_ref()
        .map(|s| {
            let s = s.read(cx);
            (s.listing_cloud(), s.in_cloud().cloned(), s.restore_notice().cloned())
        })
        .unwrap_or_default();
    let bringing = svc.as_ref().and_then(|s| {
        let s = s.read(cx);
        s.bringing().map(|b| (b.step, b.percent))
    });
    let label = match (listing, listed.is_some()) {
        (true, _) => tr!("Looking…"),
        (false, true) => tr!("Look again"),
        (false, false) => tr!("See what is in the cloud"),
    };
    let head = row(
        tr!("What is in the cloud"),
        None,
        button_sm("cloud-look", ButtonKind::Glass, label)
            .when(!listing, |b| b.on_click(cx.listener(|_, _, _, cx| with_service(cx, |s, cx| s.list_cloud(cx))))),
    );
    let mut card = div().flex().flex_col().child(head);
    match listed {
        Some(Err(e)) => card = card.child(div().pb(px(8.)).text_size(px(12.5)).text_color(theme::danger_text()).child(e)),
        Some(Ok(listed)) => card = card.child(separator()).child(listed_body(d, &listed, bringing.is_some(), cx)),
        None => {}
    }
    if let Some((step, percent)) = bringing {
        card = card.child(separator()).child(progress(step, percent, cx));
    }
    if let Some((text, error)) = notice {
        card = card.child(
            div().pt(px(8.)).text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text),
        );
    }
    card
}
