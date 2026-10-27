//! The Launch tab's "Actions": what the user adds to the game besides Play
//! (`heronite_data::game_actions`), listed under Play in the focus panel,
//! the context menu, the palette and the console. One action is edited at a
//! time, in a card in place of its row (or under the list, for a new one).

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::game_actions::{self, Action, ActionKind};
use heronite_i18n::tr;

use super::{block, GameSettingsDialog};
use crate::dialogs::DialogEvent;
use crate::pages::{caption_with, empty_row, icon_action, list_item};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card, card_box, help_text, row, tabs, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, input_box, labeled, new_input};
use crate::ui::game_actions::{default_name, icon_of, kind_label, target_summary};
use crate::ui::icon;

pub(super) struct ActionsState {
    pub(super) list: Vec<Action>,
    pub(super) editor: Option<Editor>,
    /// Action asking "Delete?" in its row.
    pub(super) deleting: Option<String>,
    pub(super) saving: bool,
}

pub(super) struct Editor {
    /// `None`: a new action.
    pub(super) id: Option<String>,
    pub(super) name: Entity<InputState>,
    pub(super) kind: ActionKind,
    pub(super) target: Entity<InputState>,
    pub(super) args: Entity<InputState>,
    pub(super) plays: bool,
}

impl Editor {
    pub(super) fn text(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    pub(super) fn can_save(&self, cx: &App) -> bool {
        !Self::text(&self.target, cx).is_empty()
    }
}

/// The kinds, in the editor's order.
pub(super) const KINDS: [ActionKind; 3] = [ActionKind::Program, ActionKind::Command, ActionKind::Open];

/// How Play starts this game (it cannot change here except for local programs).
pub(super) fn play_summary(game: &Game) -> String {
    if let Some(client) = game.client() {
        return tr!("Starts through {client}.", client = client.name());
    }
    match (game.runner.as_str(), game.launch_kind.as_deref()) {
        _ if game.steam_launch_id.is_some() => tr!("Starts through Steam."),
        (_, Some("emulator")) => tr!("Starts the ROM in its emulator."),
        ("legendary", _) => tr!("Starts through Epic Games."),
        ("gog", _) => tr!("Starts through GOG."),
        ("nile", _) => tr!("Starts through Amazon Games."),
        _ => tr!("Starts the game's program."),
    }
    .to_string()
}

impl GameSettingsDialog {
    pub(super) fn build_actions(game: &Game, paths: &heronite_data::DataPaths) -> ActionsState {
        let list = game_actions::load(paths).remove(&game.key).unwrap_or_default();
        ActionsState { list, editor: None, deleting: None, saving: false }
    }

    /// Opens the editor on `id`, or on a new action.
    pub(super) fn edit_action(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let current = id.as_ref().and_then(|id| self.actions.list.iter().find(|a| &a.id == id)).cloned().unwrap_or_default();
        let name = new_input(window, cx, &current.name, tr!("Mod manager"));
        let target = new_input(window, cx, &current.target, "");
        let args = new_input(window, cx, &current.args, tr!("Optional"));
        for input in [&name, &target, &args] {
            cx.subscribe_in(input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        self.actions.editor = Some(Editor { id, name, kind: current.kind, target, args, plays: current.counts_as_playing });
        self.actions.deleting = None;
        cx.notify();
    }

    pub(super) fn save_action(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.actions.editor.as_ref() else { return };
        if !e.can_save(cx) {
            return;
        }
        let target = Editor::text(&e.target, cx);
        let mut name = Editor::text(&e.name, cx);
        if name.is_empty() {
            name = default_name(e.kind, &target);
        }
        let action = Action {
            id: e.id.clone().unwrap_or_else(|| game_actions::new_id(&self.actions.list)),
            name,
            kind: e.kind,
            target,
            args: if e.kind == ActionKind::Program { Editor::text(&e.args, cx) } else { String::new() },
            counts_as_playing: e.plays && e.kind != ActionKind::Open,
        };
        let mut list = self.actions.list.clone();
        match list.iter_mut().find(|a| a.id == action.id) {
            Some(slot) => *slot = action,
            None => list.push(action),
        }
        self.write_actions(list, cx);
    }

    pub(super) fn delete_action(&mut self, id: &str, cx: &mut Context<Self>) {
        let list = self.actions.list.iter().filter(|a| a.id != id).cloned().collect();
        self.write_actions(list, cx);
    }

    fn move_action(&mut self, id: &str, up: bool, cx: &mut Context<Self>) {
        let mut list = self.actions.list.clone();
        let Some(i) = list.iter().position(|a| a.id == id) else { return };
        let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < list.len()) };
        if let Some(j) = j {
            list.swap(i, j);
            self.write_actions(list, cx);
        }
    }

