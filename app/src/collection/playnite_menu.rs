//! Header SyncAlt dropdown (`PlayniteMenu.tsx`): Import library…, Merge from
//! Playnite, Export for Playnite…. The dialogs live in
//! `dialogs/playnite_{import,merge}.rs`.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_playnite::Playnite;

use super::CollectionView;
use crate::dialogs::playnite_import::ImportPlayniteDialog;
use crate::dialogs::playnite_merge::MergePlayniteDialog;
use crate::ui::dropdown::{menu_item, menu_panel};

#[derive(Default)]
pub struct PlayniteState {
    /// A saved library path exists (enables "Merge from Playnite").
    has_library: bool,
    /// The saved path was read since the last time the dialogs ran.
    checked: Cell<bool>,
    exporting: bool,
}

impl CollectionView {
    pub(super) fn playnite_menu_body(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.playnite.checked.replace(true) {
            let heroic = self.heroic.clone();
            cx.spawn(async move |this, cx| {
                let saved = cx
                    .background_executor()
                    .spawn(async move { Playnite::new(heroic).last_library_path().ok().flatten() })
                    .await;
                this.update(cx, |this, cx| {
                    this.playnite.has_library = saved.is_some_and(|p| Path::new(&p).join("games.db").exists());
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let merge_enabled = self.playnite.has_library;
        let exporting = self.playnite.exporting;
        // Same row as `menu_item` without the hover (a disabled `button`).
        let disabled = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .w_full()
                .py(px(6.67))
                .px(px(10.))
                .text_size(px(13.33))
                .opacity(0.5)
                .text_color(rgb(0xcaf3fd))
                .child(label)
        };
        menu_panel(240.)
            .child(menu_item("pn-import", false).child("Import library\u{2026}").on_click(cx.listener(
                |this, _, window, cx| this.open_playnite_import(window, cx),
            )))
            .child({
                if merge_enabled {
                    menu_item("pn-merge", false)
                        .child("Merge from Playnite")
                        .on_click(cx.listener(|this, _, window, cx| this.open_playnite_merge(window, cx)))
                        .into_any_element()
                } else {
                    disabled("pn-merge", "Merge from Playnite").into_any_element()
                }
            })
            .child({
                if exporting {
                    disabled("pn-export", "Exporting\u{2026}").into_any_element()
                } else {
                    menu_item("pn-export", false)
                        .child("Export for Playnite\u{2026}")
                        .on_click(cx.listener(|this, _, _, cx| this.export_playnite(cx)))
                        .into_any_element()
                }
            })
            .into_any_element()
    }

    pub(super) fn open_playnite_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        self.playnite.checked.set(false);
        let view = cx.new(|cx| ImportPlayniteDialog::new(ctx, window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn open_playnite_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        let view = cx.new(|cx| MergePlayniteDialog::new(ctx, window, cx));
        self.open_dialog(view, cx);
    }

    /// Asks where to save the export, writes it off-thread and toasts the result.
    pub(super) fn export_playnite(&mut self, cx: &mut Context<Self>) {
        if self.playnite.exporting {
            return;
        }
        self.open_menu = None;
        let suggested = heronite_playnite::export::resolve_export_path(Path::new("/"))
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let rx = cx.prompt_for_new_path(&dir, suggested.as_deref());
        let heroic = self.heroic.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut target))) = rx.await else { return };
            if !target.to_string_lossy().to_lowercase().ends_with(".json") {
                let mut s = target.into_os_string();
                s.push(".json");
                target = PathBuf::from(s);
            }
            this.update(cx, |this, cx| {
                this.playnite.exporting = true;
                cx.notify();
            })
            .ok();
            let res = cx.background_executor().spawn(async move { Playnite::new(heroic).export(&target) }).await;
            this.update(cx, |this, cx| {
                this.playnite.exporting = false;
                match res {
                    Ok(r) => this.show_toast(
                        format!(
                            "Exported {} games to {}. Copy this file to Windows when you want Playnite to pick up Heroic playtime and status.",
                            r.game_count, r.path
                        ),
                        false,
                        cx,
                    ),
                    Err(err) => this.show_toast(format!("Export for Playnite failed: {err:#}"), true, cx),
                }
            })
            .ok();
        })
        .detach();
    }
}
