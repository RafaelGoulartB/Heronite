//! `ManageCategoriesDialog`: add, rename and remove custom categories
//! (`heronite_data::categories`). Removing one keeps its games in the
//! library.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::{self, Categories};
use heronite_i18n::{tr, trn};

use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, button_sm, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::input_box;

struct Row {
    name: String,
    /// Games of the library in it.
    games: usize,
    input: Entity<InputState>,
}

pub struct ManageCategoriesDialog {
    ctx: DialogCtx,
    rows: Vec<Row>,
    new_name: Entity<InputState>,
    error: Option<SharedString>,
    /// The category whose removal is being confirmed.
    confirm: Option<String>,
    /// A rename to the name of another category, waiting for "Merge":
    /// `(from, into)`.
    merge: Option<(String, String)>,
}

impl EventEmitter<DialogEvent> for ManageCategoriesDialog {}

/// A category was renamed (or merged into another): the Collection follows
/// it in the filters and saved views that name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryRenamed {
    pub from: String,
    pub to: String,
}

impl EventEmitter<CategoryRenamed> for ManageCategoriesDialog {}

fn text_input(window: &mut Window, cx: &mut App, value: &str, placeholder: &str) -> Entity<InputState> {
    let value = value.to_string();
    let placeholder = placeholder.to_string();
    cx.new(|cx| {
        let mut s = InputState::new(window, cx);
        s.set_placeholder(placeholder, window, cx);
        s.set_value(value, window, cx);
        s
    })
}

impl ManageCategoriesDialog {
    pub fn new(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let new_name = text_input(window, cx, "", tr!("New category"));
        cx.subscribe_in(&new_name, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.add(window, cx);
            }
        })
        .detach();
        let mut this = Self { ctx, rows: Vec::new(), new_name, error: None, confirm: None, merge: None };
        this.load(window, cx);
        this
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cats = Categories::load(&self.ctx.data_paths);
        let lib = self.ctx.lib.clone();
        self.rows = cats
            .names()
            .map(|name| {
                let games = lib.games.iter().filter(|g| cats.contains(name, &g.runner, &g.app_name)).count();
                let input = text_input(window, cx, name, "");
                let old = name.to_string();
                cx.subscribe_in(&input, window, move |this, state, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        let new = state.read(cx).value().trim().to_string();
                        this.rename(&old, new, window, cx);
                    }
                })
                .detach();
                Row { name: name.to_string(), games, input }
            })
            .collect();
    }

    fn apply(&mut self, result: anyhow::Result<()>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.error = None;
                self.load(window, cx);
                cx.emit(DialogEvent::Changed);
            }
            Err(err) => self.error = Some(format!("{err:#}").into()),
        }
        cx.notify();
    }

    fn rename(&mut self, old: &str, new: String, window: &mut Window, cx: &mut Context<Self>) {
        if new.is_empty() || new == old {
            return;
        }
        // Another category has that name: the two would become one. Ask.
        if self.rows.iter().any(|r| r.name == new) {
            self.confirm = None;
            self.merge = Some((old.to_owned(), new));
            cx.notify();
            return;
        }
        self.do_rename(old, new, window, cx);
    }

    fn do_rename(&mut self, old: &str, new: String, window: &mut Window, cx: &mut Context<Self>) {
        self.merge = None;
        let res = categories::rename_category(&self.ctx.data_paths, old, &new);
        if res.is_ok() {
            cx.emit(CategoryRenamed { from: old.to_owned(), to: new });
        }
        self.apply(res, window, cx);
    }

    /// Esc, offered by the host before it closes the dialog: answers the
    /// question about removing or merging a category (no).
    pub fn on_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.merge.is_some() {
            self.cancel_merge(window, cx);
            return true;
        }
        let asking = self.confirm.take().is_some();
        if asking {
            cx.notify();
        }
        asking
    }

    /// The category keeps its name: its box shows it again.
    fn cancel_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((from, _)) = self.merge.take() else { return };
        if let Some(row) = self.rows.iter().find(|r| r.name == from) {
            row.input.update(cx, |s, cx| s.set_value(from.clone(), window, cx));
        }
        cx.notify();
    }

    fn delete(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        let res = categories::remove_category(&self.ctx.data_paths, &name);
        self.apply(res, window, cx);
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let res = categories::add_category(&self.ctx.data_paths, &name);
        self.new_name.update(cx, |s, cx| s.set_value("", window, cx));
        self.apply(res, window, cx);
    }
}

