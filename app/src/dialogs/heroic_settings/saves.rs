//! "Cloud saves" tab (legendary): saves folder, manual sync, autosync.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::saves::{self, SyncMode};
use serde_json::Value;

use super::parts::{column, heading, help, hint, rows};
use super::{HeroicSettingsPanel, Notice, Section};
use crate::theme;

pub const MODES: [(&str, SyncMode); 3] = [
    ("Sync (download and upload)", SyncMode::Both),
    ("Force Download", SyncMode::ForceDownload),
    ("Force Upload", SyncMode::ForceUpload),
];

impl HeroicSettingsPanel {
    /// Heroic guesses the saves folder when the tab opens with none set.
    pub(super) fn maybe_detect_saves(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.section != Section::Saves || self.saves.detect_tried || !self.is_game() {
            return;
        }
        self.saves.detect_tried = true;
        if self.text("savesPath").is_empty() && self.facts().is_some_and(|f| f.cloud_saves) {
            self.detect_saves(window, cx);
        }
    }

    pub(super) fn detect_saves(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        if self.saves.detecting {
            return;
        }
        self.saves.detecting = true;
        let paths = self.paths.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_spawn(async move { saves::detect_path(&paths, &app) }).await;
            this.update_in(cx, |this, window, cx| {
                this.saves.detecting = false;
                match res {
                    Ok(Some(path)) => {
                        this.set_input("savesPath", &path, window, cx);
                        this.set("savesPath", Value::String(path), cx);
                    }
                    Ok(None) => {}
                    Err(err) => this.notice = Some(Notice { text: format!("Could not detect the saves folder: {err:#}").into(), error: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn sync_saves(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        let path = self.text("savesPath");
        if path.is_empty() || self.saves.syncing {
            return;
        }
        self.saves.syncing = true;
        self.saves.output = None;
        self.notice = None;
        let mode = MODES[self.saves.mode].1;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { saves::sync(&paths, &app, &path, mode) }).await;
            this.update(cx, |this, cx| {
                this.saves.syncing = false;
                match res {
                    Ok(out) => {
                        this.saves.output = Some(out.lines().map(str::to_string).collect());
                        this.notice = Some(Notice { text: "Sync Complete".into(), error: false });
                    }
                    Err(err) => this.notice = Some(Notice { text: format!("Sync failed: {err:#}").into(), error: true }),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn saves_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        if !self.facts().is_some_and(|f| f.cloud_saves) {
            return column().child(hint("This game does not support Cloud Saves."));
        }
        let has_path = !self.text("savesPath").is_empty();
        let mut col = column().child(hint(
            "Cloud Saves feature is in Beta, please backup your saves before syncing (in case something goes wrong)",
        ));
        if self.saves.detecting {
            return col.child(help("Trying to detect the correct save folder\u{2026}").px(px(6.)));
        }
        let retry = div()
            .id("saves-retry")
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(theme::accent())
            .child(if has_path {
                "Please check twice if the path is correct (click to retry)"
            } else {
                "Save folder not found, please select it manually (click to retry)"
            })
            .on_click(cx.listener(|this, _, window, cx| this.detect_saves(window, cx)));
        let folder = rows().field_help(
            self.path_row("savesPath", Some("Saves folder"), true, "Choose the saves directory", true, cx),
            vec![
                div().child(retry),
                help("Heroic tries to guess the right save folder and this will work on the majority of cases. In case the folder is wrong, use the box above to change it."),
                help("In case you change the prefix folder for Wine or Proton, check the path again since Proton uses a different prefix (/pfx) and username (steamuser): clear the path and click retry."),
            ],
        );
        col = col.child(heading("Location")).child(folder.card());

        let mode = self.saves.mode;
        let select = self.select_row(
            "saves-mode",
            Some("Manual Sync Saves"),
            MODES[mode].0.to_string(),
            MODES.iter().enumerate().map(|(i, (l, _))| (SharedString::from(*l), i == mode)).collect(),
            !has_path,
            cx,
            |this, i, _, cx| {
                this.saves.mode = i;
                cx.notify();
            },
        );
        let syncing = self.saves.syncing;
        let sync = div()
            .flex()
            .items_end()
            .gap(px(12.))
            .child(div().flex_1().child(select))
            .child(self.outline_button("saves-sync", if syncing { "Syncing" } else { "Sync" }, has_path && !syncing, cx, |this, _, cx| {
                this.sync_saves(cx)
            }));
        let mut sync_rows = rows().field_help(
            sync,
            vec![help("Manual Sync: Sync downloads and uploads what changed. Force Download and Force Upload ignore the version that is locally or on the cloud.")],
        );
        if let Some(lines) = self.saves.output.clone() {
            let mut out = div()
                .flex()
                .flex_col()
                .max_h(px(180.))
                .overflow_hidden()
                .my(px(10.))
                .p(px(10.))
                .rounded(px(theme::R_INPUT))
                .bg(theme::input_fill())
                .border_1()
                .border_color(theme::input_border());
            for l in lines.into_iter().take(60) {
                out = out.child(div().font_family(theme::FONT_MONO).text_size(px(11.5)).text_color(theme::text_muted()).child(l));
            }
            sync_rows = sync_rows.child(out);
        }
        col = col.child(heading("Sync")).child(sync_rows.card());

        let mut auto = rows().child(self.toggle_row(
            "autosync",
            "autoSyncSaves",
            false,
            "Autosync Saves",
            Some("Autosync Saves will sync the saves every time you start a game and after finishing playing."),
            !has_path,
            cx,
        ));
        if has_path {
            auto = auto.child(self.toggle_row(
                "quick-saves",
                "enableQuickSavesMenu",
                false,
                "Enable Quick Save-Sync Menu on game page",
                None,
                false,
                cx,
            ));
        }
        col.child(heading("Automatic")).child(auto.card())
    }
}
