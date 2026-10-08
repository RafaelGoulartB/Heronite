//! Right-click menu of a card (`CollectionContextMenu`): glass menu (radius
//! 14, items radius 10) with the status flyout.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::status::{set_game_status, SetStatusExtras};
use heronite_data::{CollectionGame, Library};

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub(super) struct ContextMenuState {
    pub key: SharedString,
    pub position: Point<Pixels>,
    pub status_open: bool,
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

fn paper() -> Div {
    crate::ui::glass::glass_menu_surface(theme::R_MENU).flex().flex_col().gap(px(2.)).p(px(6.))
}

impl CollectionView {
    pub(super) fn open_context_menu(&mut self, key: SharedString, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some(ContextMenuState { key, position, status_open: false });
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

    fn set_status(&mut self, key: &str, status_id: String, cx: &mut Context<Self>) {
        let Some(lib) = self.lib.clone() else { return };
        let Some(game) = lib.get(key) else { return };
        let extras = SetStatusExtras {
            runner: Some(game.runner.clone()),
            title: Some(game.title.clone()),
            playnite_id: game.meta.as_ref().map(|m| m.playnite_id.clone()),
        };
        if let Err(err) = set_game_status(&self.heroic, &game.app_name, &status_id, &extras) {
            self.show_toast(format!("Could not change status: {err:#}"), true, cx);
        }
        self.reload(cx);
    }

    pub(super) fn render_context_menu(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.context_menu.as_ref()?;
        let lib = self.lib.clone()?;
        let game = lib.get(&state.key)?.clone();
        let key = state.key.clone();
        let (position, status_open) = (state.position, state.status_open);
        let playing = self.playing.contains_key(&game.app_name);
        let viewport = window.viewport_size();
        let flyout_left = f32::from(viewport.width - position.x) < 280. + 240.;

        let mut menu = paper().min_w(px(240.));
        menu = menu.child(Self::status_row(&lib, &game, &key, status_open, flyout_left, cx));
        menu = Self::game_items(menu, &game, &key, playing, cx);
        menu = self.extras_context_items(menu, &game, cx);
        menu = self.epic_context_items(menu, &game, cx);
        menu = self.system_context_items(menu, &game, cx);
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
                .child("Status")
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(18.), theme::text_muted()))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        if let Some(m) = this.context_menu.as_mut() {
                            m.status_open = true;
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
                        .child(st.name.clone())
                        .when(st.id == current, |d| {
                            d.child(div().flex_1()).child(icon(Icon::Check, px(18.), theme::accent()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_status(&k, id.clone(), cx);
                            this.close_context_menu(cx);
                        })),
                );
            }
            let fly = div().absolute().top(px(-6.)).occlude().child(fly);
            let fly = if flyout_left { fly.right(relative(1.)).mr(px(-2.)) } else { fly.left(relative(1.)).ml(px(-2.)) };
            status_row = status_row.child(fly);
        }
        status_row
    }

    /// Play / favourite / folder / settings / metadata / hide / log items.
    fn game_items(mut menu: Div, game: &CollectionGame, key: &SharedString, playing: bool, cx: &mut Context<Self>) -> Div {
        let k = key.clone();
        let (label, ic) = if playing {
            ("Stop", Icon::Stop)
        } else if game.installed {
            ("Play", Icon::PlayArrow)
        } else {
            ("Install", Icon::Download)
        };
        menu = menu.child(
            item("cm-primary")
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        if let Some(m) = this.context_menu.as_mut() {
                            m.status_open = false;
                        }
                        cx.notify();
                    }
                }))
                .child(icon(ic, px(18.), theme::text_muted()))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.primary_action(&k, cx);
                    this.close_context_menu(cx);
                })),
        );

        let k = key.clone();
        menu = menu.child(
            item("cm-fav")
                .child(icon(if game.favourite { Icon::Favorite } else { Icon::FavoriteBorder }, px(18.), theme::text_muted()))
                .child(if game.favourite { "Remove from favourites" } else { "Add to favourites" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(g) = this.lib.as_ref().and_then(|l| l.get(&k)).cloned() {
                        if let Err(err) = heronite_data::config_games::toggle_favourite(&this.heroic, &g.app_name, &g.title) {
                            this.show_toast(format!("{err:#}"), true, cx);
                        }
                        this.reload(cx);
                    }
                    this.close_context_menu(cx);
                })),
        );

        if let Some(path) = game.details.install_path.clone() {
            menu = menu.child(
                item("cm-folder")
                    .child(icon(Icon::FolderOpen, px(18.), theme::text_muted()))
                    .child("Open install folder")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let dir = std::path::Path::new(&path);
                        let target = if dir.is_file() { dir.parent().unwrap_or(dir) } else { dir };
                        if let Err(err) = std::process::Command::new("xdg-open").arg(target).spawn() {
                            this.show_toast(format!("{err}"), true, cx);
                        }
                        this.close_context_menu(cx);
                    })),
            );
        }

        if game.installed {
            let k = key.clone();
            menu = menu.child(
                item("cm-settings")
                    .child(icon(Icon::Tune, px(18.), theme::text_muted()))
                    .child("Settings")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let (Some(ctx), Some(g)) = (this.dialog_ctx(), this.lib.as_ref().and_then(|l| l.get(&k)).cloned()) else { return };
                        let tab = crate::dialogs::game_settings::Tab::heroic_start(&g);
                        let view = cx.new(|cx| crate::dialogs::game_settings::GameSettingsDialog::new(ctx, g, tab, window, cx));
                        this.open_dialog(view, cx);
                    })),
            );
        }

        // Edit game (sideloaded programs and browser games; not Steam/emulated entries).
        if game.runner == "sideload" && !matches!(game.launch_kind.as_deref(), Some("steam-uri" | "emulator")) {
            let k = key.clone();
            menu = menu.child(
                item("cm-edit-game")
                    .child(icon(Icon::Edit, px(18.), theme::text_muted()))
                    .child("Edit game")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_edit_game(&k, window, cx);
                    })),
            );
        }

        let k = key.clone();
        menu = menu.child(
            item("cm-metadata")
                .child(icon(Icon::TravelExplore, px(18.), theme::text_muted()))
                .child("Collection metadata")
                .on_click(cx.listener(move |this, _, window, cx| this.open_metadata_for(&k, window, cx))),
        );

        let k = key.clone();
        menu = menu.child(
            item("cm-hide")
                .child(icon(if game.hidden { Icon::Visibility } else { Icon::VisibilityOff }, px(18.), theme::text_muted()))
                .child(if game.hidden { "Unhide" } else { "Hide" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(g) = this.lib.as_ref().and_then(|l| l.get(&k)).cloned() {
                        if let Err(err) = heronite_data::config_games::toggle_hidden(&this.heroic, &g.app_name, &g.title) {
                            this.show_toast(format!("{err:#}"), true, cx);
                        }
                        this.reload(cx);
                    }
                    this.close_context_menu(cx);
                })),
        );

        if game.recent {
            let k = key.clone();
            menu = menu.child(
                item("cm-recent")
                    .child(icon(Icon::History, px(18.), theme::text_muted()))
                    .child("Remove from recent")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(g) = this.lib.as_ref().and_then(|l| l.get(&k)).cloned() {
                            if let Err(err) = heronite_data::config_games::remove_recent(&this.heroic, &g.app_name) {
                                this.show_toast(format!("{err:#}"), true, cx);
                            }
                            this.reload(cx);
                        }
                        this.close_context_menu(cx);
                    })),
            );
        }

        let k = key.clone();
        menu = menu.child(
            item("cm-log")
                .child(icon(Icon::Description, px(18.), theme::text_muted()))
                .child("View log")
                .on_click(cx.listener(move |this, _, _, cx| this.open_game_log(&k, cx))),
        );
        menu
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
