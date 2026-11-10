//! `ManageCategoriesDialog`: the categories of both kinds. Manual ones
//! (`heronite_data::categories`) are added, renamed, removed, coloured and
//! pinned here; smart ones (`heronite_data::smart_groups`) are listed with
//! their games, pinned here and edited in their own editor. Removing a
//! category keeps its games in the library.
//!
//! The colours and pins live with the Collection (`UiState::category_looks`,
//! `groups.json`): the dialog asks for each change with a
//! [`CategoryAction`] and shows it at once.

use std::collections::BTreeMap;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::{self, Categories};
use heronite_data::ui_state::CategoryLook;
use heronite_i18n::{tr, trn};

use super::{DialogCtx, DialogEvent};
use crate::collection::category_badges::colour_of;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, button_sm, help_text, ButtonKind};
use crate::ui::dialog::{dialog_scrolled, DialogProps};
use crate::ui::fields::input_box;
use crate::ui::{icon, tooltip};

struct Row {
    name: String,
    /// Games of the library in it.
    games: usize,
    input: Entity<InputState>,
}

/// A smart category as the manager lists it.
#[derive(Clone, Debug)]
pub struct SmartRow {
    pub id: String,
    pub name: String,
    pub colour: Rgba,
    pub pinned: bool,
    /// Games of the library in it.
    pub games: usize,
}

/// What the manager shows besides the manual categories themselves.
#[derive(Clone, Debug, Default)]
pub struct Looks {
    /// Colour and pin of the manual categories, by name.
    pub manual: BTreeMap<String, CategoryLook>,
    pub smart: Vec<SmartRow>,
}

/// A change the Collection makes for the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CategoryAction {
    /// A manual category's colour (`None`: the default one).
    Colour { name: String, colour: Option<String> },
    /// Pins a category to the header (`smart`: `key` is a smart one's id,
    /// else a manual one's name) or takes it off.
    Pin { smart: bool, key: String, pinned: bool },
    /// The editor on a smart category (`None`: a new one).
    EditSmart(Option<String>),
    /// The manual category was removed: its colour and pin go too.
    Removed(String),
}

impl EventEmitter<CategoryAction> for ManageCategoriesDialog {}

