//! The editor (every option of the theme in use, in the groups of the
//! console's Customize panel, under its live picture, with its name and
//! author) and New theme (where to start from, a name).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{presets, ConsoleTheme};
use heronite_i18n::{tr, trn};

use super::home::{picture, shown_name, tabs_line, Status};
use super::{cards, column_width, SettingsPage, View};
use crate::console::theme_rows::{self, Arg, Card, Command, StoreOp, PANEL};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, button_sm, caption, card_box, dialog_title, help_text, icon_button_sm, tabs, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text};

/// Opens the editor on the theme in use; the name and author fields start
/// from the saved theme it is based on (empty for a built-in one).
pub(super) fn open(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(st) = Status::of(d) else { return };
    let Some(s) = d.console_mode.as_mut() else { return };
    let (name, author) = st.own.map(|t| (t.name, t.author)).unwrap_or_default();
    s.view = View::Editor;
    s.pending = None;
    s.theme.notice = None;
    s.name.update(cx, |i, cx| i.set_value(name, window, cx));
    s.author.update(cx, |i, cx| i.set_value(author, window, cx));
    cx.notify();
}

fn back_button(cx: &mut Context<SettingsPage>) -> Stateful<Div> {
    icon_button_sm("cm-back", ButtonKind::Ghost, Icon::ChevronLeft, tr!("Themes"), true, {
        let entity = cx.entity().downgrade();
        move |_, _, cx| {
            entity
                .update(cx, |d, cx| {
                    if let Some(s) = d.console_mode.as_mut() {
                        s.view = View::Home;
                        s.theme.notice = None;
                    }
                    cx.notify();
                })
                .ok();
        }
    })
}

/// The name and author typed in the fields.
fn typed(d: &SettingsPage, cx: &App) -> (String, String) {
    d.console_mode
        .as_ref()
        .map(|s| (s.name.read(cx).value().trim().to_string(), s.author.read(cx).value().trim().to_string()))
        .unwrap_or_default()
}

fn ask_for_a_name(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if let Some(s) = d.console_mode.as_mut() {
        s.theme.notice = Some((tr!("Give the theme a name first.").into(), true));
    }
    cx.notify();
}

pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(st) = Status::of(d) else { return div() };
    let col_w = column_width(window);
    let group = d.console_mode.as_ref().map_or(0, |s| s.group).min(PANEL.len() - 1);

    let groups: Vec<(usize, &'static str)> = PANEL.iter().enumerate().map(|(i, (name, _))| (i, tr(name))).collect();
    let entity = cx.entity().downgrade();
    let bar = tabs("cm-groups", &groups, group, move |i, _, cx| {
        entity
            .update(cx, |d, cx| {
                if let Some(s) = d.console_mode.as_mut() {
                    s.group = i;
                    s.open = None;
                }
                cx.notify();
            })
            .ok();
    });
    // The theme card is the header above; startup is not part of a theme.
    let list: Vec<Card> = PANEL[group].1.iter().copied().filter(|c| !matches!(c, Card::Theme | Card::Startup)).collect();
    let (body, hidden) = cards(d, &list, window, cx);
    let hidden_note = (hidden > 0).then(|| {
        div().px(px(6.)).pt(px(10.)).child(meta_text(trn!(
            hidden,
            "{n} option is hidden: no tab uses a layout it applies to.",
            "{n} options are hidden: no tab uses a layout they apply to.",
            n = hidden
        )))
    });

    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(div().flex().child(back_button(cx)))
        .child(identity(d, &st, col_w, window, cx))
        .child(div().pt(px(6.)).child(bar))
        .child(body)
        .children(hidden_note)
}

/// The picture, the name and author, saving, undoing and exporting.
fn identity(d: &mut SettingsPage, st: &Status, col_w: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let wide = col_w >= 700.;
    let pic_w = if wide { (col_w * 0.5).floor() } else { col_w - 34. };
    let pic = picture(d, &st.theme, pic_w, window);
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let (name_input, author_input) = (s.name.clone(), s.author.clone());
    let notice = s.theme.notice.clone();

    let status = match (&st.own, st.edited, &st.base_name) {
        (Some(_), false, _) => tr!("Saved on this PC.").to_string(),
        (Some(_), true, _) => tr!("Changed since it was saved.").to_string(),
        (None, false, Some(b)) => tr!("{name} is built in: save a copy to make it yours.", name = b.clone()),
        (None, true, Some(b)) => tr!("Changed from {name}. Save it as a theme to keep these changes.", name = b.clone()),
        (None, _, None) => tr!("Not saved as a theme.").to_string(),
    };
    let own = st.own.clone();
    let save = if let Some(t) = own {
        button("cm-editor-save", ButtonKind::Primary, tr!("Save")).on_click(cx.listener(move |d, _, window, cx| {
            let (name, author) = typed(d, cx);
            let name = if name.is_empty() { t.name.clone() } else { name };
            theme_rows::store_op(d, StoreOp::SaveDetails { name, author }, window, cx);
        }))
    } else {
        button("cm-editor-save-as", ButtonKind::Primary, tr!("Save as a new theme")).on_click(cx.listener(|d, _, window, cx| {
            let (name, author) = typed(d, cx);
            if name.is_empty() {
                ask_for_a_name(d, cx);
                return;
            }
            theme_rows::store_op(d, StoreOp::SaveAs { name, author, description: String::new() }, window, cx);
        }))
    };
    let mut actions = div().flex().flex_wrap().items_center().gap(px(8.)).child(save);
    if st.edited && st.base_name.is_some() {
        actions =
            actions.child(button("cm-editor-revert", ButtonKind::Ghost, tr!("Undo changes")).on_click(cx.listener(|d, _, window, cx| {
                theme_rows::run_press(d, "theme.revert", cx);
                open(d, window, cx);
            })));
    }
    actions = actions.child(div().flex_1()).child(
        button_sm("cm-export", ButtonKind::Glass, tr!("Export…"))
            .on_click(cx.listener(|d, _, window, cx| theme_rows::store_op(d, StoreOp::Export, window, cx))),
    );
    let notice_line = notice.map(|(text, error)| {
        div().text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
    });
    let fields = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(dialog_title(st.name()).truncate())
                .when(st.edited, |d| d.child(meta_text(tr!("Changed")))),
        )
        .child(meta_text(status))
        .child(labeled(tr!("Name"), input_box(&name_input, false)))
        .child(labeled(tr!("Author"), input_box(&author_input, false)))
        .child(actions)
        .children(notice_line)
        .child(meta_text(tr!("Every change shows in console mode at once. Saving keeps a copy to come back to, export or share.")));
    let body = if wide {
        div().flex().items_start().gap(px(20.)).child(div().flex_none().child(pic)).child(fields)
    } else {
        div().flex().flex_col().gap(px(16.)).child(pic).child(fields)
    };
    card_box().p(px(16.)).child(body)
}

