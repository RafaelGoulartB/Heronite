//! The actions users add to their games (`heronite_data::game_actions`):
//! running one from the menu under Play, the context menu, the palette or
//! the console, and opening the game settings where they are edited.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_actions::Action;
use heronite_i18n::tr;

use super::context_menu::{item, place_flyout};
use super::CollectionView;
use crate::dialogs::game_settings::{GameSettingsDialog, Tab};
use crate::icons::Icon;
use crate::theme;
use crate::ui::game_actions::{icon_of, menu_note};
use crate::ui::icon;

impl CollectionView {
    /// The game's actions, in the user's order.
    pub(super) fn actions_of(&self, key: &str) -> &[Action] {
        self.user_actions.get(key).map(Vec::as_slice).unwrap_or_default()
    }

    /// Runs the action `id` of the game under `key`. One that counts as
    /// playing waits for the game (or another such action) to end.
    pub(super) fn run_user_action(&mut self, key: &str, id: &str, cx: &mut Context<Self>) {
        self.facts.launch_menu_open = false;
        self.context_menu = None;
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let Some(action) = self.actions_of(key).iter().find(|a| a.id == id).cloned() else { return };
        if action.plays() && self.playing.contains_key(&game.app_name) {
            self.show_toast(tr!("{title} is already running.", title = game.title), true, cx);
            return;
        }
        match self.runtime.run_action(&game, &action) {
            Ok(()) if !action.plays() => self.show_toast(tr!("Starting {name}\u{2026}", name = action.name), false, cx),
            Ok(()) => {}
            Err(err) => self.report_launch_error(err, cx),
        }
        cx.notify();
    }

    /// The game settings on the Launch tab, where the actions are edited.
    pub(super) fn open_actions_editor(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.facts.launch_menu_open = false;
        self.context_menu = None;
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(key)) else { return };
        let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, Tab::Launch, window, cx));
        self.open_dialog(view, cx);
    }

    /// The context menu's flyout of the Play row: the actions, then "Edit actions".
    pub(super) fn actions_flyout(&mut self, key: &SharedString, flyout_left: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut fly = crate::ui::glass::glass_menu_surface(theme::R_MENU).flex().flex_col().gap(px(2.)).p(px(6.)).min_w(px(280.));
        for a in self.actions_of(key).to_vec() {
            let (k, id) = (key.to_string(), a.id.clone());
            fly = fly.child(
                item(SharedString::from(format!("cm-action-{}", a.id)))
                    .child(icon(icon_of(&a), px(18.), theme::text_muted()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().truncate().child(a.name.clone()))
                            .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(menu_note(&a))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.run_user_action(&k, &id, cx))),
            );
        }
        let k = key.to_string();
        fly = fly.child(crate::ui::dropdown::menu_separator()).child(
            item("cm-action-edit")
                .child(icon(Icon::Edit, px(18.), theme::text_muted()))
                .child(tr!("Edit actions\u{2026}"))
                .on_click(cx.listener(move |this, _, window, cx| this.open_actions_editor(&k, window, cx))),
        );
        place_flyout(div().absolute().top(px(-6.)).occlude().child(fly), flyout_left).into_any_element()
    }
}
