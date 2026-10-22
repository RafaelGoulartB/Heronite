//! Right-click menu of a card (`CollectionContextMenu`): glass menu (radius
//! 14, items radius 10) with the status, categories, install and saves
//! flyouts.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::status::{set_game_status, SetStatusExtras};
use heronite_data::{CollectionGame, Library};

use super::game_actions::{GameAction, GameActionItem};
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::{tr, trn};

pub(super) struct ContextMenuState {
    pub key: SharedString,
    pub position: Point<Pixels>,
    /// The one flyout shown, if any.
    pub open: Option<Flyout>,
    /// Opened on a selected game: the menu acts on the whole selection.
    pub batch: bool,
}

/// Rows of the menu that open a flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flyout {
    Status,
    /// The Play / Install row: the game's actions or the other ways to install.
    Install,
    Categories,
    /// "Manage installation" (the store's install actions).
    Manage,
    /// "Saves": the backup now and the after-play backup switch.
    Saves,
}

/// Groups of a game's context menu, in order: play, organize (status,
/// categories, favourite, hidden), the game, its installation, shortcuts,
/// removing it, the log, then "Select multiple".
const GROUPS: usize = 8;

/// The store's install actions (verify, modify, move...): one "Manage
/// installation" row with a flyout.
const INSTALL_GROUP: usize = 3;

/// Actions under the "Saves" row.
fn in_saves(action: &GameAction) -> bool {
    matches!(action, GameAction::Ludusavi | GameAction::AutoBackup { .. })
}

/// Group and place in the group of an action in the context menu.
fn menu_place(action: &GameAction) -> (usize, u8) {
    use super::epic::EpicAction as E;
    use GameAction as A;
    match action {
        A::Primary | A::PlayWith(_) | A::User(_) | A::EditActions | A::InstallWay(_) => (0, 0),
        A::SteamPlay => (0, 1),
        A::SteamInstall => (0, 2),
        A::Epic(E::Update) => (0, 3),
        A::Epic(E::ShowInDownloads) => (0, 4),
        A::Epic(E::Import) => (0, 5),
        A::PlayAsPc => (0, 6),
        A::Status(_) | A::Category { .. } => (1, 0),
        A::Favourite => (1, 1),
        A::Hide => (1, 2),
        A::RemoveRecent => (1, 3),
        A::Settings => (2, 0),
        A::Mods => (2, 1),
        A::EditGame => (2, 1),
        A::OpenFolder => (2, 2),
        A::Ludusavi | A::AutoBackup { .. } => (2, 3),
        A::Details | A::Metadata | A::EditPlaytime | A::Epic(E::Sync) => (2, 4),
        A::Epic(E::Verify) => (3, 0),
        A::Epic(E::Modify) => (3, 1),
        A::Epic(E::Move) => (3, 2),
        A::Epic(E::Relocate) => (3, 3),
        A::Epic(E::Eos) => (3, 4),
        A::Epic(E::Forget) => (3, 5),
        A::Shortcuts { .. } => (4, 0),
        A::Steam { .. } => (4, 1),
        A::Epic(E::Uninstall) => (5, 0),
        A::SteamUninstall => (5, 1),
        A::LocalUninstall => (5, 2),
        A::RemoveFromCollection => (5, 3),
        A::Log => (6, 0),
    }
}

pub(super) fn item(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .py(px(6.))
        .min_h(px(34.))
        .rounded(px(theme::R_ITEM))
        .text_size(px(14.))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .hover(|s| s.bg(theme::menu_item_hover()).text_color(white()))
}

/// Inner padding of the menu (and its flyouts).
const PAPER_PAD: f32 = 6.;

fn paper() -> Div {
    crate::ui::glass::glass_menu_surface(theme::R_MENU).flex().flex_col().gap(px(2.)).p(px(PAPER_PAD))
}

/// Puts an absolute flyout next to its row, clear of the menu's edge: the
/// row is inset by the menu padding, so step over it plus a small gap.
pub(super) fn place_flyout(fly: Div, left: bool) -> Div {
    let offset = px(PAPER_PAD + 4.);
    if left {
        fly.right(relative(1.)).mr(offset)
    } else {
        fly.left(relative(1.)).ml(offset)
    }
}

impl CollectionView {
    pub(super) fn open_context_menu(&mut self, key: SharedString, position: Point<Pixels>, cx: &mut Context<Self>) {
        let batch = self.sel.contains(&key);
        self.sel.pop = None;
        self.context_menu = Some(ContextMenuState { key, position, open: None, batch });
        cx.notify();
    }

