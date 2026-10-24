//! Other ways to install a game that is not installed. "Install" keeps doing
//! what the game's store does (Epic downloads it, Steam opens the client);
//! the arrow next to it lists the rest:
//!
//! * the store's own install, and Epic's "Import game";
//! * "Install with Steam" for a game also owned on Steam (its Steam twin);
//! * "Install manually": the Add game dialog in [`Mode::Install`] with the
//!   game's title and art filled in, so only the files are left to choose
//!   (run a setup program in a prefix, or pick an existing executable).
//!
//! A sideloaded entry (Steam / Playnite imports, local games) is installed
//! into that same entry, so it keeps its playtime and status. A game of
//! another store gets a new local entry. A game without a store install
//! (no Steam id, not Epic: emulated games, browser games...) opens the manual
//! install from "Install" itself.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;

use super::epic::Request;
use super::CollectionView;
use crate::dialogs::add_game::{Mode, Prefill};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

/// What "Install" does today for `game`, if its store can install it.
pub(super) fn store_install_label(game: &CollectionGame) -> Option<&'static str> {
    if game.runner == "legendary" {
        Some(tr!("Install with Epic Games"))
    } else if game.runner == "gog" {
        Some(tr!("Install with GOG"))
    } else if game.runner == "nile" {
        Some(tr!("Install with Amazon Games"))
    } else if game.steam_app_id.as_deref().is_some_and(|id| !id.is_empty()) {
        Some(tr!("Install with Steam"))
    } else {
        None
    }
}

/// The game can be installed from files: every game that is not installed
/// (an emulated one becomes an executable game), except a DLC.
pub(super) fn can_install_manually(game: &CollectionGame) -> bool {
    !game.installed && game.game.install.is_dlc != Some(true)
}

/// "Install" has other ways next to it: the arrow / submenu are shown.
pub(super) fn has_more_ways(game: &CollectionGame) -> bool {
    can_install_manually(game) && store_install_label(game).is_some()
}

/// Steam's install is a way of its own: the game is owned on Steam and its
/// store installs it some other way.
pub(super) fn steam_way(game: &CollectionGame) -> bool {
    !game.installed && game.steam_installable().is_some() && store_install_label(game) != Some(tr!("Install with Steam"))
}

/// One way to install, as listed by the arrow and the context submenu.
struct Way {
    id: &'static str,
    icon: Icon,
    label: &'static str,
    desc: &'static str,
    kind: WayKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WayKind {
    Store,
    EpicImport,
    Steam,
    Manual,
}

fn prefill(game: &CollectionGame) -> Prefill {
    let pick =
        |a: &Option<String>, b: Option<&String>| a.clone().filter(|s| !s.trim().is_empty()).or_else(|| b.cloned()).unwrap_or_default();
    let art = game.art.as_deref();
    Prefill {
        app_name: (game.runner == "sideload").then(|| game.app_name.clone()),
        title: game.title.clone(),
        square: pick(&game.game.art_square, art.and_then(|a| a.cover_url.as_ref())),
        hero: pick(&game.game.art_cover, art.and_then(|a| a.hero_url.as_ref())),
        convert: false,
    }
}

impl CollectionView {
    /// "Install" of the focus panel, the context menu and console mode: the
    /// store's install, or the manual one when the store has none.
    pub(super) fn install_or_play(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let game = self.lib.as_ref().and_then(|l| l.get(key)).cloned();
        if let Some(g) = game.filter(|g| !self.playing.contains_key(&g.app_name)) {
            if store_install_label(&g).is_none() && can_install_manually(&g) {
                self.open_manual_install(key, window, cx);
                return;
            }
        }
        self.primary_action(key, cx);
    }

    /// An emulated game becomes a PC game (a native port or a PC version):
    /// the install form, converting the entry in place (playtime and status
    /// stay).
    pub(super) fn open_play_as_pc(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        if game.runner != "sideload" {
            return;
        }
        let Some(ctx) = self.dialog_ctx() else { return };
        self.context_menu = None;
        self.open_menu = None;
        let fill = Prefill { convert: true, ..prefill(&game) };
        let view = cx.new(|cx| crate::dialogs::add_game::AddGameDialog::new(ctx, Mode::Install(fill), window, cx));
        self.open_dialog(view, cx);
    }

    pub(super) fn open_manual_install(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.facts.install_menu_open = false;
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        if !can_install_manually(&game) {
            return;
        }
        let Some(ctx) = self.dialog_ctx() else { return };
        self.context_menu = None;
        self.open_menu = None;
        let view = cx.new(|cx| crate::dialogs::add_game::AddGameDialog::new(ctx, Mode::Install(prefill(&game)), window, cx));
        self.open_dialog(view, cx);
    }

