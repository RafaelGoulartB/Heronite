//! Right-click menu of a card (`CollectionContextMenu`): glass menu (radius
//! 14, items radius 10) with the status and categories flyouts.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::status::{set_game_status, SetStatusExtras};
use heronite_data::{CollectionGame, Library};

use super::game_actions::GameActionItem;
use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;
use heronite_i18n::tr;

pub(super) struct ContextMenuState {
    pub key: SharedString,
    pub position: Point<Pixels>,
    pub status_open: bool,
    /// Flyout of the Install row (the other ways to install).
    pub install_open: bool,
    /// Flyout of the Categories row.
    pub categories_open: bool,
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
        self.context_menu = Some(ContextMenuState { key, position, status_open: false, install_open: false, categories_open: false });
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
        let (position, status_open, install_open, categories_open) =
            (state.position, state.status_open, state.install_open, state.categories_open);
        let playing = self.playing.contains_key(&game.app_name);
        let viewport = window.viewport_size();
        let flyout_left = f32::from(viewport.width - position.x) < 280. + 240.;

        let mut menu = paper().min_w(px(240.));
        menu = menu.child(Self::status_row(&lib, &game, &key, status_open, flyout_left, cx));
        menu = menu.child(self.categories_row(&game, &key, categories_open, flyout_left, cx));
        menu = menu.child(self.primary_row(&game, &key, playing, install_open, flyout_left, cx));
        // Everything else comes from the game's action list (shared with the
        // command palette, see `game_actions.rs`).
        for a in self.game_actions(&game, cx).into_iter().filter(|a| a.in_menu) {
            menu = menu.child(Self::action_row(&key, a, cx));
        }
        Some(Self::menu_backdrop(menu, position, cx))
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
                        if let Some(m) = this.context_menu.as_mut() {
                            m.status_open = true;
                            m.install_open = false;
                            m.categories_open = false;
                        }
                        cx.notify();
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
                        if let Some(m) = this.context_menu.as_mut() {
                            m.categories_open = true;
                            m.status_open = false;
                            m.install_open = false;
                        }
                        cx.notify();
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
        let more = !playing && super::install_options::has_more_ways(game);
        let mut row = item("cm-primary")
            .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                if *h {
                    if let Some(m) = this.context_menu.as_mut() {
                        m.status_open = false;
                        m.install_open = more;
                        m.categories_open = false;
                    }
                    cx.notify();
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
            wrap = wrap.child(self.install_flyout(game, flyout_left, cx));
        }
        wrap
    }

    /// One plain row: hovering it folds the flyouts, a click runs the action.
    fn action_row(key: &SharedString, a: GameActionItem, cx: &mut Context<Self>) -> Stateful<Div> {
        let (k, action) = (key.clone(), a.action);
        item(SharedString::from(format!("cm-{}", a.id)))
            .on_hover(cx.listener(|this, h: &bool, _, cx| {
                if let Some(m) = this.context_menu.as_mut().filter(|m| *h && (m.status_open || m.install_open || m.categories_open)) {
                    m.status_open = false;
                    m.install_open = false;
                    m.categories_open = false;
                    cx.notify();
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
