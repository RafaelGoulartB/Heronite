//! The desktop editors of the lists: tabs (with the layouts as small
//! pictures), shelves and menu items (reorder, switch, add).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{Layout, MAX_TABS};
use heronite_i18n::tr;

use super::form::{choice_select, option_row, text_input};
use super::thumbs::thumb;
use super::{card_rows, send, SettingsPage};
use crate::console::list::{Outcome, Value};
use crate::console::theme_rows::{self, Arg, Card, Command};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, card_box, toggle, ButtonKind};
use crate::ui::fields::{meta_text, select_menu_field, SelectOption};
use crate::ui::icon;

/// A small round icon button.
fn tool(id: SharedString, ic: Icon, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.08))))
        .when(!enabled, |d| d.opacity(0.3))
        .child(icon(ic, px(16.), theme::text_muted()))
}

pub(super) fn tabs(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let tabs = s.theme.prefs.theme.tabs.clone();
    let facts = s.theme.data.facts.clone();
    let mut col = card_box().py(px(12.)).gap(px(12.));
    col = col.child(meta_text(tr!("LB and RB switch between the tabs. Each shows its games in a layout.")));
    for (i, t) in tabs.iter().enumerate() {
        let name = text_input(d, &format!("tab.{i}.name"), &format!("tab.{i}.name"), &t.name, tr!("Name"), window, cx);
        let sources = theme_rows::source_options(&facts, &t.source);
        let at = sources.iter().position(|(v, _)| *v == t.source).unwrap_or(0);
        let labels: Vec<SharedString> = sources.into_iter().map(|(_, l)| l.into()).collect();
        let source = choice_select(d, &format!("tab.{i}.source").into(), &labels, at, true, cx);
        let up = tool(format!("cm-tab-{i}-up").into(), Icon::ArrowUpward, i > 0).when(i > 0, |b| {
            b.on_click(cx.listener(move |d, _, _, cx| theme_rows::run_press(d, &format!("tab.{i}.up"), cx)))
        });
        let remove = tool(format!("cm-tab-{i}-remove").into(), Icon::Close, tabs.len() > 1).when(tabs.len() > 1, |b| {
            b.on_click(cx.listener(move |d, _, _, cx| theme_rows::run_press(d, &format!("tab.{i}.remove"), cx)))
        });
        let mut layouts = div().flex().flex_wrap().gap(px(8.));
        for (li, l) in Layout::ALL.iter().copied().enumerate() {
            let on = t.layout == l;
            layouts = layouts.child(
                div()
                    .id(SharedString::from(format!("cm-tab-{i}-layout-{li}")))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |d, _, window, cx| {
                        send(d, Outcome::Changed(format!("tab.{i}.layout").into(), Value::Choice { options: Vec::new(), index: li }), window, cx);
                    }))
                    .child(thumb(l, on))
                    .child(div().text_size(px(11.5)).text_color(if on { theme::text_primary() } else { theme::text_muted() }).child(tr(theme_rows::layout_label(l)))),
            );
        }
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(theme::R_ITEM))
                .bg(theme::ink_a(0.03))
                .border_1()
                .border_color(theme::card_border())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().flex_none().w(px(48.)).text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Tab {n}", n = i + 1)))
                        .child(div().w(px(180.)).child(name))
                        .child(div().flex_1().min_w_0().child(source))
                        .child(up)
                        .child(remove),
                )
                .child(layouts),
        );
    }
    if tabs.len() < MAX_TABS {
        col = col.child(div().flex().child(button_sm("cm-tab-add", ButtonKind::Glass, tr!("Add a tab")).on_click(cx.listener(|d, _, _, cx| theme_rows::run_press(d, "tab.add", cx)))));
    }
    col
}

