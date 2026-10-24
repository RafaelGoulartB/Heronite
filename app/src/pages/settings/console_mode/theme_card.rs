//! The Theme card: the built-in and saved themes as tiles, "Save as theme",
//! rename, delete, export, import and back to Classic.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::presets;
use heronite_i18n::tr;

use super::{send, SettingsPage};
use crate::console::list::{Outcome, Value};
use crate::console::theme_rows::{self, Arg, Command, StoreOp};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, card_box, ButtonKind};
use crate::ui::fields::{input_box, meta_text};

pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let st = &s.theme;
    let saved = st.data.saved.clone();
    let list = theme_rows::theme_list(&saved);
    let edited = theme_rows::is_edited(&st.prefs, &saved);
    let based_on = st.prefs.based_on.clone();
    let own = theme_rows::based_on_saved(&st.prefs, &saved).map(|t| t.name.clone());
    let notice = st.notice.clone();
    let confirm = s.confirm_delete;
    let (name_input, author_input) = (s.name.clone(), s.author.clone());

    let mut tiles = div().flex().flex_wrap().gap(px(10.));
    for (i, (id, name)) in list.iter().enumerate() {
        let current = *id == based_on;
        let about = presets::by_id(id)
            .map(|t| tr(&t.description).to_string())
            .or_else(|| saved.iter().find(|t| t.id == *id).map(|t| t.description.clone()))
            .unwrap_or_default();
        let tile = div()
            .id(SharedString::from(format!("cm-theme-{id}")))
            .flex()
            .flex_col()
            .gap(px(3.))
            .w(px(168.))
            .h(px(86.))
            .flex_none()
            .px(px(12.))
            .py(px(10.))
            .rounded(px(theme::R_ITEM))
            .border_1()
            .border_color(if current { theme::accent().into() } else { theme::card_border() })
            .bg(if current { theme::accent_tint() } else { theme::ink_a(0.03) })
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.06)))
            .on_click(cx.listener(move |d, _, window, cx| {
                send(d, Outcome::Changed("theme.use".into(), Value::Choice { options: Vec::new(), index: i }), window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).text_color(theme::text_primary()).child(name.clone()))
                    .when(current && edited, |d| {
                        d.child(div().flex_none().text_size(px(11.)).text_color(theme::accent()).child(tr!("edited")))
                    }),
            )
            .child(div().text_size(px(11.5)).line_height(px(15.)).text_color(theme::text_dim()).line_clamp(2).text_ellipsis().child(about));
        tiles = tiles.child(tile);
    }

    let save_as = action_button("cm-save-as", ButtonKind::Primary, tr!("Save as theme"), true, {
        let (name_input, author_input) = (name_input.clone(), author_input.clone());
        let entity = cx.entity().downgrade();
        move |_, window, cx| {
            let name = name_input.read(cx).value().trim().to_string();
            let author = author_input.read(cx).value().trim().to_string();
            if name.is_empty() {
                return;
            }
            entity.update(cx, |d, cx| theme_rows::store_op(d, StoreOp::SaveAs { name, author }, window, cx)).ok();
            name_input.update(cx, |s, cx| s.set_value("", window, cx));
        }
    });

    let mut actions = div().flex().flex_wrap().items_center().gap(px(8.));
    if let (true, Some(name)) = (edited, own.clone()) {
        actions = actions.child(
            button_sm("cm-save", ButtonKind::Glass, tr!("Save changes to “{name}”", name))
                .on_click(cx.listener(|d, _, window, cx| theme_rows::store_op(d, StoreOp::Save, window, cx))),
        );
    }
    if own.is_some() {
        actions = actions.child(button_sm("cm-rename", ButtonKind::Glass, tr!("Rename")).on_click(cx.listener(move |d, _, window, cx| {
            let typed = d.console_mode.as_ref().map(|s| s.name.read(cx).value().trim().to_string()).unwrap_or_default();
            if !typed.is_empty() {
                theme_rows::store_op(d, StoreOp::Rename(typed), window, cx);
            }
        })));
        actions = if confirm {
            actions
                .child(button_sm("cm-delete-yes", ButtonKind::Danger, tr!("Delete it")).on_click(cx.listener(|d, _, window, cx| {
                    if let Some(s) = d.console_mode.as_mut() {
                        s.confirm_delete = false;
                    }
                    theme_rows::store_op(d, StoreOp::Delete, window, cx);
                })))
                .child(button_sm("cm-delete-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|d, _, _, cx| {
                    if let Some(s) = d.console_mode.as_mut() {
                        s.confirm_delete = false;
                    }
                    cx.notify();
                })))
        } else {
            actions.child(button_sm("cm-delete", ButtonKind::Danger, tr!("Delete")).on_click(cx.listener(|d, _, _, cx| {
                if let Some(s) = d.console_mode.as_mut() {
                    s.confirm_delete = true;
                }
                cx.notify();
            })))
        };
    }
    actions = actions
        .child(div().flex_1())
        .child(
            button_sm("cm-export", ButtonKind::Glass, tr!("Export…"))
                .on_click(cx.listener(|d, _, window, cx| theme_rows::store_op(d, StoreOp::Export, window, cx))),
        )
        .child(
            button_sm("cm-import", ButtonKind::Glass, tr!("Import…"))
                .on_click(cx.listener(|d, _, window, cx| theme_rows::store_op(d, StoreOp::Import, window, cx))),
        )
        .child(button_sm("cm-reset", ButtonKind::Glass, tr!("Back to Classic")).on_click(cx.listener(|d, _, _, cx| {
            theme_rows::change(d, &Command::new("theme.reset", Arg::Press), cx);
        })));
    let _ = window;

    let notice_line = notice.map(|(text, error)| {
        div().text_size(px(13.)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
    });

    card_box()
        .py(px(14.))
        .gap(px(12.))
        .child(meta_text(tr!(
            "A theme holds the tabs, shelves and every option below. Yours are saved on this PC; export one to share it."
        )))
        .child(tiles)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(200.)).child(input_box(&name_input, false)))
                .child(div().w(px(160.)).child(input_box(&author_input, false)))
                .child(save_as),
        )
        .child(actions)
        .children(notice_line)
}