/// A theme New theme can start from: `None` is the theme in use, changes and all.
fn start_tile(
    d: &mut SettingsPage,
    t: &ConsoleTheme,
    id: Option<String>,
    label: String,
    w: f32,
    on: bool,
    window: &Window,
    cx: &mut Context<SettingsPage>,
) -> Stateful<Div> {
    let pic = picture(d, t, w - 12., window);
    let key = id.clone().unwrap_or_else(|| "in-use".into());
    div()
        .id(SharedString::from(format!("cm-start-{key}")))
        .flex()
        .flex_col()
        .gap(px(8.))
        .w(px(w))
        .flex_none()
        .p(px(5.))
        .pb(px(8.))
        .rounded(px(theme::R_ITEM + 4.))
        .border_1()
        .border_color(if on { theme::accent().into() } else { theme::card_border() })
        .bg(if on { theme::accent_tint() } else { theme::card_fill() })
        .cursor_pointer()
        .hover(|s| s.border_color(theme::accent_alpha(0.7)))
        .on_click(cx.listener(move |d, _, _, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.start_from = id.clone();
            }
            cx.notify();
        }))
        .child(pic)
        .child(
            div()
                .px(px(4.))
                .flex()
                .flex_col()
                .child(div().text_size(px(13.)).text_color(theme::text_primary()).truncate().child(label))
                .child(div().text_size(px(11.)).text_color(theme::text_dim()).truncate().child(tabs_line(t))),
        )
}

pub(super) fn render_create(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(st) = Status::of(d) else { return div() };
    let col_w = column_width(window);
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let start = s.start_from.clone();
    let (name_input, author_input) = (s.name.clone(), s.author.clone());
    let notice = s.theme.notice.clone();

    let cols = if col_w >= 760. { 4 } else { 3 };
    let gap = 12.;
    let w = ((col_w - gap * (cols - 1) as f32) / cols as f32).floor();
    let mut grid = div().flex().flex_wrap().gap(px(gap));
    if st.edited {
        let label = tr!("The one in use, with its changes").to_string();
        grid = grid.child(start_tile(d, &st.theme, None, label, w, start.is_none(), window, cx));
    }
    let mut list: Vec<ConsoleTheme> = presets::all();
    list.extend(st.saved.iter().cloned());
    for t in &list {
        let on = start.as_deref() == Some(t.id.as_str()) || (start.is_none() && !st.edited && t.id == st.based_on);
        grid = grid.child(start_tile(d, t, Some(t.id.clone()), shown_name(t), w, on, window, cx));
    }

    let create = button("cm-create", ButtonKind::Primary, tr!("Create theme")).on_click(cx.listener(|d, _, window, cx| {
        let (name, author) = typed(d, cx);
        if name.is_empty() {
            ask_for_a_name(d, cx);
            return;
        }
        let start = d.console_mode.as_ref().and_then(|s| s.start_from.clone());
        if let Some(id) = start {
            theme_rows::change(d, &Command::new("theme.use", Arg::Pick(id)), cx);
        }
        theme_rows::store_op(d, StoreOp::SaveAs { name, author, description: String::new() }, window, cx);
        if let Some(s) = d.console_mode.as_mut() {
            s.view = View::Editor;
            s.group = 0;
        }
        cx.notify();
    }));
    let cancel = button("cm-create-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|d, _, _, cx| {
        if let Some(s) = d.console_mode.as_mut() {
            s.view = View::Home;
            s.theme.notice = None;
        }
        cx.notify();
    }));
    let notice_line =
        notice.filter(|(_, error)| *error).map(|(text, _)| div().text_size(px(12.5)).text_color(theme::danger_text()).child(text));

    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().flex().pb(px(8.)).child(back_button(cx)))
        .child(dialog_title(tr!("New theme")))
        .child(help_text(tr!(
            "Pick the theme to start from and give yours a name. Then change anything in it: tabs, layouts, covers, background, colours and controls."
        )))
        .child(caption(tr!("Start from")))
        .child(grid)
        .child(caption(tr!("Your theme")))
        .child(
            card_box().p(px(16.)).gap(px(12.)).child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap(px(12.))
                    .child(div().w(px(260.)).child(labeled(tr!("Name"), input_box(&name_input, false))))
                    .child(div().w(px(200.)).child(labeled(tr!("Author"), input_box(&author_input, false))))
                    .child(div().flex_1())
                    .child(cancel)
                    .child(create),
            )
            .children(notice_line),
        )
        .child(div().h(px(4.)))
        .child(div().px(px(6.)).child(meta_text(tr!("It is saved on this PC and put in use; export it later to share it."))))
}
