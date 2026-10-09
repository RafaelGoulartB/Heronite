//! The Epic items of the context menu as console rows (game page in console
//! mode): same choices and the same requests as `focus.rs`.

use gpui_kit::*;
use heronite_data::CollectionGame;
use serde_json::Value;

use super::super::CollectionView;
use super::Request;
use crate::console::list::Row;
use crate::icons::Icon;
use crate::services::downloads::EosGame;

impl CollectionView {
    /// Rows `epic:<action>` for `game` (nothing for other stores).
    pub(in crate::collection) fn epic_console_rows(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Vec<Row> {
        if game.runner != "legendary" {
            return Vec::new();
        }
        let svc = self.epic_service(cx);
        let (busy, queued, update) = {
            let s = svc.read(cx);
            (s.busy_of(&game.app_name).is_some(), s.queued(&game.app_name).is_some(), s.has_update(&game.app_name))
        };
        let cloud = game.game.rest.get("cloud_save_enabled").and_then(Value::as_bool).unwrap_or(false);
        let flags = svc.read(cx).game_flags(&game.app_name);
        let mut rows = Vec::new();
        if queued {
            rows.push(Row::action("epic:downloads", "Show in Downloads").icon(Icon::Downloading));
        }
        if !game.installed {
            rows.push(Row::action("epic:import", "Import an existing install").icon(Icon::FolderOpen));
            return rows;
        }
        if busy || queued {
            return rows;
        }
        if update {
            rows.push(Row::action("epic:update", "Update").icon(Icon::Update));
        }
        rows.push(Row::action("epic:verify", "Verify and repair").icon(Icon::Build));
        rows.push(Row::action("epic:move", "Move install").icon(Icon::Folder));
        if flags.has_dlcs && !flags.third_party {
            rows.push(Row::action("epic:modify", "Modify installation").icon(Icon::Tune).desc("Choose the DLCs to keep installed"));
        }
        if cfg!(target_os = "linux") && !flags.third_party {
            let app = game.app_name.clone();
            svc.update(cx, |s, cx| s.eos_load_game(&app, cx));
            if let Some(state @ (EosGame::Enabled | EosGame::Disabled)) = svc.read(cx).eos_game(&game.app_name) {
                rows.push(Row::toggle("epic:eos", "EOS Overlay", state == EosGame::Enabled).icon(Icon::Fullscreen));
            }
        }
        if cloud {
            rows.push(Row::action("epic:sync", "Sync cloud saves").icon(Icon::CloudSync));
        }
        rows.push(Row::action("epic:uninstall", "Uninstall\u{2026}").icon(Icon::DeleteForever).danger());
        rows
    }

    /// Runs the row `id` (`epic:<action>`) for `key`.
    pub(in crate::collection) fn epic_console_activate(&mut self, id: &str, key: &str, cx: &mut Context<Self>) {
        let k = key.to_string();
        self.epic.pending = match id {
            "epic:downloads" => Some(Request::OpenDownloads),
            "epic:import" => Some(Request::Install { key: k, import: true }),
            "epic:verify" => Some(Request::Verify(k)),
            "epic:move" => Some(Request::Move(k)),
            "epic:modify" => Some(Request::Modify(k)),
            "epic:eos" => Some(Request::EosToggle(k)),
            "epic:uninstall" => Some(Request::Uninstall(k)),
            "epic:update" => {
                self.epic_direct(key, cx);
                None
            }
            "epic:sync" => {
                if let Some(game) = self.epic_game(key) {
                    let (app, title) = (game.app_name.clone(), game.title.clone());
                    self.epic_service(cx).update(cx, |s, cx| s.sync_saves(&app, &title, cx));
                }
                None
            }
            _ => None,
        };
        cx.notify();
    }
}
