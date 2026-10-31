//! The Actions tab: Play and, under it, what the user adds to the game
//! besides Play (`heronite_data::game_actions`), listed in the menu next to
//! Play, the context menu, the palette and the console. All of it is one
//! card of one-line rows; a row opens its editor in place (one at a time)
//! and the others step back until it is saved or cancelled. Rows are
//! reordered by dragging them.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::game_actions::{self, Action, ActionKind};
use heronite_i18n::tr;

use super::GameSettingsDialog;
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::pages::status_pill;
use crate::theme;
use crate::ui::controls::{action_button, card_box, help_text, icon_button_sm, tabs, toggle_row, ButtonKind};
use crate::ui::fields::{choose_path, input_box, labeled, new_input};
use crate::ui::game_actions::{default_name, icon_of, kind_label, target_summary};
use crate::ui::{icon, tooltip};

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
        self.close_play(window, cx);
        cx.notify();
    }

    /// Opens the editor on a new action of one of the empty menu's shortcuts.
    fn edit_new_action(&mut self, kind: ActionKind, name: &str, plays: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_action(None, window, cx);
        if let Some(e) = self.actions.editor.as_mut() {
            e.kind = kind;
            e.plays = plays;
            e.name.update(cx, |s, cx| s.set_value(name.to_string(), window, cx));
        }
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

    /// Puts `id` where the action at `to` is (a drop on that row).
    fn move_action_to(&mut self, id: &str, to: usize, cx: &mut Context<Self>) {
        let mut list = self.actions.list.clone();
        let Some(from) = list.iter().position(|a| a.id == id) else { return };
        if from == to {
            return;
        }
        let moved = list.remove(from);
        list.insert(to.min(list.len()), moved);
        self.write_actions(list, cx);
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

    /// The tab: Play, then the menu next to it, in one card.
    pub(super) fn actions_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let play_open = self.launch.as_ref().is_some_and(|l| l.open);
        let editing = self.actions.editor.as_ref().map(|e| e.id.clone());
        // Something is open: the other rows step back and do nothing.
        let quiet = play_open || editing.is_some();
        let free = !quiet && !self.actions.saving;
        let list = self.actions.list.clone();
        let count = list.len();

        let mut c = card_box().child(self.play_row(play_open, editing.is_some(), cx));
        if play_open {
            c = c.child(self.play_editor(cx));
        }
        c = c.child(separator()).child(menu_heading(count > 1 && free));
        for (i, a) in list.iter().enumerate() {
            if i > 0 {
                c = c.child(separator());
            }
            if editing.as_ref().is_some_and(|id| id.as_deref() == Some(a.id.as_str())) {
                c = c.child(self.open_action_row(a.name.clone().into(), icon_of(a), cx)).child(self.action_editor(cx));
            } else {
                c = c.child(self.action_row(a, i, count, free, cx));
            }
        }
        if editing.as_ref().is_some_and(|id| id.is_none()) {
            if count > 0 {
                c = c.child(separator());
            }
            c = c.child(self.open_action_row(tr!("New action").into(), Icon::Add, cx)).child(self.action_editor(cx));
        } else if count == 0 {
            c = c.child(self.empty_menu(free, cx));
        } else {
            c = c.child(separator()).child(self.add_row(free, cx));
        }
        div().flex().flex_col().child(c)
    }

    /// Play: what it starts, in one line. Local games open its editor here;
    /// the others start the way they were added (a store, an emulator...).
    fn play_row(&self, open: bool, other_open: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let editable = self.launch.is_some();
        let summary = match self.launch.as_ref() {
            Some(_) if open => tr!("How the game starts").to_string(),
            Some(l) => l.summary(cx),
            None => play_summary(&self.game),
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(tr!("Play")))
            .child(status_pill(tr!("Main"), theme::accent()));
        let trailing = match (editable, open) {
            (false, _) => icon(Icon::Lock, px(16.), theme::text_dim()),
            (true, false) => icon(Icon::ChevronRight, px(18.), theme::text_muted()),
            (true, true) => icon(Icon::ExpandMore, px(18.), theme::text_muted()),
        };
        let row = line("gs-play", tile(Icon::PlayArrow, true), title, Some(summary), trailing);
        if !editable {
            return row;
        }
        if other_open {
            return row.opacity(0.4);
        }
        clickable(row).on_click(cx.listener(move |this, _, window, cx| {
            if open {
                this.close_play(window, cx);
            } else {
                this.open_play(cx);
            }
        }))
    }

    fn action_row(&self, a: &Action, i: usize, count: usize, free: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let leading = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .when(count > 1, |d| d.child(icon(Icon::DragIndicator, px(18.), theme::ink_a(0.3))))
            .child(tile(icon_of(a), false));
        let title = div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(a.name.clone());
        let summary = format!("{} \u{b7} {}", kind_label(a.kind), target_summary(a));
        let trailing = div().flex().items_center().gap(px(8.)).when(a.plays(), |d| d.child(playtime_pill(&a.id))).child(icon(
            Icon::ChevronRight,
            px(18.),
            theme::text_muted(),
        ));
        let row = line(SharedString::from(format!("gs-action-{}", a.id)), leading, title, Some(summary), trailing);
        if !free {
            return row.opacity(0.4);
        }
        let (edit, drop) = (a.id.clone(), a.id.clone());
        clickable(row).on_click(cx.listener(move |this, _, window, cx| this.edit_action(Some(edit.clone()), window, cx))).when(
            count > 1,
            |r| {
                r.on_drag(ActionDrag { id: a.id.clone(), name: a.name.clone().into() }, |d, _, _, cx| {
                    cx.new(|_| ActionGhost(d.name.clone()))
                })
                .drag_over::<ActionDrag>(|s, _, _, _| s.bg(theme::accent_alpha(0.1)))
                .on_drop(cx.listener(move |this, d: &ActionDrag, _, cx| {
                    if d.id != drop {
                        this.move_action_to(&d.id, i, cx);
                    }
                }))
            },
        )
    }

    /// The row of the action being edited, above its editor: a click closes it.
    fn open_action_row(&self, name: SharedString, ic: Icon, cx: &mut Context<Self>) -> Stateful<Div> {
        let title = div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).truncate().child(name);
        clickable(line("gs-action-open", tile(ic, false), title, None, icon(Icon::ExpandMore, px(18.), theme::text_muted()))).on_click(
            cx.listener(|this, _, _, cx| {
                this.actions.editor = None;
                this.actions.deleting = None;
                cx.notify();
            }),
        )
    }

    fn add_row(&self, free: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let lead = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(34.))
            .rounded(px(10.))
            .bg(theme::accent_alpha(0.12))
            .child(icon(Icon::Add, px(18.), theme::accent()));
        let title = div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::accent()).child(tr!("Add to menu"));
        let row = line("gs-action-add", lead, title, None, div());
        if !free {
            return row.opacity(0.4);
        }
        clickable(row).on_click(cx.listener(|this, _, window, cx| this.edit_action(None, window, cx)))
    }

    /// No actions yet: what they are for and a shortcut per common kind.
    fn empty_menu(&self, free: bool, cx: &mut Context<Self>) -> Div {
        let shortcuts = TEMPLATES.iter().enumerate().map(|(i, t)| {
            let (kind, name, plays) = (t.kind, t.name, t.plays);
            icon_button_sm(
                SharedString::from(format!("gs-action-tpl-{i}")),
                ButtonKind::Glass,
                t.icon,
                template_text(t.label),
                free,
                cx.listener(move |this, _, window, cx| this.edit_new_action(kind, template_text(name), plays, window, cx)),
            )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .pt(px(4.))
            .pb(px(14.))
            .when(!free, |d| d.opacity(0.4))
            .child(help_text(tr!("Nothing here yet. Add a mod manager, a settings tool, a script or a web page.")))
            .child(div().flex().flex_wrap().gap(px(6.)).children(shortcuts))
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
        let saving = self.actions.saving;
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .pt(px(2.))
            .pb(px(14.))
            .child(labeled(tr!("Name"), input_box(&e.name, false)))
            .child(labeled(tr!("What it opens"), kind_tabs))
            .child(div().flex().flex_col().gap(px(6.)).child(labeled(target_label, target_field)).child(help_text(target_help)));
        if e.kind == ActionKind::Program {
            col = col.child(labeled(tr!("Arguments"), input_box(&e.args, false)));
        }
        if e.kind != ActionKind::Open {
            col = col.child(
                toggle_row(
                    "gs-action-plays",
                    tr!("Counts as playing"),
                    Some(tr!("Starts like Play, with this game's settings; counts as playtime and Stop closes it.")),
                    e.plays,
                )
                .py(px(0.))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(e) = this.actions.editor.as_mut() {
                        e.plays = !e.plays;
                    }
                    cx.notify();
                })),
            );
        }
        let confirming = e.id.as_ref().filter(|id| self.actions.deleting.as_ref() == Some(*id)).cloned();
        let footer = div().flex().items_center().gap(px(8.)).pt(px(2.));
        let footer = if let Some(id) = confirming {
            let name = self.actions.list.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap_or_default();
            footer
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(14.))
                        .text_color(theme::text_primary())
                        .child(tr!("Delete \u{201c}{name}\u{201d}?", name = name)),
                )
                .child(action_button(
                    "gs-action-keep",
                    ButtonKind::Ghost,
                    tr!("Cancel"),
                    !saving,
                    cx.listener(|this, _, _, cx| {
                        this.actions.deleting = None;
                        cx.notify();
                    }),
                ))
                .child(action_button(
                    "gs-action-del",
                    ButtonKind::Danger,
                    tr!("Delete"),
                    !saving,
                    cx.listener(move |this, _, _, cx| this.delete_action(&id, cx)),
                ))
        } else {
            let existing = e.id.clone();
            footer
                .children(existing.map(|id| {
                    action_button(
                        "gs-action-rm",
                        ButtonKind::Danger,
                        tr!("Delete"),
                        !saving,
                        cx.listener(move |this, _, _, cx| {
                            this.actions.deleting = Some(id.clone());
                            cx.notify();
                        }),
                    )
                }))
                .child(div().flex_1())
                .child(action_button(
                    "gs-action-cancel",
                    ButtonKind::Ghost,
                    tr!("Cancel"),
                    !saving,
                    cx.listener(|this, _, _, cx| {
                        this.actions.editor = None;
                        cx.notify();
                    }),
                ))
                .child(action_button(
                    "gs-action-save",
                    ButtonKind::Primary,
                    if saving { tr!("Saving…") } else { tr!("Save") },
                    e.can_save(cx) && !saving,
                    cx.listener(|this, _, _, cx| this.save_action(cx)),
                ))
        };
        col.child(footer)
    }
}