    /// Opens the log viewer on the logs of the game under `key`.
    pub(super) fn open_game_log(&mut self, key: &str, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        let (app, title) = (game.app_name.clone(), game.title.clone());
        let view = cx.new(|cx| crate::dialogs::log_viewer::LogViewerDialog::new(ctx, app, title, cx));
        self.open_dialog(view, cx);
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
    }

    /// Shows `which` flyout and folds the others (`None`: folds them all).
    fn show_flyout(&mut self, which: Option<Flyout>, cx: &mut Context<Self>) {
        if let Some(m) = self.context_menu.as_mut().filter(|m| m.open != which) {
            m.open = which;
            cx.notify();
        }
    }

    pub(super) fn set_status(&mut self, key: &str, status_id: String, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(key) else { return };
        let extras = SetStatusExtras {
            runner: Some(game.runner.clone()),
            title: Some(game.title.clone()),
            playnite_id: game.meta.as_ref().map(|m| m.playnite_id.clone()),
        };
        if let Err(err) = set_game_status(&self.data_paths, &game.app_name, &status_id, &extras) {
            self.show_toast(tr!("Could not change status: {err}", err = format!("{err:#}")), true, cx);
        } else {
            self.mark_onboarding(crate::dialogs::onboarding::Step::Statuses);
        }
        self.reload(cx);
    }

    pub(super) fn render_context_menu(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.context_menu.as_ref()?;
        let lib = self.lib.clone()?;
        let game = lib.get(&state.key)?.clone();
        let key = state.key.clone();
        let (position, open) = (state.position, state.open);
        let playing = self.playing.contains_key(&game.app_name);
        let viewport = window.viewport_size();
        let flyout_left = f32::from(viewport.width - position.x) < 280. + 240.;
        if state.batch {
            let menu = self.batch_menu(open, flyout_left, cx);
            return Some(Self::menu_backdrop(menu, position, cx));
        }

        // Rows by group (see `menu_place`); separators between the groups.
        let mut groups: [Vec<AnyElement>; GROUPS] = Default::default();
        groups[0].push(self.primary_row(&game, &key, playing, open == Some(Flyout::Install), flyout_left, cx).into_any_element());
        groups[1].push(Self::status_row(&lib, &game, &key, open == Some(Flyout::Status), flyout_left, cx).into_any_element());
        groups[1].push(self.categories_row(&game, &key, open == Some(Flyout::Categories), flyout_left, cx).into_any_element());
        // Everything else comes from the game's action list (shared with the
        // command palette, see `game_actions.rs`).
        let mut actions: Vec<GameActionItem> = self.game_actions(&game, cx).into_iter().filter(|a| a.in_menu).collect();
        actions.sort_by_key(|a| menu_place(&a.action));
        let (install, actions): (Vec<_>, Vec<_>) = actions.into_iter().partition(|a| menu_place(&a.action).0 == INSTALL_GROUP);
        let (saves, actions): (Vec<_>, Vec<_>) = actions.into_iter().partition(|a| in_saves(&a.action));
        let mut saves_row = self.saves_row(&key, saves, open == Some(Flyout::Saves), flyout_left, cx);
        let saves_at = menu_place(&GameAction::Ludusavi);
        for a in actions {
            let place = menu_place(&a.action);
            // The sorted rows reach past the saves' place: put the row in.
            if place > saves_at {
                groups[saves_at.0].extend(saves_row.take().map(IntoElement::into_any_element));
            }
            groups[place.0].push(Self::action_row(&key, a, true, cx).into_any_element());
        }
        groups[saves_at.0].extend(saves_row.map(IntoElement::into_any_element));
        // The store's install actions share one row; a single one stays a plain row.
        match install.len() {
            0 => {}
            1 => groups[INSTALL_GROUP].extend(install.into_iter().map(|a| Self::action_row(&key, a, true, cx).into_any_element())),
            _ => {
                groups[INSTALL_GROUP].push(self.manage_row(&key, install, open == Some(Flyout::Manage), flyout_left, cx).into_any_element())
            }
        }
        groups[GROUPS - 1].push(self.select_row(&key, cx).into_any_element());
        let mut menu = paper().min_w(px(240.));
        for (i, rows) in groups.into_iter().filter(|g| !g.is_empty()).enumerate() {
            if i > 0 {
                menu = menu.child(crate::ui::dropdown::menu_separator());
            }
            menu = menu.children(rows);
        }
        Some(Self::menu_backdrop(menu, position, cx))
    }