    /// Saves the list off the UI thread; the Collection reloads it.
    fn write_actions(&mut self, list: Vec<Action>, cx: &mut Context<Self>) {
        if self.actions.saving {
            return;
        }
        self.actions.saving = true;
        self.error.clear();
        let (paths, key) = (self.ctx.data_paths.clone(), self.game.key.clone());
        let written = list.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { game_actions::save(&paths, &key, &written) }).await;
            this.update(cx, |this, cx| {
                this.actions.saving = false;
                match res {
                    Ok(()) => {
                        this.actions.list = list;
                        this.actions.editor = None;
                        this.actions.deleting = None;
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "Play" (what it does here) for games whose start is not edited here.
    pub(super) fn play_card(&self) -> Div {
        div().flex().flex_col().child(caption(tr!("Play"))).child(card([row(
            tr!("Play"),
            Some(&play_summary(&self.game)),
            div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Default")),
        )
        .into_any_element()]))
    }

    pub(super) fn actions_section(&mut self, cx: &mut Context<Self>) -> Div {
        let busy = self.actions.saving;
        let editing_new = self.actions.editor.as_ref().is_some_and(|e| e.id.is_none());
        let add = button_sm("gs-action-add", ButtonKind::Glass, tr!("Add action"))
            .when(!busy && !editing_new, |b| b.on_click(cx.listener(|this, _, window, cx| this.edit_action(None, window, cx))))
            .when(busy || editing_new, |b| b.opacity(0.45));
        let mut col = div().flex().flex_col().child(caption_with(tr!("Other actions"), add)).child(
            help_text(tr!("Extra ways to start the game, in the menu next to Play: a mod manager, a settings tool, a script, a web page."))
                .px(px(6.))
                .pb(px(6.)),
        );
        let count = self.actions.list.len();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, a) in self.actions.list.clone().into_iter().enumerate() {
            if self.actions.editor.as_ref().is_some_and(|e| e.id.as_deref() == Some(a.id.as_str())) {
                continue;
            }
            rows.push(self.action_row(&a, i, count, cx));
        }
        if rows.is_empty() && self.actions.editor.is_none() {
            rows.push(empty_row(tr!("No actions yet.")).into_any_element());
        }
        if !rows.is_empty() {
            col = col.child(card(rows));
        }
        if self.actions.editor.is_some() {
            col = col.child(div().pt(px(8.)).child(self.action_editor(cx)));
        }
        col
    }

    fn action_row(&self, a: &Action, i: usize, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let id = a.id.clone();
        if self.actions.deleting.as_deref() == Some(id.as_str()) {
            let (keep, del) = (id.clone(), id.clone());
            return list_item(
                None,
                div().text_size(px(14.)).child(tr!("Delete \u{201c}{name}\u{201d}?", name = a.name)),
                div()
                    .flex()
                    .gap(px(8.))
                    .child(button_sm(SharedString::from(format!("gs-action-keep-{keep}")), ButtonKind::Ghost, tr!("Cancel")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.actions.deleting = None;
                            cx.notify();
                        }),
                    ))
                    .child(
                        button_sm(SharedString::from(format!("gs-action-del-{del}")), ButtonKind::Danger, tr!("Delete"))
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_action(&del, cx))),
                    ),
            )
            .into_any_element();
        }
        let leading = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(34.))
            .rounded(px(10.))
            .bg(theme::ink_a(0.06))
            .child(icon(icon_of(a), px(18.), theme::text_muted()))
            .into_any_element();
        let mut meta = vec![kind_label(a.kind).to_string()];
        if a.plays() {
            meta.push(tr!("Counts as playing").to_string());
        }
        meta.push(target_summary(a));
        let content = div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(a.name.clone()))
            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(meta.join(" \u{b7} ")));
        let free = !self.actions.saving;
        let (up, down, edit, del) = (id.clone(), id.clone(), id.clone(), id);
        let trailing = div()
            .flex()
            .gap(px(6.))
            .when(count > 1, |d| {
                d.child(
                    icon_action(SharedString::from(format!("gs-action-up-{up}")), crate::icons::Icon::ArrowUpward, free && i > 0)
                        .when(free && i > 0, |b| b.on_click(cx.listener(move |this, _, _, cx| this.move_action(&up, true, cx)))),
                )
                .child(
                    icon_action(
                        SharedString::from(format!("gs-action-down-{down}")),
                        crate::icons::Icon::ArrowDownward,
                        free && i + 1 < count,
                    )
                    .when(free && i + 1 < count, |b| b.on_click(cx.listener(move |this, _, _, cx| this.move_action(&down, false, cx)))),
                )
            })
            .child(
                icon_action(SharedString::from(format!("gs-action-edit-{edit}")), crate::icons::Icon::Edit, free)
                    .on_click(cx.listener(move |this, _, window, cx| this.edit_action(Some(edit.clone()), window, cx))),
            )
            .child(icon_action(SharedString::from(format!("gs-action-rm-{del}")), crate::icons::Icon::DeleteOutline, free).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.actions.deleting = Some(del.clone());
                    cx.notify();
                }),
            ));
        list_item(Some(leading), content, trailing).into_any_element()
    }

    fn action_editor(&self, cx: &mut Context<Self>) -> Div {
        let Some(e) = self.actions.editor.as_ref() else { return div() };
        let weak = cx.entity().downgrade();
        let kind_tabs = tabs("gs-action-kind", &KINDS.map(|k| (k, kind_label(k))), e.kind, move |k, _, cx| {
            weak.update(cx, |this, cx| {
                if let Some(e) = this.actions.editor.as_mut() {
                    e.kind = k;
                }
                cx.notify();
            })
            .ok();
        });
        let (target_label, target_help, browse) = match e.kind {
            ActionKind::Program => {
                (tr!("Program"), tr!("Linux programs run directly; .exe files run with this game's Wine or Proton."), true)
            }
            ActionKind::Command => {
                (tr!("Command"), tr!("Runs in the game's folder, like a terminal opened there. Example: ./tools/patch.sh --all"), false)
            }
            ActionKind::Open => {
                (tr!("Address, file or folder"), tr!("Opened with your default app: a web page, a manual, the mods folder..."), true)
            }
        };
        let target_field =
            div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(input_box(&e.target, false))).when(browse, |d| {
                d.child(action_button(
                    "gs-action-browse",
                    ButtonKind::Glass,
                    tr!("Browse"),
                    true,
                    cx.listener(|_, _, window, cx| {
                        choose_path(window, cx, false, tr!("Select"), |this: &mut Self, path, window, cx| {
                            if let Some(e) = this.actions.editor.as_ref() {
                                e.target.update(cx, |s, cx| s.set_value(path, window, cx));
                            }
                        })
                    }),
                ))
            });
        let mut rows: Vec<AnyElement> = vec![
            block(labeled(tr!("Name"), input_box(&e.name, false))),
            block(div().flex().flex_col().gap(px(6.)).child(caption_label(tr!("Type"))).child(kind_tabs)),
            block(div().flex().flex_col().gap(px(6.)).child(labeled(target_label, target_field)).child(help_text(target_help))),
        ];
        if e.kind == ActionKind::Program {
            rows.push(block(labeled(tr!("Arguments"), input_box(&e.args, false))));
        }
        if e.kind != ActionKind::Open {
            rows.push(
                toggle_row(
                    "gs-action-plays",
                    tr!("Counts as playing"),
                    Some(tr!("Starts like Play, with this game's settings; counts as playtime and Stop closes it.")),
                    e.plays,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(e) = this.actions.editor.as_mut() {
                        e.plays = !e.plays;
                    }
                    cx.notify();
                }))
                .into_any_element(),
            );
        }
        let can_save = e.can_save(cx) && !self.actions.saving;
        let buttons = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .py(px(12.))
            .child(action_button(
                "gs-action-cancel",
                ButtonKind::Ghost,
                tr!("Cancel"),
                !self.actions.saving,
                cx.listener(|this, _, _, cx| {
                    this.actions.editor = None;
                    cx.notify();
                }),
            ))
            .child(action_button(
                "gs-action-save",
                ButtonKind::Primary,
                if self.actions.saving { tr!("Saving…") } else { tr!("Save") },
                can_save,
                cx.listener(|this, _, _, cx| this.save_action(cx)),
            ));
        let title = if e.id.is_some() { tr!("Edit action") } else { tr!("New action") };
        let mut c = card_box().child(div().pt(px(12.)).text_size(px(15.)).font_weight(theme::W_BOLD).child(title));
        for r in rows {
            c = c.child(r);
        }
        c.child(buttons)
    }
}

fn caption_label(text: &str) -> Div {
    crate::ui::fields::field_label(text.to_string())
}