pub struct ManageCategoriesDialog {
    ctx: DialogCtx,
    rows: Vec<Row>,
    looks: Looks,
    /// The manual category whose colours are open.
    picking: Option<String>,
    new_name: Entity<InputState>,
    error: Option<SharedString>,
    /// The category whose removal is being confirmed.
    confirm: Option<String>,
    /// A rename to the name of another category, waiting for "Merge":
    /// `(from, into)`.
    merge: Option<(String, String)>,
    /// The body's scroll, for its scrollbar.
    scroll: ScrollHandle,
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
    pub fn new(ctx: DialogCtx, looks: Looks, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let new_name = text_input(window, cx, "", tr!("New category"));
        cx.subscribe_in(&new_name, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.add(window, cx);
            }
        })
        .detach();
        let mut this = Self {
            ctx,
            rows: Vec::new(),
            looks,
            picking: None,
            new_name,
            error: None,
            confirm: None,
            merge: None,
            scroll: ScrollHandle::new(),
        };
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
            if let Some(look) = self.looks.manual.remove(old) {
                self.looks.manual.entry(new.clone()).or_insert(look);
            }
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
        if res.is_ok() {
            self.looks.manual.remove(&name);
            cx.emit(CategoryAction::Removed(name.clone()));
        }
        self.apply(res, window, cx);
    }

    fn set_colour(&mut self, name: String, colour: Option<String>, cx: &mut Context<Self>) {
        self.picking = None;
        let mut look = self.looks.manual.remove(&name).unwrap_or_default();
        look.colour = colour.clone();
        if !look.is_plain() {
            self.looks.manual.insert(name.clone(), look);
        }
        cx.emit(CategoryAction::Colour { name, colour });
        cx.notify();
    }

    fn set_manual_pinned(&mut self, name: String, pinned: bool, cx: &mut Context<Self>) {
        let mut look = self.looks.manual.remove(&name).unwrap_or_default();
        look.pinned = pinned;
        if !look.is_plain() {
            self.looks.manual.insert(name.clone(), look);
        }
        cx.emit(CategoryAction::Pin { smart: false, key: name, pinned });
        cx.notify();
    }

    fn set_smart_pinned(&mut self, id: String, pinned: bool, cx: &mut Context<Self>) {
        if let Some(row) = self.looks.smart.iter_mut().find(|r| r.id == id) {
            row.pinned = pinned;
        }
        cx.emit(CategoryAction::Pin { smart: true, key: id, pinned });
        cx.notify();
    }

    /// The colour a manual category shows: its own, else its place's.
    fn colour_of(&self, i: usize, name: &str) -> Rgba {
        self.looks.manual.get(name).and_then(|l| l.colour.as_deref()).and_then(theme::parse_hex).unwrap_or_else(|| colour_of(i))
    }

    /// The default colour, then the palette smart categories pick from.
    fn render_swatches(&self, i: usize, name: &str, cx: &mut Context<Self>) -> Div {
        let current = self.looks.manual.get(name).and_then(|l| l.colour.clone());
        let swatch = |id: SharedString, fill: Rgba, colour: Option<String>, on: bool| {
            let name = name.to_owned();
            div()
                .id(id)
                .flex_none()
                .size(px(20.))
                .rounded_full()
                .bg(fill)
                .cursor_pointer()
                .border_2()
                .border_color(if on { theme::text_bright() } else { theme::ink_a(0.) })
                .on_click(cx.listener(move |this, _, _, cx| this.set_colour(name.clone(), colour.clone(), cx)))
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(34.))
            .pb(px(4.))
            .child(
                swatch(format!("cat-colour-{i}-default").into(), colour_of(i), None, current.is_none())
                    .tooltip(tooltip(tr!("Default colour"))),
            )
            .children(theme::GROUP_COLOURS.iter().enumerate().map(|(k, c)| {
                let fill = theme::parse_hex(c).unwrap_or_else(theme::accent);
                swatch(format!("cat-colour-{i}-{k}").into(), fill, Some((*c).to_owned()), current.as_deref() == Some(*c))
            }))
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
                        .child(
                            button_sm(SharedString::from(format!("cat-merge-no-{i}")), ButtonKind::Ghost, tr!("Cancel"))
                                .on_click(cx.listener(|this, _, w, cx| this.cancel_merge(w, cx))),
                        )
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
            let pinned = self.looks.manual.get(&name).is_some_and(|l| l.pinned);
            let picking = self.picking.as_ref() == Some(&name);
            let (pick_name, pin_name) = (name.clone(), name.clone());
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(SharedString::from(format!("cat-colour-{i}")))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(26.))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::ink_a(0.1)))
                            .when(picking, |s| s.bg(theme::ink_a(0.1)))
                            .child(div().size(px(12.)).rounded_full().bg(self.colour_of(i, &name)))
                            .tooltip(tooltip(tr!("Colour")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.picking = if this.picking.as_ref() == Some(&pick_name) { None } else { Some(pick_name.clone()) };
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1().min_w_0().child(input_box(&row.input, false)))
                    .child(games_label(row.games))
                    .child(
                        pin_button(SharedString::from(format!("cat-pin-{i}")), pinned)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_manual_pinned(pin_name.clone(), !pinned, cx))),
                    )
                    .child(crate::pages::icon_action(SharedString::from(format!("cat-del-{i}")), Icon::FaTrash, true).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.confirm = Some(name.clone());
                            cx.notify();
                        }),
                    )),
            );
            if picking {
                list = list.child(self.render_swatches(i, &row.name, cx));
            }
        }
        let mut smart = div().flex().flex_col().gap(px(6.));
        if self.looks.smart.is_empty() {
            smart = smart.child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(tr!("A smart category keeps itself up to date: games join and leave it as they match its rules.")),
            );
        }
        for (i, row) in self.looks.smart.iter().enumerate() {
            let (pin_id, edit_id, pinned) = (row.id.clone(), row.id.clone(), row.pinned);
            smart = smart.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_h(px(36.))
                    .child(div().flex().flex_none().justify_center().w(px(26.)).child(icon(Icon::AutoAwesome, px(15.), row.colour)))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(row.name.clone()))
                    .child(games_label(row.games))
                    .child(
                        pin_button(SharedString::from(format!("cat-smart-pin-{i}")), pinned)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_smart_pinned(pin_id.clone(), !pinned, cx))),
                    )
                    .child(
                        crate::pages::icon_action(SharedString::from(format!("cat-smart-edit-{i}")), Icon::Edit, true)
                            .tooltip(tooltip(tr!("Edit rules")))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(CategoryAction::EditSmart(Some(edit_id.clone()))))),
                    ),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(help_text(tr!(
                "Sort games your own way. You put games in a manual category; a smart category gathers the games that match its rules. A game can be in several, and removing a category keeps its games."
            )))
            .child(sub_head(Icon::Label, tr!("Manual categories")))
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().child(input_box(&self.new_name, false)))
                    .child(
                        button("cat-add", ButtonKind::Glass, tr!("New category")).on_click(cx.listener(|this, _, w, cx| this.add(w, cx))),
                    ),
            )
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).text_size(px(13.)).child(e)))
            .child(sub_head(Icon::AutoAwesome, tr!("Smart categories")))
            .child(smart)
            .child(
                div().flex().child(
                    button("cat-new-smart", ButtonKind::Glass, tr!("New smart category…"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(CategoryAction::EditSmart(None)))),
                ),
            );
        let footer = button("cat-close", ButtonKind::Primary, tr!("Done"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
            .into_any_element();
        // The body's size is known only after it is laid out: draw once more
        // so the scrollbar shows from the start.
        if self.scroll.bounds().size.height <= px(0.) {
            let view = cx.entity_id();
            cx.defer(move |cx| cx.notify(view));
        }
        let entity = cx.entity().downgrade();
        dialog_scrolled(
            DialogProps { id: "manage-categories".into(), title: tr!("Manage categories").into(), width: 560., height: None },
            &self.scroll,
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

/// A section's title in the dialog: the kind's icon and its name.
fn sub_head(ic: Icon, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mb(px(-6.))
        .text_size(px(12.5))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .child(icon(ic, px(15.), theme::text_muted()))
        .child(label)
        .child(div().flex_1().h(px(1.)).bg(theme::ink_a(0.08)))
}

fn games_label(n: usize) -> Div {
    div().flex_none().w(px(64.)).text_size(px(12.)).text_color(theme::text_muted()).child(trn!(n, "{n} game", "{n} games"))
}

/// The pin of a row: filled while the category is in the header.
fn pin_button(id: SharedString, pinned: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded_full()
        .bg(theme::ink_a(0.08))
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.14)))
        .child(if pinned {
            icon(Icon::PushPin, px(16.), theme::accent())
        } else {
            icon(Icon::PushPinOutlined, px(16.), theme::text_muted())
        })
        .tooltip(tooltip(if pinned { tr!("Unpin from the header") } else { tr!("Pin to the header") }))
}