impl Render for ManageCategoriesDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap(px(6.));
        if self.rows.is_empty() {
            list = list.child(div().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("No categories yet.")));
        }
        for (i, row) in self.rows.iter().enumerate() {
            let name = row.name.clone();
            if let Some((from, into)) = self.merge.clone().filter(|(from, _)| *from == name) {
                let question = if row.games == 0 {
                    tr!("“{into}” already exists. Merge “{from}” into it?", into = into.as_str(), from = from.as_str())
                } else {
                    trn!(
                        row.games,
                        "“{into}” already exists. Merge “{from}” into it? Its {n} game joins “{into}”.",
                        "“{into}” already exists. Merge “{from}” into it? Its {n} games join “{into}”.",
                        into = into.as_str(),
                        from = from.as_str()
                    )
                };
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_h(px(40.))
                        .child(div().flex_1().min_w_0().text_size(px(13.5)).child(question))
                        .child(button_sm(SharedString::from(format!("cat-merge-no-{i}")), ButtonKind::Ghost, tr!("Cancel")).on_click(
                            cx.listener(|this, _, w, cx| this.cancel_merge(w, cx)),
                        ))
                        .child(
                            button_sm(SharedString::from(format!("cat-merge-yes-{i}")), ButtonKind::Danger, tr!("Merge"))
                                .on_click(cx.listener(move |this, _, w, cx| this.do_rename(&from, into.clone(), w, cx))),
                        ),
                );
                continue;
            }
            if self.confirm.as_ref() == Some(&name) {
                let question = if row.games == 0 {
                    tr!("Remove the category “{name}”?", name = name.as_str())
                } else {
                    trn!(
                        row.games,
                        "Remove the category “{name}”? Its {n} game stays in your library.",
                        "Remove the category “{name}”? Its {n} games stay in your library.",
                        name = name.as_str()
                    )
                };
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_h(px(40.))
                        .child(div().flex_1().min_w_0().text_size(px(13.5)).child(question))
                        .child(button_sm(SharedString::from(format!("cat-del-no-{i}")), ButtonKind::Ghost, tr!("Cancel")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.confirm = None;
                                cx.notify();
                            }),
                        ))
                        .child(
                            button_sm(SharedString::from(format!("cat-del-yes-{i}")), ButtonKind::Danger, tr!("Remove"))
                                .on_click(cx.listener(move |this, _, w, cx| this.delete(name.clone(), w, cx))),
                        ),
                );
                continue;
            }
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(input_box(&row.input, false)))
                    .child(div().flex_none().w(px(64.)).text_size(px(12.)).text_color(theme::text_muted()).child(trn!(
                        row.games,
                        "{n} game",
                        "{n} games"
                    )))
                    .child(crate::pages::icon_action(SharedString::from(format!("cat-del-{i}")), Icon::FaTrash, true).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.confirm = Some(name.clone());
                            cx.notify();
                        }),
                    )),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(help_text(tr!("Group games your own way. A game can be in several categories; removing a category keeps its games.")))
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(input_box(&self.new_name, false)))
                    .child(button("cat-add", ButtonKind::Glass, tr!("Add")).on_click(cx.listener(|this, _, w, cx| this.add(w, cx)))),
            )
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).text_size(px(13.)).child(e)));
        let footer = button("cat-close", ButtonKind::Primary, tr!("Done"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "manage-categories".into(), title: tr!("Manage categories").into(), width: 520., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
