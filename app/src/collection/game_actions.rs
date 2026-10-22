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
use heronite_i18n::tr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GameAction {
    /// Play, Stop or Install: the focus panel's main button.
    Primary,
    /// Play an Epic game with one of its ways to start (`None`: the default).
    PlayWith(Option<heronite_epic::launch_options::LaunchOption>),
    /// Runs one of the actions the user added to the game (by id).
    User(String),
    /// The game settings on the Actions tab, where the actions are edited.
    EditActions,
    /// One of the other ways to install (the Install flyout).
    InstallWay(WayKind),
    /// Sets this status (`CompletionStatus::id`).
    Status(String),
    /// Puts the game in this custom category, or takes it out.
    Category {
        name: String,
        on: bool,
    },
    Favourite,
    OpenFolder,
    Settings,
    /// The Mods manager (installed PC games).
    Mods,
    EditGame,
    /// An emulated game played as a PC game from now on (a native port).
    PlayAsPc,
    Metadata,
    Hide,
    RemoveRecent,
    Log,
    Details,
    EditPlaytime,
    Ludusavi,
    /// The after-play save backup of this game (`on` = turn it on).
    AutoBackup {
        on: bool,
    },
    SteamUninstall,
    /// The Steam copy of a game whose own copy is installed too.
    SteamPlay,
    /// Steam's install of a game owned on Steam from another entry.
    SteamInstall,
    LocalUninstall,
    RemoveFromCollection,
    /// Desktop / application menu shortcuts (`add` = create them).
    Shortcuts {
        add: bool,
    },
    /// Non-Steam game shortcut in Steam (`add` = create it).
    Steam {
        add: bool,
    },
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
    pub fn new(
        action: GameAction,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: Icon,
        keywords: &'static str,
    ) -> Self {
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
            (tr!("Stop"), Icon::Stop, "quit close kill end exit parar fechar encerrar sair")
        } else if game.installed {
            (tr!("Play"), Icon::PlayArrow, "launch run start jogar iniciar executar")
        } else {
            (tr!("Install"), Icon::Download, "download get setup instalar baixar")
        };
        v.push(item(A::Primary, "primary", label, ic, kw).palette_only());
        // The focused Epic game's other ways to start (read with its facts).
        if game.installed && !playing && self.selected.as_deref() == Some(game.key.as_str()) {
            for (i, choice) in super::launch_options::choices(&self.facts.launch_options).into_iter().enumerate() {
                if self.facts.launch_options.is_empty() {
                    break;
                }
                let label = tr!("Play with: {option}", option = super::launch_options::choice_label(&choice));
                v.push(
                    GameActionItem::new(
                        A::PlayWith(choice),
                        format!("play-with-{i}"),
                        label,
                        Icon::PlayArrow,
                        "launch option mode vr editor dlc jogar opcao modo",
                    )
                    .palette_only(),
                );
            }
        }
        for a in self.actions_of(&game.key).to_vec() {
            // One that plays waits for the game to end.
            if a.plays() && playing {
                continue;
            }
            // The context menu has them in the flyout of its Play row.
            v.push(
                GameActionItem::new(
                    A::User(a.id.clone()),
                    format!("action-{}", a.id),
                    a.name.clone(),
                    crate::ui::game_actions::icon_of(&a),
                    "action run open tool mod acao executar abrir ferramenta",
                )
                .palette_only(),
            );
        }
        v.push(
            item(A::EditActions, "edit-actions", tr!("Edit actions\u{2026}"), Icon::Edit, "actions add launch acoes adicionar iniciar")
                .palette_only(),
        );
        if !playing {
            v.extend(self.install_way_actions(game, cx));
        }

        v.push(if game.favourite {
            item(
                A::Favourite,
                "fav",
                tr!("Remove from favourites"),
                Icon::Favorite,
                "unfavourite unfavorite star desfavoritar favorito estrela",
            )
        } else {
            item(
                A::Favourite,
                "fav",
                tr!("Add to favourites"),
                Icon::FavoriteBorder,
                "favourite favorite star like favoritar favorito estrela curtir",
            )
        });
        if game.installed {
            // A folder that is gone cannot be opened (the store actions offer to find it).
            if !game.install_folder_missing() {
                v.push(item(
                    A::OpenFolder,
                    "folder",
                    tr!("Open install folder"),
                    Icon::FolderOpen,
                    "directory files browse pasta diretorio arquivos abrir",
                ));
            }
            v.push(item(
                A::Settings,
                "settings",
                tr!("Settings"),
                Icon::Tune,
                "config configure options wine proton launch configuracoes opcoes ajustes",
            ));
        }
        // Emulated games have no mods; console mode builds no manager.
        if game.installed && game.launch_kind.as_deref() != Some("emulator") && !crate::console::active() {
            v.push(item(
                A::Mods,
                "mods",
                tr!("Mods"),
                Icon::Extension,
                "mod mods nexus thunderstore plugin plugins fomod load order modificacoes complementos",
            ));
        }
        // Sideloaded programs and browser games; not Steam/emulated entries.
        if game.runner == "sideload" && !matches!(game.launch_kind.as_deref(), Some("steam-uri" | "emulator")) {
            v.push(item(
                A::EditGame,
                "edit-game",
                tr!("Edit game"),
                Icon::Edit,
                "change executable path editar alterar executavel caminho",
            ));
        }
        if game.runner == "sideload" && game.launch_kind.as_deref() == Some("emulator") {
            v.push(item(
                A::PlayAsPc,
                "as-pc",
                tr!("Play as a PC game\u{2026}"),
                Icon::SportsEsports,
                "native port pc version executable linux windows nativo versao pc executavel",
            ));
        }
        // The focus panel has its own button; the palette keeps it.
        v.push(
            item(
                A::Metadata,
                "metadata",
                tr!("Collection metadata"),
                Icon::TravelExplore,
                "igdb cover art genre description metadados capa arte genero descricao",
            )
            .palette_only(),
        );
        v.push(if game.hidden {
            item(A::Hide, "hide", tr!("Unhide"), Icon::Visibility, "show reexibir mostrar exibir")
        } else {
            item(A::Hide, "hide", tr!("Hide"), Icon::VisibilityOff, "conceal ocultar esconder")
        });
        if game.recent {
            v.push(item(
                A::RemoveRecent,
                "recent",
                tr!("Remove from recent"),
                Icon::History,
                "history forget recentes historico esquecer remover",
            ));
        }
        v.push(item(A::Log, "log", tr!("View log"), Icon::Description, "logs debug output ver registro depuracao saida"));
        v.extend(self.extras_actions(game));
        v.extend(self.steam_copy_actions(game));
        v.extend(self.epic_actions(game, cx));
        v.extend(self.system_actions(game, cx));

        if let Some(lib) = self.lib.clone() {
            for st in lib.statuses.iter().filter(|st| st.id != game.status_id) {
                let label = tr!("Status: {name}", name = heronite_data::status::status_label(&st.name));
                v.push(
                    GameActionItem::new(
                        A::Status(st.id.clone()),
                        format!("st-{}", st.id),
                        label,
                        Icon::Flag,
                        "status mark set marcar definir estado",
                    )
                    .palette_only(),
                );
            }
        }
        for name in self.categories.names() {
            let on = self.categories.contains(name, &game.runner, &game.app_name);
            let label = if on { tr!("Remove from category: {name}", name) } else { tr!("Add to category: {name}", name) };
            v.push(
                GameActionItem::new(
                    A::Category { name: name.to_string(), on: !on },
                    format!("cat-{name}"),
                    label,
                    Icon::Label,
                    "category categories tag group categoria categorias grupo",
                )
                .palette_only(),
            );
        }
        v
    }

    /// Runs `action` on the game under `key` (closing the context menu).
    pub(super) fn run_game_action(&mut self, key: &str, action: &GameAction, window: &mut Window, cx: &mut Context<Self>) {
        use GameAction as A;
        self.context_menu = None;
        match action {
            A::Primary => self.install_or_play(key, window, cx),
            A::PlayWith(choice) => self.play_with(key, choice.clone(), cx),
            A::User(id) => self.run_user_action(key, id, cx),
            A::EditActions => self.open_actions_editor(key, window, cx),
            A::InstallWay(kind) => self.run_way(key, *kind, window, cx),
            A::Status(id) => self.set_status(key, id.clone(), cx),
            A::Category { name, on } => self.set_game_category(key, name, *on, cx),
            A::Favourite => {
                self.edit_game_lists(key, cx, |paths, g| heronite_data::config_games::toggle_favourite(paths, &g.app_name, &g.title))
            }
            A::Hide => self.edit_game_lists(key, cx, |paths, g| heronite_data::config_games::toggle_hidden(paths, &g.app_name, &g.title)),
            A::RemoveRecent => self.edit_game_lists(key, cx, |paths, g| heronite_data::config_games::remove_recent(paths, &g.app_name)),
            A::OpenFolder => self.open_install_folder(key, cx),
            A::Settings => {
                let (Some(ctx), Some(g)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
                let tab = crate::dialogs::game_settings::Tab::settings_start(&g);
                let view = cx.new(|cx| crate::dialogs::game_settings::GameSettingsDialog::new(ctx, g, tab, window, cx));
                self.open_dialog(view, cx);
            }
            A::Mods => self.open_mods(key, window, cx),
            A::EditGame => {
                self.open_edit_game(key, window, cx);
            }
            A::PlayAsPc => self.open_play_as_pc(key, window, cx),
            A::Metadata => self.open_metadata_for(key, window, cx),
            A::Log => self.open_game_log(key, cx),
            A::Details => self.open_details(key, cx),
            A::EditPlaytime => self.open_playtime_settings(key, window, cx),
            A::Ludusavi => self.open_ludusavi_backup(key, cx),
            A::AutoBackup { on } => self.set_auto_backup(key, *on, window, cx),
            A::SteamUninstall => self.open_steam_uninstall(key, window, cx),
            A::SteamPlay => self.play_with_steam(key, cx),
            A::SteamInstall => self.install_with_steam(key, cx),
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
