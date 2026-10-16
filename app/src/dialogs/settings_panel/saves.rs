//! "Cloud saves" tab: Epic (legendary: saves folder, manual sync, autosync) and
//! GOG (the folders GOG lists, manual sync through the download service, autosync).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::saves::{self, SyncMode};
use heronite_gog::saves::{self as gog_saves, SaveLocation};
use serde_json::Value;

use super::parts::{column, danger_text, heading, help, hint, rows};
use super::{Notice, Section, SettingsPanel};
use crate::theme;
use crate::ui::fields::field_label;
use heronite_i18n::{n_, tr};

pub const MODES: [(&str, SyncMode); 3] = [
    (n_!("Sync (download and upload)"), SyncMode::Both),
    (n_!("Force Download"), SyncMode::ForceDownload),
    (n_!("Force Upload"), SyncMode::ForceUpload),
];

pub const GOG_MODES: [(&str, gog_saves::SyncMode); 3] = [
    (n_!("Sync (download and upload)"), gog_saves::SyncMode::Both),
    (n_!("Force Download"), gog_saves::SyncMode::ForceDownload),
    (n_!("Force Upload"), gog_saves::SyncMode::ForceUpload),
];

impl SettingsPanel {
    /// Heroic guesses the saves folder when the tab opens with none set.
    pub(super) fn maybe_detect_saves(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.section != Section::Saves || self.saves.detect_tried || !self.is_game() {
            return;
        }
        self.saves.detect_tried = true;
        if self.is_gog() {
            self.load_gog_locations(false, cx);
            return;
        }
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
                    Err(err) => {
                        this.notice = Some(Notice {
                            text: tr!("Could not detect the saves folder: {error}", error = format!("{err:#}")).into(),
                            error: true,
                        })
                    }
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
                        this.notice = Some(Notice { text: tr!("Sync Complete").into(), error: false });
                    }
                    Err(err) => {
                        this.notice = Some(Notice { text: tr!("Sync failed: {error}", error = format!("{err:#}")).into(), error: true })
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn saves_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        if self.is_gog() {
            return self.gog_saves_tab(cx);
        }
        if !self.facts().is_some_and(|f| f.cloud_saves) {
            return column().child(hint(tr!("This game does not support Cloud Saves.")));
        }
        let has_path = !self.text("savesPath").is_empty();
        let mut col = column().child(hint(tr!("Cloud saves are in beta: back up your saves before syncing.")));
        if self.saves.detecting {
            return col.child(help(tr!("Trying to detect the correct save folder\u{2026}")).px(px(6.)));
        }
        let retry = div()
            .id("saves-retry")
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(theme::accent())
            .child(if has_path {
                tr!("Please check twice if the path is correct (click to retry)")
            } else {
                tr!("Save folder not found, please select it manually (click to retry)")
            })
            .on_click(cx.listener(|this, _, window, cx| this.detect_saves(window, cx)));
        let folder = rows().field_help(
            self.path_row("savesPath", Some(tr!("Saves folder")), true, tr!("Choose the saves directory"), true, cx),
            vec![
                div().child(retry),
                help(tr!("Found by itself in most games; change it above if it is wrong.")),
                help(tr!("After changing the prefix, clear this path and detect it again: Proton uses /pfx and the user steamuser.")),
            ],
        );
        col = col.child(heading(tr!("Location"))).child(folder.card());

        let mode = self.saves.mode;
        let select = self.select_row(
            "saves-mode",
            Some(tr!("Sync now")),
            tr(MODES[mode].0).to_string(),
            MODES.iter().enumerate().map(|(i, (l, _))| (SharedString::from(tr(l)), i == mode)).collect(),
            !has_path,
            cx,
            |this, i, _, cx| {
                this.saves.mode = i;
                cx.notify();
            },
        );
        let syncing = self.saves.syncing;
        let sync = div().flex().items_end().gap(px(12.)).child(div().flex_1().child(select)).child(self.outline_button(
            "saves-sync",
            if syncing { tr!("Syncing") } else { tr!("Sync") },
            has_path && !syncing,
            cx,
            |this, _, cx| this.sync_saves(cx),
        ));
        let mut sync_rows = rows().field_help(
            sync,
            vec![help(tr!("Sync sends and gets what changed; Force download or upload replaces one side with the other."))],
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
        col = col.child(heading(tr!("Sync"))).child(sync_rows.card());

        let mut auto = rows().child(self.toggle_row(
            "autosync",
            "autoSyncSaves",
            false,
            tr!("Sync automatically"),
            Some(tr!("Syncs when the game starts and after it exits.")),
            !has_path,
            cx,
        ));
        if has_path {
            auto = auto.child(self.toggle_row(
                "quick-saves",
                "enableQuickSavesMenu",
                false,
                tr!("Quick sync menu on the game page"),
                None,
                false,
                cx,
            ));
        }
        col.child(heading(tr!("Automatic"))).child(auto.card())
    }
}

/// GOG games.
impl SettingsPanel {
    pub(super) fn is_gog(&self) -> bool {
        self.facts().is_some_and(|f| f.is_gog())
    }