pub(super) fn shelves(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let theme = s.theme.prefs.theme.clone();
    let facts = s.theme.data.facts.clone();
    let uses = theme.tabs.iter().any(|t| matches!(t.layout, Layout::Shelves | Layout::Cinema | Layout::Spotlight | Layout::Xbox));
    let n = theme.shelves.len();
    let mut col = card_box().py(px(12.)).gap(px(6.));
    col = col.child(meta_text(if uses {
        tr!("The rows of covers. Leave a title empty to use the shelf's own name.")
    } else {
        tr!("No tab uses a layout with shelves.")
    }));
    for (i, sh) in theme.shelves.iter().enumerate() {
        let default = theme_rows::kind_label(&facts, &sh.kind);
        let title = text_input(d, &format!("title:shelf.{i}"), &format!("shelf.{i}"), &sh.title, &default, window, cx);
        let on = sh.on;
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(4.))
                .when(!uses, |d| d.opacity(0.45))
                .child(tool(format!("cm-shelf-{i}-up").into(), Icon::ArrowUpward, i > 0).when(i > 0, |b| {
                    b.on_click(cx.listener(move |d, _, _, cx| theme_rows::run_move(d, &format!("shelf.{i}"), -1, cx)))
                }))
                .child(tool(format!("cm-shelf-{i}-down").into(), Icon::ArrowDownward, i + 1 < n).when(i + 1 < n, |b| {
                    b.on_click(cx.listener(move |d, _, _, cx| theme_rows::run_move(d, &format!("shelf.{i}"), 1, cx)))
                }))
                .child(div().w(px(230.)).child(title))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(theme_rows::kind_group(&sh.kind)))
                .child(toggle(SharedString::from(format!("cm-shelf-{i}-on")), on, false).on_click(cx.listener(move |d, _, window, cx| {
                    send(d, Outcome::Changed(format!("shelf.{i}").into(), Value::Toggle(!on)), window, cx);
                })))
                .child(tool(format!("cm-shelf-{i}-remove").into(), Icon::Close, true).on_click(cx.listener(move |d, _, _, cx| {
                    theme_rows::change(d, &Command::new(format!("shelf.{i}"), Arg::Remove), cx);
                }))),
        );
    }
    // Add: the kinds the theme does not have yet, by group.
    let kinds = {
        let s = d.console_mode.as_ref().expect("checked above");
        theme_rows::addable_kinds(&s.theme.ctx())
    };
    let options: Vec<SelectOption> = kinds
        .iter()
        .map(|(g, _, name)| SelectOption { label: name.clone().into(), group: Some((*g).into()), ..Default::default() })
        .collect();
    let open = d.console_mode.as_ref().is_some_and(|s| s.open.as_deref() == Some("shelf.add"));
    let add = select_menu_field(
        "cm-shelf-add",
        None,
        tr!("Add a shelf…"),
        false,
        open,
        options,
        None,
        cx,
        |d: &mut SettingsPage, _, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.open = if s.open.as_deref() == Some("shelf.add") { None } else { Some("shelf.add".into()) };
            }
            cx.notify();
        },
        move |d: &mut SettingsPage, i, _, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.open = None;
            }
            if let Some((_, kind, _)) = kinds.get(i) {
                theme_rows::change(d, &Command::new("shelf.add", Arg::Pick(kind.clone())), cx);
            }
        },
    );
    col = col.child(div().w(px(300.)).pt(px(6.)).child(add));
    // Covers per shelf and dimming.
    let rows = card_rows(d, Card::Shelves);
    for r in rows {
        col = col.child(div().child(option_row(d, &r, window, cx)));
    }
    col
}

pub(super) fn menu(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let rows: Vec<_> = theme_rows::rows(Card::Menu, &s.theme.ctx()).into_iter().filter(|r| r.id.starts_with("menu.")).collect();
    let n = rows.len();
    let mut col = card_box().py(px(12.)).gap(px(2.));
    col = col.child(div().pb(px(6.)).child(meta_text(tr!("The ☰ menu: which items it has and in what order."))));
    for (i, r) in rows.iter().enumerate() {
        let id = r.id.to_string();
        let item_id = id.clone();
        let (up_id, down_id) = (id.clone(), id.clone());
        let line = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(3.))
            .child(tool(format!("cm-{id}-up").into(), Icon::ArrowUpward, i > 0).when(i > 0, |b| {
                b.on_click(cx.listener(move |d, _, _, cx| theme_rows::run_move(d, &up_id, -1, cx)))
            }))
            .child(tool(format!("cm-{id}-down").into(), Icon::ArrowDownward, i + 1 < n).when(i + 1 < n, |b| {
                b.on_click(cx.listener(move |d, _, _, cx| theme_rows::run_move(d, &down_id, 1, cx)))
            }));
        let line = match &r.value {
            Value::Toggle(on) => {
                let (on, enabled) = (*on, r.enabled);
                line.child(div().flex_1().text_size(px(14.)).text_color(theme::text_primary()).child(r.label.clone()))
                    .children((!enabled).then(|| meta_text(tr!("Always in the menu"))))
                    .child(toggle(SharedString::from(format!("cm-{id}-on")), on, !enabled).when(enabled, |t| {
                        t.on_click(cx.listener(move |d, _, window, cx| {
                            send(d, Outcome::Changed(item_id.clone().into(), Value::Toggle(!on)), window, cx);
                        }))
                    }))
            }
            _ => line.child(div().flex_1().text_size(px(13.)).text_color(theme::text_dim()).child(format!("\u{2014} {} \u{2014}", r.label))),
        };
        col = col.child(line);
    }
    col
}