/// A shortcut of the empty menu: how the new action starts.
struct Template {
    label: &'static str,
    icon: Icon,
    kind: ActionKind,
    /// Its name ("" lets the file or address name it).
    name: &'static str,
    plays: bool,
}

const TEMPLATES: [Template; 5] = [
    Template { label: "Mod manager", icon: Icon::Build, kind: ActionKind::Program, name: "Mod manager", plays: true },
    Template { label: "Settings tool", icon: Icon::Tune, kind: ActionKind::Program, name: "", plays: false },
    Template { label: "Script", icon: Icon::Terminal, kind: ActionKind::Command, name: "", plays: false },
    Template { label: "Web page", icon: Icon::Language, kind: ActionKind::Open, name: "", plays: false },
    Template { label: "Other", icon: Icon::Add, kind: ActionKind::Program, name: "", plays: false },
];

/// The translation of a template's text ("" stays empty).
fn template_text(english: &'static str) -> &'static str {
    match english {
        "Mod manager" => tr!("Mod manager"),
        "Settings tool" => tr!("Settings tool"),
        "Script" => tr!("Script"),
        "Web page" => tr!("Web page"),
        "Other" => tr!("Other"),
        other => other,
    }
}

/// A row being dragged to another place in the menu.
struct ActionDrag {
    id: String,
    name: SharedString,
}

/// What follows the pointer while a row is dragged.
struct ActionGhost(SharedString);

impl Render for ActionGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::glass::glass_menu_surface(theme::R_ITEM)
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .text_size(px(13.))
            .text_color(theme::text_primary())
            .child(icon(Icon::DragIndicator, px(16.), theme::text_dim()))
            .child(self.0.clone())
    }
}

