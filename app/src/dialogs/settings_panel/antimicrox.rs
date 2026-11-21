//! "AntiMicroX profiles" in the Advanced tab of a game: one profile per
//! player, loaded while the game runs (see `heronite_launch::antimicrox`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_launch::antimicrox::{parse, to_value, KEY};

use super::companions::file_name;
use super::parts::{column, help, rows};
use super::SettingsPanel;
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::choose_path;
use crate::ui::icon;
use heronite_i18n::tr;

/// "Player 2": the controller `index` (from 0) gets the profile.
pub(super) fn player_label(index: usize) -> String {
    tr!("Player {number}", number = index + 1)
}

impl SettingsPanel {
    pub(super) fn antimicrox_profiles(&self) -> Vec<String> {
        parse(self.settings.get(KEY))
    }

    fn set_antimicrox_profiles(&mut self, list: &[String], cx: &mut Context<Self>) {
        self.set(KEY, to_value(list), cx);
    }

    /// Gives the next player `path`.
    pub(super) fn add_antimicrox_profile(&mut self, path: String, cx: &mut Context<Self>) {
        let mut list = self.antimicrox_profiles();
        list.push(path);
        self.set_antimicrox_profiles(&list, cx);
    }

    /// Gives player `index` (from 0) `path` instead.
    pub(super) fn replace_antimicrox_profile(&mut self, index: usize, path: String, cx: &mut Context<Self>) {
        let mut list = self.antimicrox_profiles();
        if let Some(p) = list.get_mut(index) {
            *p = path;
            self.set_antimicrox_profiles(&list, cx);
        }
    }

    /// Removes player `index`: the next ones move up a controller.
    pub(super) fn remove_antimicrox_profile(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut list = self.antimicrox_profiles();
        if index < list.len() {
            list.remove(index);
            self.set_antimicrox_profiles(&list, cx);
        }
    }

    pub(super) fn antimicrox_section(&mut self, cx: &mut Context<Self>) -> Div {
        let round = |id: String, ic: Icon| {
            div()
                .id(SharedString::from(id))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded_full()
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.10)))
                .child(icon(ic, px(16.), theme::text_muted()))
        };
        let mut list = rows();
        for (i, path) in self.antimicrox_profiles().iter().enumerate() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .child(icon(Icon::SportsEsports, px(18.), theme::text_muted()))
                    .child(div().w(px(84.)).flex_none().text_size(px(14.)).text_color(theme::text_primary()).child(player_label(i)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(file_name(path)))
                            .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(path.clone())),
                    )
                    .child(round(format!("antimicrox-change-{i}"), Icon::FolderOpen).on_click(cx.listener(move |_, _, window, cx| {
                        choose_path(window, cx, false, &tr!("Choose the AntiMicroX profile of {player}", player = player_label(i)), {
                            move |this: &mut Self, path, _, cx| this.replace_antimicrox_profile(i, path, cx)
                        });
                    })))
                    .child(
                        round(format!("antimicrox-del-{i}"), Icon::DeleteOutline)
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_antimicrox_profile(i, cx))),
                    ),
            );
        }
        let next = player_label(self.antimicrox_profiles().len());
        list = list.child(div().py(px(10.)).child(self.outline_button(
            "antimicrox-add",
            tr!("Add player"),
            true,
            cx,
            move |_, window, cx| {
                choose_path(
                    window,
                    cx,
                    false,
                    &tr!("Choose the AntiMicroX profile of {player}", player = next),
                    |this: &mut Self, path, _, cx| this.add_antimicrox_profile(path, cx),
                );
            },
        )));
        column()
            .child(self.marked_heading(tr!("AntiMicroX profiles"), "antimicrox", cx))
            .child(list.card())
            .child(help(tr!("A profile (.amgp) for each controller, loaded while the game runs: player 1's on the first controller, player 2's on the second. Needs AntiMicroX installed.")).px(px(6.)))
    }
}
