//! The store items of the context menu as console rows (game page in console
//! mode): same choices and the same requests as `focus.rs`.

use gpui_kit::*;
use heronite_data::CollectionGame;

use super::super::CollectionView;
use super::Request;
use crate::console::list::Row;
use crate::icons::Icon;
use crate::services::downloads::EosGame;
use heronite_i18n::tr;

impl CollectionView {
    /// Rows `epic:<action>` for a store game (nothing for other games).
    pub(in crate::collection) fn epic_console_rows(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Vec<Row> {
        if !super::is_store_runner(&game.runner) {
            return Vec::new();
        }
        let svc = self.epic_service(cx);
        let (busy, queued, update) = {
            let s = svc.read(cx);
            (s.busy_of(&game.app_name).is_some(), s.queued(&game.app_name).is_some(), s.has_update(&game.app_name))
        };
        let cloud = super::has_cloud_saves(game);
        let flags = svc.read(cx).game_flags(&game.app_name);
        let mut rows = Vec::new();
        if queued {
            rows.push(Row::action("epic:downloads", tr!("Show in Downloads")).icon(Icon::Downloading));
        }
        // EA app / Ubisoft Connect keep their games: legendary cannot import,
        // update, verify or move them (Heroic hides these too).
        if !game.store_installed() {
            if !flags.third_party {
                rows.push(Row::action("epic:import", tr!("Import an existing install")).icon(Icon::FolderOpen));
            }
            return rows;
        }
        if busy || queued {
            return rows;
        }
        if game.install_folder_missing() {
            rows.push(
                Row::action("epic:relocate", tr!("Change install location\u{2026}"))
                    .icon(Icon::FolderOpen)
                    .desc(tr!("The game's folder is missing")),
            );
            rows.push(Row::action("epic:forget", tr!("Remove from installed games\u{2026}")).icon(Icon::DeleteOutline).danger());
            return rows;
        }
        if !flags.third_party {
            if update {
                rows.push(Row::action("epic:update", tr!("Update")).icon(Icon::Update));
            }
            rows.push(Row::action("epic:verify", tr!("Verify and repair")).icon(Icon::Build));
            rows.push(Row::action("epic:move", tr!("Move install")).icon(Icon::Folder));
            rows.push(Row::action("epic:relocate", tr!("Change install location\u{2026}")).icon(Icon::FolderOpen));
        }
        if flags.has_dlcs && !flags.third_party {
            rows.push(
                Row::action("epic:modify", tr!("Modify installation")).icon(Icon::Tune).desc(tr!("Choose the DLCs to keep installed")),
            );
        }
        if cfg!(target_os = "linux") && !flags.third_party && game.runner == "legendary" {
            let app = game.app_name.clone();
            svc.update(cx, |s, cx| s.eos_load_game(&app, cx));
            if let Some(state @ (EosGame::Enabled | EosGame::Disabled)) = svc.read(cx).eos_game(&game.app_name) {
                rows.push(Row::toggle("epic:eos", tr!("EOS Overlay"), state == EosGame::Enabled).icon(Icon::Fullscreen));
            }
        }
        if cloud {
            rows.push(Row::action("epic:sync", tr!("Sync cloud saves")).icon(Icon::CloudSync));
        }
        rows.push(Row::action("epic:uninstall", tr!("Uninstall\u{2026}")).icon(Icon::DeleteForever).danger());
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
            "epic:relocate" => Some(Request::Relocate(k)),
            "epic:forget" => Some(Request::Forget(k)),
            "epic:update" => {
                self.epic_direct(key, cx);
                None
            }
            "epic:sync" => {
                if let Some(game) = self.epic_game(key) {
                    self.epic_sync_saves(&game, cx);
                }
                None
            }
            _ => None,
        };
        cx.notify();
    }
}