    /// Reads the save folders in the background; `force` asks GOG again.
    pub(super) fn load_gog_locations(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(app) = self.facts().map(|f| f.app_name.clone()) else { return };
        if self.saves.gog.loading {
            return;
        }
        self.saves.gog.loading = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if force {
                        gog_saves::refresh_locations(&paths, &app)
                    } else {
                        gog_saves::locations(&paths, &app)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.saves.gog.loading = false;
                this.saves.gog.locations = Some(res.map_err(|e| format!("{e:#}")));
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The folders once loaded; empty while loading, on errors and for games
    /// without cloud saves.
    pub(super) fn gog_locations(&self) -> &[SaveLocation] {
        match &self.saves.gog.locations {
            Some(Ok(list)) => list,
            _ => &[],
        }
    }

    /// A manual sync of this game runs in the download service.
    pub(super) fn gog_syncing(&self, cx: &App) -> bool {
        let Some(app) = self.facts().map(|f| f.app_name.as_str()) else { return false };
        crate::services::downloads_started(cx).is_some_and(|d| d.read(cx).busy_of(app).is_some())
    }

    pub(super) fn gog_sync(&mut self, cx: &mut Context<Self>) {
        let Some((app, title)) = self.facts().map(|f| (f.app_name.clone(), f.title.clone())) else { return };
        let mode = GOG_MODES[self.saves.mode.min(GOG_MODES.len() - 1)].1;
        let service = crate::services::downloads(cx);
        service.update(cx, |s, cx| s.gog_sync_saves(&app, &title, mode, cx));
    }

    fn gog_location_row(location: &SaveLocation) -> Div {
        let name = if location.name == "__default" { tr!("Default folder").to_string() } else { location.name.clone() };
        div().flex().flex_col().py(px(12.)).child(field_label(name)).child(
            div()
                .font_family(theme::FONT_MONO)
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme::text_muted())
                .child(location.location.clone()),
        )
    }

    fn gog_saves_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let loading = self.saves.gog.loading;
        let mut places = rows();
        match &self.saves.gog.locations {
            None => places = places.child(div().py(px(12.)).child(help(tr!("Loading the save folders\u{2026}")))),
            Some(Err(err)) => {
                places = places.child(div().py(px(12.)).child(danger_text(&tr!("Could not load the save folders: {error}", error = err))))
            }
            Some(Ok(list)) if list.is_empty() => {
                places = places.child(div().py(px(12.)).child(help(tr!("This game does not support Cloud Saves."))))
            }
            Some(Ok(list)) => {
                for location in list {
                    places = places.child(Self::gog_location_row(location));
                }
            }
        }
        let refresh = self.outline_button(
            "gog-saves-refresh",
            if loading { tr!("Loading\u{2026}") } else { tr!("Refresh locations") },
            !loading,
            cx,
            |this, _, cx| this.load_gog_locations(true, cx),
        );
        places = places.field_help(
            div().flex().child(refresh),
            vec![help(tr!(
                "GOG lists these folders. They are kept in the game's settings (gogSaves); refresh them after changing the Wine prefix."
            ))],
        );

        let has_places = !self.gog_locations().is_empty();
        let syncing = self.gog_syncing(cx);
        let mode = self.saves.mode;
        let select = self.select_row(
            "saves-mode",
            Some(tr!("Sync now")),
            tr(GOG_MODES[mode].0).to_string(),
            GOG_MODES.iter().enumerate().map(|(i, (l, _))| (SharedString::from(tr(l)), i == mode)).collect(),
            !has_places,
            cx,
            |this, i, _, cx| {
                this.saves.mode = i;
                cx.notify();
            },
        );
        let sync = div().flex().items_end().gap(px(12.)).child(div().flex_1().child(select)).child(self.outline_button(
            "saves-sync",
            if syncing { tr!("Syncing") } else { tr!("Sync") },
            has_places && !syncing,
            cx,
            |this, _, cx| this.gog_sync(cx),
        ));
        let sync_rows = rows().field_help(
            sync,
            vec![help(tr!("Sync sends and gets what changed; Force download or upload replaces one side with the other."))],
        );
        // Offline or unreadable: the launcher still tries around a launch.
        let no_cloud = matches!(&self.saves.gog.locations, Some(Ok(list)) if list.is_empty());
        let auto = rows().child(self.toggle_row(
            "autosync",
            "autoSyncSaves",
            false,
            tr!("Sync automatically"),
            Some(tr!("Syncs when the game starts and after it exits.")),
            no_cloud,
            cx,
        ));
        column()
            .child(hint(tr!("Cloud saves are in beta: back up your saves before syncing.")))
            .child(heading(tr!("Location")))
            .child(places.card())
            .child(heading(tr!("Sync")))
            .child(sync_rows.card())
            .child(heading(tr!("Automatic")))
            .child(auto.card())
    }
}
