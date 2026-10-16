//! An Epic game's other ways to start (`heronite_epic::launch_options`):
//! the arrow next to Play and its menu, and playing with one of them. The
//! pick is saved in the game's settings, so Play (and the palette, the
//! console, the game settings) keep using it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_epic::launch_options::{self, LaunchOption};
use heronite_i18n::tr;
use heronite_launch::launch_option::KEY;

use super::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// The menu's rows: the default start, then each option.
pub(super) fn choices(options: &[LaunchOption]) -> Vec<Option<LaunchOption>> {
    std::iter::once(None).chain(options.iter().cloned().map(Some)).collect()
}

pub(super) fn choice_label(choice: &Option<LaunchOption>) -> String {
    match choice {
        None => tr!("Default").to_string(),
        Some(o) => o.label().to_string(),
    }
}

/// The options of an installed Epic game, and the one picked (if it is still
/// one of them). Reads a few small files.
pub(super) fn options_of(paths: &heronite_data::DataPaths, game: &CollectionGame) -> (Vec<LaunchOption>, Option<LaunchOption>) {
    if game.runner != "legendary" || !game.store_installed() {
        return (Vec::new(), None);
    }
    let options = launch_options::list(paths, &game.app_name);
    if options.is_empty() {
        return (options, None);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let current = heronite_data::game_config::load_game(paths, &game.app_name, &home)
        .get(KEY)
        .and_then(LaunchOption::from_value)
        .filter(|c| options.contains(c));
    (options, current)
}

impl CollectionView {
    /// Reads the focused Epic game's ways to start off the UI thread.
    pub(super) fn load_launch_options(&mut self, key: SharedString, game: std::sync::Arc<CollectionGame>, cx: &mut Context<Self>) {
        if game.runner != "legendary" || !game.store_installed() {
            return;
        }
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let (options, current) = cx.background_spawn(async move { options_of(&paths, &game) }).await;
            this.update(cx, |this, cx| {
                if this.selected.as_ref() == Some(&key) {
                    this.facts.launch_options = options;
                    this.facts.launch_current = current;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// After a reload (the game settings may have changed the pick).
    pub(super) fn refresh_launch_options(&mut self, cx: &mut Context<Self>) {
        let (Some(key), Some(lib)) = (self.selected.clone(), self.lib.clone()) else { return };
        if let Some(game) = lib.get(&key).cloned() {
            self.load_launch_options(key, game, cx);
        }
    }

    /// Saves `choice` as the game's way to start (`None`: the default).
    pub(super) fn set_launch_option(&mut self, key: &str, choice: Option<LaunchOption>, cx: &mut Context<Self>) {
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let value = choice.as_ref().map(LaunchOption::to_value).unwrap_or(serde_json::Value::Null);
        let home = std::env::var("HOME").unwrap_or_default();
        if let Err(err) = heronite_data::game_config::set_game_settings(&self.data_paths, &game.app_name, &home, &[(KEY.into(), value)]) {
            self.show_toast(format!("{err:#}"), true, cx);
            return;
        }
        if self.selected.as_deref() == Some(key) {
            self.facts.launch_current = choice;
        }
        cx.notify();
    }

    /// Picks `choice` and plays.
    pub(super) fn play_with(&mut self, key: &str, choice: Option<LaunchOption>, cx: &mut Context<Self>) {
        self.facts.launch_menu_open = false;
        self.context_menu = None;
        self.set_launch_option(key, choice, cx);
        self.primary_action(key, cx);
    }

    /// The arrow next to Play, for an Epic game with other ways to start.
    pub(super) fn launch_more_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if self.facts.launch_options.is_empty() {
            return None;
        }
        let open = self.facts.launch_menu_open;
        Some(
            div()
                .id("focus-launch-more")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(46.))
                .h(px(46.))
                .rounded_full()
                .bg(if open { rgb(0xffffff) } else { rgb(0xf3f5f7) })
                .hover(|s| s.bg(rgb(0xffffff)))
                .cursor_pointer()
                .child(icon(Icon::ExpandMore, px(22.), rgb(0x161616)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.facts.launch_menu_open = !this.facts.launch_menu_open;
                    cx.notify();
                })),
        )
    }

    /// The ways to start under Play; the one Play uses has a check.
    pub(super) fn launch_menu(&mut self, key: &SharedString, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.facts.launch_menu_open {
            return None;
        }
        let current = self.facts.launch_current.clone();
        let rows =
            choices(&self.facts.launch_options).into_iter().enumerate().map(|(i, choice)| {
                let on = choice == current;
                let label = choice_label(&choice);
                let desc = match &choice {
                    None => tr!("How the game starts normally").to_string(),
                    Some(LaunchOption::Basic { parameters, .. }) => parameters.clone(),
                    Some(LaunchOption::Dlc { .. }) => tr!("Starts this DLC instead of the game").to_string(),
                };
                let k = key.to_string();
                crate::ui::dropdown::menu_item(SharedString::from(format!("focus-launch-{i}")), on)
                    .child(icon(
                        if on { Icon::Check } else { Icon::PlayArrow },
                        px(18.),
                        if on { theme::accent() } else { theme::text_muted() },
                    ))
                    .child(div().flex().flex_col().min_w_0().child(label).when(!desc.is_empty(), |d| {
                        d.child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(desc))
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.play_with(&k, choice.clone(), cx)))
            });
        let menu = crate::ui::dropdown::menu_panel(280.)
            .child(div().px(px(10.)).pt(px(4.)).pb(px(4.)).text_size(px(12.)).text_color(theme::text_muted()).child(tr!("Play with")))
            .children(rows);
        let close = cx.listener(|this, _, _, cx| {
            this.facts.launch_menu_open = false;
            cx.notify();
        });
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(
                    deferred(anchored().position(point(px(0.), px(0.))).child(
                        div().id("focus-launch-catcher").w(px(20000.)).h(px(20000.)).occlude().on_mouse_down(MouseButton::Left, close),
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
}
