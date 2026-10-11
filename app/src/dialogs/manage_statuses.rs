//! `ManageStatusesDialog`: rename, reorder, add and remove completion
//! statuses (`local_library/statuses.json`).

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::CompletionStatus;
use heronite_data::status;

use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::input_box;

struct Row {
    status: CompletionStatus,
    input: Entity<InputState>,
}

pub struct ManageStatusesDialog {
    ctx: DialogCtx,
    rows: Vec<Row>,
    new_name: Entity<InputState>,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for ManageStatusesDialog {}

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

impl ManageStatusesDialog {
    pub fn new(ctx: DialogCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let new_name = text_input(window, cx, "", "New status");
        cx.subscribe_in(&new_name, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.add(window, cx);
            }
        })
        .detach();
        let statuses = ctx.lib.statuses.clone();
        let mut this = Self { ctx, rows: Vec::new(), new_name, error: None };
        this.set_statuses(statuses, window, cx);
        this
    }

    fn set_statuses(&mut self, statuses: Vec<CompletionStatus>, window: &mut Window, cx: &mut Context<Self>) {
        self.rows = statuses
            .into_iter()
            .map(|status| {
                let input = text_input(window, cx, &status.name, "");
                let id = status.id.clone();
                cx.subscribe_in(&input, window, move |this, state, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        let name = state.read(cx).value().trim().to_string();
                        this.rename(&id, name, window, cx);
                    }
                })
                .detach();
                Row { status, input }
            })
            .collect();
    }

    fn apply(&mut self, result: anyhow::Result<Vec<CompletionStatus>>, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(list) => {
                self.error = None;
                self.set_statuses(list, window, cx);
                cx.emit(DialogEvent::Changed);
            }
            Err(err) => self.error = Some(format!("{err:#}").into()),
        }
        cx.notify();
    }

    fn rename(&mut self, id: &str, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|r| r.status.id == id) else { return };
        if name.is_empty() || name == row.status.name {
            return;
        }
        let mut st = row.status.clone();
        st.name = name;
        let res = status::upsert_status(&self.ctx.data_paths, st);
        self.apply(res, window, cx);
    }

    fn move_row(&mut self, index: usize, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let other = if up { index.checked_sub(1) } else { Some(index + 1) };
        let Some(other) = other.filter(|o| *o < self.rows.len()) else { return };
        let mut ids: Vec<String> = self.rows.iter().map(|r| r.status.id.clone()).collect();
        ids.swap(index, other);
        let res = status::reorder_statuses(&self.ctx.data_paths, &ids);
        self.apply(res, window, cx);
    }

    fn delete(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.rows.len() <= 1 {
            return;
        }
        let res = status::delete_status(&self.ctx.data_paths, &id);
        self.apply(res, window, cx);
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let current: Vec<CompletionStatus> = self.rows.iter().map(|r| r.status.clone()).collect();
        let st = status::new_custom_status(&name, &current);
        let res = status::upsert_status(&self.ctx.data_paths, st);
        self.new_name.update(cx, |s, cx| s.set_value("", window, cx));
        self.apply(res, window, cx);
    }
}

fn field(input: &Entity<InputState>) -> Div {
    div().flex_1().min_w_0().child(input_box(input, false))
}

fn icon_btn(id: impl Into<ElementId>, ic: Icon, enabled: bool) -> Stateful<Div> {
    crate::pages::icon_action(id, ic, enabled)
}

impl Render for ManageStatusesDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.rows.len();
        let mut list = div().flex().flex_col().gap(px(6.));
        for (i, row) in self.rows.iter().enumerate() {
            let id = row.status.id.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().size(px(7.)).rounded_full().opacity(0.9).bg(theme::status_color(&row.status.slug)))
                    .child(field(&row.input))
                    .child(
                        icon_btn(SharedString::from(format!("st-up-{i}")), Icon::FaChevronUp, i > 0)
                            .on_click(cx.listener(move |this, _, w, cx| this.move_row(i, true, w, cx))),
                    )
                    .child(
                        icon_btn(SharedString::from(format!("st-down-{i}")), Icon::FaChevronDown, i + 1 < n)
                            .on_click(cx.listener(move |this, _, w, cx| this.move_row(i, false, w, cx))),
                    )
                    .child(
                        icon_btn(SharedString::from(format!("st-del-{i}")), Icon::FaTrash, n > 1)
                            .on_click(cx.listener(move |this, _, w, cx| this.delete(id.clone(), w, cx))),
                    ),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(help_text("Rename, reorder, or remove statuses. Games on a removed status move to another one."))
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(field(&self.new_name))
                    .child(button("st-add", ButtonKind::Glass, "Add").on_click(cx.listener(|this, _, w, cx| this.add(w, cx)))),
            )
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).text_size(px(13.)).child(e)));
        let footer = button("st-close", ButtonKind::Primary, "Done")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "manage-statuses".into(), title: "Manage statuses".into(), width: 520., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
