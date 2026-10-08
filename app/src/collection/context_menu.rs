//! Right-click menu of a card (`CollectionContextMenu`): MUI menu on
//! `--navbar-background`, radius 10, accent text, status flyout.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::status::{set_game_status, SetStatusExtras};

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

pub struct ContextMenuState {
    pub key: SharedString,
    pub position: Point<Pixels>,
    pub status_open: bool,
}

fn item(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(6.))
        .min_h(px(36.))
        .text_size(px(16.))
        .text_color(theme::accent())
        .cursor_pointer()
        .hover(|s| s.bg(theme::navbar_active_background()))
}

fn paper() -> Div {
    div()
        .flex()
        .flex_col()
        .py(px(8.))
        .rounded(px(10.))
        .bg(theme::navbar_background())
        .shadow(vec![BoxShadow {
            color: theme::black_a(0.45),
            offset: point(px(0.), px(8.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
            inset: false,
        }])
}

impl CollectionView {
    pub(super) fn open_context_menu(&mut self, key: SharedString, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.context_menu = Some(ContextMenuState { key, position, status_open: false });
        cx.notify();
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
        let playing = self.playing.contains_key(&game.app_name);
        let viewport = window.viewport_size();
        let flyout_left = f32::from(viewport.width - state.position.x) < 280. + 240.;

        let mut menu = paper().min_w(px(240.));

        // Status ▸
        let current = game.status_id.clone();
        let mut status_row = div().relative().child(
            item("cm-status")
                .child(icon(Icon::Flag, px(20.), rgb(0xcaf3fd)))
                .child("Status")
                .child(div().flex_1())
                .child(icon(Icon::ChevronRight, px(20.), rgb(0xcaf3fd)))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if *h {
                        if let Some(m) = this.context_menu.as_mut() {
                            m.status_open = true;
                        }
                        cx.notify();
                    }
                })),
        );
        if state.status_open {
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
        menu = menu.child(status_row);

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
                .child(icon(ic, px(20.), rgb(0xcaf3fd)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.primary_action(&k, cx);
                    this.close_context_menu(cx);
                })),
        );

        let k = key.clone();
        menu = menu.child(
            item("cm-fav")
                .child(icon(if game.favourite { Icon::Favorite } else { Icon::FavoriteBorder }, px(20.), rgb(0xcaf3fd)))
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
                    .child(icon(Icon::FolderOpen, px(20.), rgb(0xcaf3fd)))
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

        let k = key.clone();
        menu = menu.child(
            item("cm-hide")
                .child(icon(Icon::VisibilityOff, px(20.), rgb(0xcaf3fd)))
                .child("Hide")
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
                    .child(icon(Icon::History, px(20.), rgb(0xcaf3fd)))
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

        let position = state.position;
        Some(
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
            .into_any_element(),
        )
    }
}
