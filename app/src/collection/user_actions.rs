//! The actions users add to their games (`heronite_data::game_actions`):
//! running one from the menu under Play, the context menu, the palette or
//! the console, and opening the game settings where they are edited.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_actions::Action;
use heronite_i18n::tr;

use super::context_menu::{item, place_flyout};
use super::CollectionView;
use crate::dialogs::game_settings::{GameSettingsDialog, Tab};
use crate::dialogs::play_choice::{Choice, PlayChoiceDialog};
use crate::icons::Icon;
use crate::theme;
use crate::ui::game_actions::{icon_of, menu_note, target_summary};
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
        if action.plays() && self.refuse_busy_launch(&game, cx) {
            return;
        }
        match self.runtime.run_action(&game, &action) {
            Ok(()) if !action.plays() => self.show_toast(tr!("Starting {name}\u{2026}", name = action.name), false, cx),
            Ok(()) => {}
            Err(err) => self.report_launch_error(err, cx),
        }
        cx.notify();
    }

    /// Play of a game set to ask (the Actions tab's "Ask how to play"),
    /// with actions that count as playing, asks which way to start it
    /// ("Choose how to start the game"). True when it asked; false when Play goes on as
    /// usual (not set to ask, no such action, not installed, running).
    pub(super) fn ask_how_to_play(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if !self.ask_on_play.contains(key) {
            return false;
        }
        let Some(lib) = self.lib.clone() else { return false };
        let Some(game) = self.copy_to_play(&lib, key).cloned() else { return false };
        let busy = self.playing.contains_key(&game.app_name) || self.preparing.contains_key(&game.app_name);
        let actions = ways_to_ask(game.installed, busy, self.actions_of(key));
        if actions.is_empty() {
            return false;
        }
        self.open_play_choice(key, &game, &actions, cx);
        true
    }

    /// "Choose how to start the game" for `game` (under `key`): Play, then `actions`.
    pub(super) fn open_play_choice(&mut self, key: &str, game: &heronite_data::CollectionGame, actions: &[Action], cx: &mut Context<Self>) {
        self.facts.launch_menu_open = false;
        self.context_menu = None;
        let choices = play_choices(game, actions);
        let last = self.last_play_choice.get(key).cloned();
        let weak = cx.entity().downgrade();
        let k = key.to_string();
        let on_play: Rc<dyn Fn(Option<String>, &mut App)> = Rc::new(move |action, cx| {
            let k = k.clone();
            weak.update(cx, |this, cx| {
                this.dialog = None;
                this.dialog_sub = None;
                this.last_play_choice.insert(k.clone(), action.clone());
                match action {
                    None => this.play_now(&k, cx),
                    Some(id) => this.run_user_action(&k, &id, cx),
                }
                cx.notify();
            })
            .ok();
        });
        let weak = cx.entity().downgrade();
        let k = key.to_string();
        let on_edit: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window, cx| {
            weak.update(cx, |this, cx| this.open_actions_editor(&k, window, cx)).ok();
        });
        let view = cx.new(|_| PlayChoiceDialog::new(game.title.clone(), choices, last, on_play, on_edit));
        self.open_dialog(view, cx);
    }

    /// The game settings on the Actions tab, where the actions are edited.
    pub(super) fn open_actions_editor(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.facts.launch_menu_open = false;
        self.context_menu = None;
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.game_for(key)) else { return };
        let view = cx.new(|cx| GameSettingsDialog::new(ctx, game, Tab::Actions, window, cx));
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

/// The actions offered besides Play when Play asks how to play: the ones that
/// count as playing, when the game is installed and neither running nor
/// starting (`busy`: Play stops it then). Empty: Play does not ask.
fn ways_to_ask(installed: bool, busy: bool, actions: &[Action]) -> Vec<Action> {
    if !installed || busy {
        return Vec::new();
    }
    actions.iter().filter(|a| a.plays()).cloned().collect()
}

/// The rows of "Choose how to start the game": the game's own Play (named
/// after its store or emulator), then `actions` (the ones that count as playing).
fn play_choices(game: &heronite_data::CollectionGame, actions: &[Action]) -> Vec<Choice> {
    let play = Choice {
        action: None,
        label: tr!("Play").to_string(),
        note: heronite_data::copies::store_label(game).to_string(),
        icon: Icon::PlayArrow,
    };
    std::iter::once(play)
        .chain(actions.iter().map(|a| Choice {
            action: Some(a.id.clone()),
            label: a.name.clone(),
            note: target_summary(a),
            icon: icon_of(a),
        }))
        .collect()
}

impl CollectionView {
    /// `HERONITE_DEBUG_DIALOG=play-choice:<key>`: "Choose how to start the game"
    /// of that game, its own actions or two made up (nothing is saved).
    pub(super) fn debug_play_choice(&mut self, key: &str, cx: &mut Context<Self>) {
        use heronite_data::game_actions::ActionKind;
        let Some(game) = self.lib.as_ref().and_then(|l| l.get(key)).cloned() else { return };
        let mut actions: Vec<Action> = self.actions_of(key).iter().filter(|a| a.plays()).cloned().collect();
        if actions.is_empty() {
            let made_up = |id: &str, name: &str, target: &str| Action {
                id: id.into(),
                name: name.into(),
                kind: ActionKind::Program,
                target: target.into(),
                counts_as_playing: true,
                ..Default::default()
            };
            actions = vec![
                made_up("recomp", "Recompiled (offline)", "/games/recomp/mkrecomp.exe"),
                made_up("online", "Online", "/games/online/launcher.exe"),
            ];
        }
        self.open_play_choice(key, &game, &actions, cx);
    }
}

#[cfg(test)]
mod tests {
    use heronite_data::game_actions::{Action, ActionKind};

    use super::ways_to_ask;

    fn action(id: &str, kind: ActionKind, plays: bool) -> Action {
        Action { id: id.into(), name: id.into(), kind, target: "/x".into(), counts_as_playing: plays, ..Default::default() }
    }

    #[test]
    fn play_asks_only_when_free_with_ways_to_play() {
        let list = [
            action("recomp", ActionKind::Program, true),
            action("tool", ActionKind::Program, false),
            action("wiki", ActionKind::Open, true),
            action("online", ActionKind::Command, true),
        ];
        let ids = |v: Vec<Action>| v.into_iter().map(|a| a.id).collect::<Vec<_>>();
        // Only the actions that start the game, in the user's order.
        assert_eq!(ids(ways_to_ask(true, false, &list)), ["recomp", "online"]);
        // Not installed, or running / starting: Play as usual.
        assert!(ways_to_ask(false, false, &list).is_empty());
        assert!(ways_to_ask(true, true, &list).is_empty());
        // Nothing that counts as playing: nothing to ask about.
        assert!(ways_to_ask(true, false, &list[1..3]).is_empty());
    }
}