    fn install_ways(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Vec<Way> {
        let mut ways = Vec::new();
        if let Some(label) = store_install_label(game) {
            ways.push(Way { id: "store", icon: Icon::Download, label, desc: tr!("Default for this game"), kind: WayKind::Store });
        }
        if self.epic_can_import(game, cx) {
            let desc = match game.runner.as_str() {
                "gog" => tr!("Use a copy of the game already on this computer"),
                "nile" => tr!("Use a copy of the game already on this computer"),
                _ => tr!("Use a copy already installed by Epic"),
            };
            ways.push(Way { id: "import", icon: Icon::FolderOpen, label: tr!("Import game"), desc, kind: WayKind::EpicImport });
        }
        if steam_way(game) {
            ways.push(Way {
                id: "steam",
                icon: Icon::SportsEsports,
                label: tr!("Install with Steam"),
                desc: tr!("You own it on Steam too"),
                kind: WayKind::Steam,
            });
        }
        if can_install_manually(game) {
            ways.push(Way {
                id: "manual",
                icon: Icon::Edit,
                label: tr!("Install manually\u{2026}"),
                desc: tr!("Run an installer or pick the game's files"),
                kind: WayKind::Manual,
            });
        }
        ways
    }

    /// The other ways as game actions (the palette; the context menu shows
    /// them in the Install flyout). The store's own install is the main
    /// action already.
    pub(super) fn install_way_actions(
        &mut self,
        game: &CollectionGame,
        cx: &mut Context<Self>,
    ) -> Vec<super::game_actions::GameActionItem> {
        use super::game_actions::{GameAction, GameActionItem};
        // Without the flyout, Epic's "Import game" is a menu row of its own.
        if !has_more_ways(game) {
            return Vec::new();
        }
        self.install_ways(game, cx)
            .into_iter()
            .filter(|w| !matches!(w.kind, WayKind::Store))
            .map(|w| {
                GameActionItem::new(GameAction::InstallWay(w.kind), format!("way-{}", w.id), w.label, w.icon, "install instalar baixar")
                    .palette_only()
            })
            .collect()
    }

    pub(super) fn run_way(&mut self, key: &str, kind: WayKind, window: &mut Window, cx: &mut Context<Self>) {
        self.facts.install_menu_open = false;
        self.context_menu = None;
        match kind {
            WayKind::Store => self.primary_action(key, cx),
            WayKind::EpicImport => self.epic.pending = Some(Request::Install { key: key.to_string(), import: true }),
            WayKind::Steam => self.install_with_steam(key, cx),
            WayKind::Manual => self.open_manual_install(key, window, cx),
        }
        cx.notify();
    }

    /// The ways as menu rows (`prefix` keeps the element ids apart).
    fn way_rows(&mut self, game: &CollectionGame, prefix: &str, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let ways = self.install_ways(game, cx);
        ways.into_iter()
            .map(|w| {
                let (key, kind) = (game.key.clone(), w.kind);
                crate::ui::dropdown::menu_item(SharedString::from(format!("{prefix}-{}", w.id)), false)
                    .child(icon(w.icon, px(18.), theme::text_muted()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(w.label)
                            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(w.desc)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.run_way(&key, kind, window, cx)))
            })
            .collect()
    }

    /// The arrow next to "Install" (only when there is more than one way).
    pub(super) fn install_more_button(&self, game: &CollectionGame, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !has_more_ways(game) {
            return None;
        }
        let open = self.facts.install_menu_open;
        Some(
            div()
                .id("focus-install-more")
                .tooltip(crate::ui::tooltip(tr!("Other ways to install")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(46.))
                .h(px(46.))
                .rounded_full()
                .bg(if open { theme::play_fill_hover() } else { theme::play_fill() })
                .hover(|s| s.bg(theme::play_fill_hover()))
                .cursor_pointer()
                .child(icon(Icon::ExpandMore, px(22.), theme::play_text()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.facts.install_menu_open = !this.facts.install_menu_open;
                    cx.notify();
                })),
        )
    }

    /// The list under the Install button, over a window-wide catcher that
    /// closes it (same layering as the status menu).
    pub(super) fn install_menu(&mut self, game: &CollectionGame, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.facts.install_menu_open {
            return None;
        }
        let menu = crate::ui::dropdown::menu_panel(280.).children(self.way_rows(game, "focus-install", cx));
        let close = cx.listener(|this, _, _, cx| {
            this.facts.install_menu_open = false;
            cx.notify();
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(
                    deferred(anchored().position(point(px(0.), px(0.))).child(
                        div().id("focus-install-catcher").w(px(20000.)).h(px(20000.)).occlude().on_mouse_down(MouseButton::Left, close),
                    ))
                    .with_priority(20),
                )
                .child(
                    div().absolute().top(px(52.)).left_0().child(
                        deferred(
                            anchored()
                                .snap_to_window_with_margin(px(8.))
                                .child(div().occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(menu)),
                        )
                        .with_priority(21),
                    ),
                )
                .into_any_element(),
        )
    }

    /// The context menu's flyout of the Install row (like the status one).
    pub(super) fn install_flyout(&mut self, game: &CollectionGame, flyout_left: bool, cx: &mut Context<Self>) -> AnyElement {
        let fly = crate::ui::glass::glass_menu_surface(theme::R_MENU)
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .min_w(px(280.))
            .children(self.way_rows(game, "cm-install", cx));
        super::context_menu::place_flyout(div().absolute().top(px(-6.)).occlude().child(fly), flyout_left).into_any_element()
    }
}
