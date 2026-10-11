//! What can be done to one game, as one list: the card's context menu draws
//! it and the command palette searches it, so both offer the same actions
//! under the same rules (installed, running, runner...).
//!
//! * [`CollectionView::game_actions`] lists what a game offers now, in the
//!   context menu's order. The Epic, extras and shortcut parts come from
//!   their own modules, next to the code that runs them.
//! * [`CollectionView::run_game_action`] runs one through the same paths as
//!   the buttons and menus.
//!
//! A new game action goes here (and in its module's list), never straight
//! into the menu: the palette then offers it too.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;

use super::epic::EpicAction;
use super::install_options::WayKind;
use super::CollectionView;
use crate::icons::Icon;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GameAction {
    /// Play, Stop or Install: the focus panel's main button.
    Primary,
    /// One of the other ways to install (the Install flyout).
    InstallWay(WayKind),
    /// Sets this status (`CompletionStatus::id`).
    Status(String),
    Favourite,
    OpenFolder,
    Settings,
    EditGame,
    Metadata,
    Hide,
    RemoveRecent,
    Log,
    Details,
    EditPlaytime,
    Ludusavi,
    SteamUninstall,
    LocalUninstall,
    RemoveFromCollection,
    /// Desktop / application menu shortcuts (`add` = create them).
    Shortcuts { add: bool },
    /// Non-Steam game shortcut in Steam (`add` = create it).
    Steam { add: bool },
    Epic(EpicAction),
}

#[derive(Clone, Debug)]
pub(super) struct GameActionItem {
    pub action: GameAction,
    /// Stable id: the menu row's element id (`cm-<id>`) and the palette's
    /// history key.
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Icon,
    /// Extra search words for the palette.
    pub keywords: &'static str,
    /// Drawn as a plain row of the context menu (the main button, the
    /// statuses and the install ways have their own rows and flyouts there).
    pub in_menu: bool,
}

impl GameActionItem {
    pub fn new(action: GameAction, id: impl Into<SharedString>, label: impl Into<SharedString>, icon: Icon, keywords: &'static str) -> Self {
        Self { action, id: id.into(), label: label.into(), icon, keywords, in_menu: true }
    }

    pub fn palette_only(mut self) -> Self {
        self.in_menu = false;
        self
    }
}

impl CollectionView {
    /// Everything `game` offers now: the main action, the other ways to
    /// install, the menu rows, then the statuses it can be set to.
    pub(super) fn game_actions(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Vec<GameActionItem> {
        use GameAction as A;
        let item = GameActionItem::new;
        let playing = self.playing.contains_key(&game.app_name);
        let mut v = Vec::new();

        let (label, ic, kw) = if playing {
            ("Stop", Icon::Stop, "quit close kill end exit")
        } else if game.installed {
            ("Play", Icon::PlayArrow, "launch run start")
        } else {
            ("Install", Icon::Download, "download get setup")
        };
        v.push(item(A::Primary, "primary", label, ic, kw).palette_only());
        if !playing {
            v.extend(self.install_way_actions(game, cx));
        }

        v.push(if game.favourite {
            item(A::Favourite, "fav", "Remove from favourites", Icon::Favorite, "unfavourite unfavorite star")
        } else {
            item(A::Favourite, "fav", "Add to favourites", Icon::FavoriteBorder, "favourite favorite star like")
        });
        if game.installed {
            v.push(item(A::OpenFolder, "folder", "Open install folder", Icon::FolderOpen, "directory files browse"));
            v.push(item(A::Settings, "settings", "Settings", Icon::Tune, "config configure options wine proton launch"));
        }
        // Sideloaded programs and browser games; not Steam/emulated entries.
        if game.runner == "sideload" && !matches!(game.launch_kind.as_deref(), Some("steam-uri" | "emulator")) {
            v.push(item(A::EditGame, "edit-game", "Edit game", Icon::Edit, "change executable path"));
        }
        v.push(item(A::Metadata, "metadata", "Collection metadata", Icon::TravelExplore, "igdb cover art genre description"));
        v.push(if game.hidden {
            item(A::Hide, "hide", "Unhide", Icon::Visibility, "show")
        } else {
            item(A::Hide, "hide", "Hide", Icon::VisibilityOff, "conceal")
        });
        if game.recent {
            v.push(item(A::RemoveRecent, "recent", "Remove from recent", Icon::History, "history forget"));
        }
        v.push(item(A::Log, "log", "View log", Icon::Description, "logs debug output"));
        v.extend(self.extras_actions(game));
        v.extend(self.epic_actions(game, cx));
        v.extend(self.system_actions(game, cx));

        if let Some(lib) = self.lib.clone() {
            for st in lib.statuses.iter().filter(|st| st.id != game.status_id) {
                let label = format!("Status: {}", st.name);
                v.push(GameActionItem::new(A::Status(st.id.clone()), format!("st-{}", st.id), label, Icon::Flag, "status mark set").palette_only());
            }
        }
        v
    }

    /// Runs `action` on the game under `key` (closing the context menu).
    pub(super) fn run_game_action(&mut self, key: &str, action: &GameAction, window: &mut Window, cx: &mut Context<Self>) {
        use GameAction as A;
        self.context_menu = None;
        match action {
            A::Primary => self.install_or_play(key, window, cx),
            A::InstallWay(kind) => self.run_way(key, *kind, window, cx),
            A::Status(id) => self.set_status(key, id.clone(), cx),
            A::Favourite => self.edit_game_lists(key, cx, |paths, g| heronite_data::config_games::toggle_favourite(paths, &g.app_name, &g.title)),
            A::Hide => self.edit_game_lists(key, cx, |paths, g| heronite_data::config_games::toggle_hidden(paths, &g.app_name, &g.title)),
            A::RemoveRecent => self.edit_game_lists(key, cx, |paths, g| heronite_data::config_games::remove_recent(paths, &g.app_name)),
            A::OpenFolder => self.open_install_folder(key, cx),
            A::Settings => {
                let (Some(ctx), Some(g)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
                let tab = crate::dialogs::game_settings::Tab::settings_start(&g);
                let view = cx.new(|cx| crate::dialogs::game_settings::GameSettingsDialog::new(ctx, g, tab, window, cx));
                self.open_dialog(view, cx);
            }
            A::EditGame => {
                self.open_edit_game(key, window, cx);
            }
            A::Metadata => self.open_metadata_for(key, window, cx),
            A::Log => self.open_game_log(key, cx),
            A::Details => self.open_details(key, cx),
            A::EditPlaytime => self.open_edit_playtime(key, window, cx),
            A::Ludusavi => self.open_ludusavi_backup(key, cx),
            A::SteamUninstall => self.open_steam_uninstall(key, window, cx),
            A::LocalUninstall => self.open_sideload_uninstall(key, window, cx),
            A::RemoveFromCollection => self.open_remove_game(key, cx),
            A::Shortcuts { add } => self.run_shortcuts(key, *add, cx),
            A::Steam { add } => self.run_steam_shortcut(key, *add, cx),
            A::Epic(a) => self.run_epic_action(key, *a, cx),
        }
        cx.notify();
    }

    /// Favourites, hidden and recent lists: one write, then a reload.
    fn edit_game_lists<T>(
        &mut self,
        key: &str,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&heronite_data::DataPaths, &CollectionGame) -> anyhow::Result<T>,
    ) {
        let Some(g) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        if let Err(err) = edit(&self.data_paths, &g) {
            self.show_toast(format!("{err:#}"), true, cx);
        }
        self.reload(cx);
    }
}