/// One row of the card: leading icon, a title over an optional dim line,
/// trailing bits. Bleeds 8px into the card's padding so its hover fill has room.
fn line(
    id: impl Into<ElementId>,
    leading: impl IntoElement,
    title: impl IntoElement,
    summary: Option<String>,
    trailing: impl IntoElement,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.))
        .mx(px(-8.))
        .px(px(8.))
        .py(px(11.))
        .rounded(px(theme::R_ITEM))
        .child(leading)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(title)
                .children(summary.map(|s| div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(s))),
        )
        .child(div().flex().flex_none().items_center().child(trailing))
}

fn clickable(row: Stateful<Div>) -> Stateful<Div> {
    row.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.04)))
}

/// The square icon of a row (accent for Play).
fn tile(ic: Icon, accent: bool) -> Div {
    let (bg, fg): (Hsla, Hsla) =
        if accent { (theme::accent().into(), theme::on_accent().into()) } else { (theme::ink_a(0.06), theme::text_muted().into()) };
    div().flex().flex_none().items_center().justify_center().size(px(34.)).rounded(px(10.)).bg(bg).child(icon(ic, px(18.), fg))
}

fn separator() -> Div {
    div().h(px(1.)).w_full().bg(theme::row_separator())
}

/// "In the menu next to Play", with the drag hint while rows can move.
fn menu_heading(can_drag: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .pt(px(12.))
        .pb(px(2.))
        .text_size(px(12.))
        .child(div().text_color(theme::text_muted()).child(tr!("In the menu next to Play")))
        .when(can_drag, |d| d.child(div().text_color(theme::text_dim()).child(tr!("Drag to reorder"))))
}

/// "Playtime" on an action that starts as the game's session.
fn playtime_pill(id: &str) -> Stateful<Div> {
    let c: Hsla = theme::success().into();
    div()
        .id(SharedString::from(format!("gs-action-pt-{id}")))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .bg(Hsla { a: 0.14, ..c })
        .text_size(px(11.))
        .text_color(c)
        .child(icon(Icon::AccessTime, px(12.), c))
        .child(tr!("Playtime"))
        .tooltip(tooltip(tr!("Starts like Play, with this game's settings; counts as playtime and Stop closes it.")))
}