    /// "Select multiple" (starts selecting with this game), or "Add to
    /// selection" while other games are selected.
    fn select_row(&self, key: &SharedString, cx: &mut Context<Self>) -> Stateful<Div> {
        let adding = self.sel.active();
        let k = key.clone();
        item("cm-select")
            .on_hover(cx.listener(|this, h: &bool, _, cx| {
                if *h {
                    this.show_flyout(None, cx);
                }
            }))
            .child(icon(if adding { Icon::Add } else { Icon::Checklist }, px(18.), theme::accent()))
            .child(if adding { tr!("Add to selection") } else { tr!("Select multiple") })
            .when(!adding, |d| d.child(div().flex_1()).child(crate::ui::controls::kbd(tr!("Ctrl click"))))
            .on_click(cx.listener(move |this, _, _, cx| this.start_selection(k.clone(), cx)))
    }

    /// The menu of a selected game: the batch changes for the whole selection.
    fn batch_menu(&mut self, open: Option<Flyout>, flyout_left: bool, cx: &mut Context<Self>) -> Div {
        let games = self.selected_games();
        let n = games.len();
        let status_fly = paper().min_w(px(230.)).max_h(px(420.)).child(
            div().id("cm-b-st-list").flex().flex_col().gap(px(2.)).overflow_y_scroll().children(self.batch_status_rows(&games, "cm-b", cx)),
        );
        let cats_fly = paper().w(px(320.)).child(self.batch_categories_body(&games, "cm-b", cx));
        let fold = |which: Option<Flyout>| {
            cx.listener(move |this: &mut Self, h: &bool, _, cx: &mut Context<Self>| {
                if *h {
                    this.show_flyout(which, cx);
                }
            })
        };
        let sub = |id: &'static str, ic: Icon, label: SharedString, open: bool, body: Div, hover| {
            let row = item(id)
                .on_hover(hover)
                .child(icon(ic, px(18.), theme::text_muted()))
                .child(label)
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(18.), theme::text_muted()));
            let mut wrap = div().relative().child(row);
            if open {
                wrap = wrap.child(place_flyout(div().absolute().top(px(-6.)).occlude().child(body), flyout_left));
            }
            wrap
        };
        let status =
            sub("cm-b-status", Icon::Flag, tr!("Status").into(), open == Some(Flyout::Status), status_fly, fold(Some(Flyout::Status)));
        let cats = sub(
            "cm-b-cats",
            Icon::Label,
            tr!("Categories").into(),
            open == Some(Flyout::Categories),
            cats_fly,
            fold(Some(Flyout::Categories)),
        );
        let all_fav = games.iter().all(|g| g.favourite);
        let all_hidden = games.iter().all(|g| g.hidden);
        let plain =
            |id: &'static str, ic: Icon, label: &str| item(id).child(icon(ic, px(18.), theme::text_muted())).child(label.to_string());
        paper()
            .min_w(px(250.))
            .child(div().px(px(12.)).pt(px(6.)).pb(px(4.)).text_size(px(12.)).text_color(theme::text_dim()).child(trn!(
                n,
                "{n} game selected",
                "{n} games selected"
            )))
            .child(status)
            .child(cats)
            .child(
                plain(
                    "cm-b-fav",
                    if all_fav { Icon::Favorite } else { Icon::FavoriteBorder },
                    if all_fav { tr!("Remove from favourites") } else { tr!("Add to favourites") },
                )
                .on_hover(fold(None))
                .on_click(cx.listener(|this, _, _, cx| this.batch_favourite(cx))),
            )
            .child(
                plain(
                    "cm-b-hide",
                    if all_hidden { Icon::Visibility } else { Icon::VisibilityOff },
                    if all_hidden { tr!("Unhide") } else { tr!("Hide") },
                )
                .on_hover(fold(None))
                .on_click(cx.listener(|this, _, _, cx| this.batch_hide(cx))),
            )
            .child(
                plain("cm-b-meta", Icon::CloudDownload, tr!("Download metadata"))
                    .on_hover(fold(None))
                    .on_click(cx.listener(|this, _, _, cx| this.batch_metadata(cx))),
            )
            .child(
                plain("cm-b-edit", Icon::Edit, tr!("Edit fields…"))
                    .child(div().flex_1())
                    .child(crate::ui::controls::kbd("F3"))
                    .on_hover(fold(None))
                    .on_click(cx.listener(|this, _, window, cx| this.open_batch_edit(window, cx))),
            )
            .child(crate::ui::dropdown::menu_separator())
            .child(
                plain("cm-b-all", Icon::SelectAll, tr!("Select all"))
                    .child(div().flex_1())
                    .child(crate::ui::controls::kbd("Ctrl A"))
                    .on_hover(fold(None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_all_shown();
                        cx.notify();
                    })),
            )
            .child(
                plain("cm-b-clear", Icon::Close, tr!("Clear selection"))
                    .child(div().flex_1())
                    .child(crate::ui::controls::kbd("Esc"))
                    .on_hover(fold(None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        this.clear_selection();
                        cx.notify();
                    })),
            )
    }

    /// "Status" row with its flyout of statuses.
    fn status_row(
        lib: &Library,
        game: &CollectionGame,
        key: &SharedString,
        status_open: bool,
        flyout_left: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let current = game.status_id.clone();
        let mut status_row = div().relative().child(
            item("cm-status")
                .child(icon(Icon::Flag, px(18.), theme::text_muted()))
                .child(tr!("Status"))
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(18.), theme::text_muted()))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        this.show_flyout(Some(Flyout::Status), cx);
                    }
                })),
        );
        if status_open {
            let mut fly = paper().py(px(6.)).min_w(px(220.)).max_h(px(420.));
            for st in lib.statuses.iter() {
                let id = st.id.clone();
                let k = key.clone();
                fly = fly.child(
                    item(SharedString::from(format!("cm-st-{}", st.id)))
                        .py(px(8.))
                        .child(div().size(px(10.)).rounded_full().bg(theme::status_color(&st.slug)))
                        .child(heronite_data::status::status_label(&st.name).to_string())
                        .when(st.id == current, |d| d.child(div().flex_1()).child(icon(Icon::Check, px(18.), theme::accent())))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_status(&k, id.clone(), cx);
                            this.close_context_menu(cx);
                        })),
                );
            }
            let fly = place_flyout(div().absolute().top(px(-6.)).occlude().child(fly), flyout_left);
            status_row = status_row.child(fly);
        }
        status_row
    }

    /// "Categories" row with its flyout: every category with a check (a
    /// click puts the game in or out, the menu stays open), then "Manage
    /// categories…".
    fn categories_row(&self, game: &CollectionGame, key: &SharedString, open: bool, flyout_left: bool, cx: &mut Context<Self>) -> Div {
        let mut row = div().relative().child(
            item("cm-categories")
                .child(icon(Icon::Label, px(18.), theme::text_muted()))
                .child(tr!("Categories"))
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(18.), theme::text_muted()))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        this.show_flyout(Some(Flyout::Categories), cx);
                    }
                })),
        );
        if !open {
            return row;
        }
        let mut fly = paper().py(px(6.)).min_w(px(220.)).max_h(px(420.));
        let mut list = div().id("cm-cat-list").flex().flex_col().gap(px(2.)).overflow_y_scroll().max_h(px(330.));
        if self.categories.is_empty() {
            list = list
                .child(div().px(px(12.)).py(px(6.)).text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No categories yet.")));
        }
        for (i, name) in self.categories.names().enumerate() {
            let on = self.categories.contains(name, &game.runner, &game.app_name);
            let (k, name) = (key.clone(), name.to_string());
            list = list.child(
                item(SharedString::from(format!("cm-cat-{i}")))
                    .py(px(8.))
                    .child(crate::ui::controls::mini_checkbox(SharedString::from(format!("cm-cat-{i}-sw")), on, false).mt(px(0.)))
                    .child(div().min_w_0().truncate().child(name.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_game_category(&k, &name, !on, cx))),
            );
        }
        fly = fly.child(list).child(
            item("cm-cat-manage")
                .py(px(8.))
                .child(icon(Icon::Add, px(18.), theme::text_muted()))
                .child(tr!("Manage categories…"))
                .on_click(cx.listener(|this, _, window, cx| this.open_manage_categories(window, cx))),
        );
        let fly = place_flyout(div().absolute().top(px(-6.)).occlude().child(fly), flyout_left);
        row = row.child(fly);
        row
    }

    /// Play / Stop / Install; a not installed game with other ways to
    /// install gets them in a flyout (same as the arrow of the focus panel).
    fn primary_row(
        &mut self,
        game: &CollectionGame,
        key: &SharedString,
        playing: bool,
        install_open: bool,
        flyout_left: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let k = key.clone();
        let (label, ic) = if playing {
            (tr!("Stop"), Icon::Stop)
        } else if game.installed {
            (tr!("Play"), Icon::PlayArrow)
        } else {
            (tr!("Install"), Icon::Download)
        };
        // Not installed: the other ways to install; installed: the game's actions.
        let ways = !playing && super::install_options::has_more_ways(game);
        let more = ways || (!playing && game.installed && !self.actions_of(key).is_empty());
        let mut row = item("cm-primary")
            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                if *h {
                    this.show_flyout(more.then_some(Flyout::Install), cx);
                }
            }))
            .child(icon(ic, px(18.), theme::text_muted()))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.close_context_menu(cx);
                this.install_or_play(&k, window, cx);
            }));
        if more {
            row = row.child(div().flex_1()).child(icon(Icon::ChevronRight, px(18.), theme::text_muted()));
        }
        let mut wrap = div().relative().child(row);
        if more && install_open {
            wrap = wrap.child(if ways { self.install_flyout(game, flyout_left, cx) } else { self.actions_flyout(key, flyout_left, cx) });
        }
        wrap
    }

    /// "Manage installation" with the store's install actions in a flyout.
    fn manage_row(&self, key: &SharedString, actions: Vec<GameActionItem>, open: bool, flyout_left: bool, cx: &mut Context<Self>) -> Div {
        let mut row = div().relative().child(
            item("cm-manage")
                .child(icon(Icon::Build, px(18.), theme::text_muted()))
                .child(tr!("Manage installation"))
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(18.), theme::text_muted()))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        this.show_flyout(Some(Flyout::Manage), cx);
                    }
                })),
        );
        if open {
            let rows: Vec<Stateful<Div>> = actions.into_iter().map(|a| Self::action_row(key, a, false, cx)).collect();
            let fly = paper().min_w(px(240.)).children(rows);
            row = row.child(place_flyout(div().absolute().top(px(-6.)).occlude().child(fly), flyout_left));
        }
        row
    }

    /// "Saves" with the backup now and the after-play backup switch in a
    /// flyout (a click on the switch keeps it open). `None` without either.
    fn saves_row(
        &self,
        key: &SharedString,
        actions: Vec<GameActionItem>,
        open: bool,
        flyout_left: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if actions.is_empty() {
            return None;
        }
        let mut row = div().relative().child(
            item("cm-saves")
                .child(icon(Icon::Save, px(18.), theme::text_muted()))
                .child(tr!("Saves"))
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(18.), theme::text_muted()))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        this.show_flyout(Some(Flyout::Saves), cx);
                    }
                })),
        );
        if !open {
            return Some(row);
        }
        let enabled = self.lib.as_ref().is_some_and(|l| l.settings.ludusavi.enabled);
        let rows = actions.into_iter().map(|a| match a.action {
            GameAction::AutoBackup { on } => {
                let k = key.clone();
                item(SharedString::from(format!("cm-{}", a.id)))
                    .py(px(8.))
                    .child(crate::ui::controls::mini_checkbox("cm-auto-backup-sw", !on, !enabled).mt(px(0.)))
                    .child(div().flex().flex_col().child(tr!("Automatic backup")).child(
                        div().text_size(px(12.)).text_color(theme::text_dim()).child(if enabled {
                            tr!("After each session of this game")
                        } else {
                            tr!("Off for every game: open Settings")
                        }),
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !enabled {
                            this.close_context_menu(cx);
                        }
                        this.set_auto_backup(&k, on, window, cx);
                    }))
            }
            _ => Self::action_row(key, a, false, cx),
        });
        let fly = paper().min_w(px(260.)).children(rows);
        row = row.child(place_flyout(div().absolute().top(px(-6.)).occlude().child(fly), flyout_left));
        Some(row)
    }

    /// One plain row: a click runs the action. In the menu itself (`fold`)
    /// hovering it folds the open flyouts; rows inside a flyout keep it open.
    fn action_row(key: &SharedString, a: GameActionItem, fold: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let (k, action) = (key.clone(), a.action);
        item(SharedString::from(format!("cm-{}", a.id)))
            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                if fold && *h {
                    this.show_flyout(None, cx);
                }
            }))
            .child(icon(a.icon, px(18.), theme::text_muted()))
            .child(a.label)
            .on_click(cx.listener(move |this, _, window, cx| this.run_game_action(&k, &action, window, cx)))
    }

    /// Full-window click catcher that closes the menu, with `menu` anchored at `position`.
    fn menu_backdrop(menu: Div, position: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        deferred(
            div()
                .id("cm-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_context_menu(cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.close_context_menu(cx)))
                .child(
                    anchored()
                        .position(position)
                        .snap_to_window_with_margin(px(8.))
                        .child(div().occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(menu)),
                ),
        )
        .with_priority(20)
        .into_any_element()
    }
}
