//! The editor (every option of the theme in use, in the groups of the
//! console's Customize panel, beside the live picture of the screen being
//! changed and the theme's details) and New theme (where to start from, a
//! name).
//!
//! In a wide window it is a studio filling the page's height: the options
//! scroll on the left while the picture stays in view on the right. A
//! narrower one stacks everything in one scrolling column.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{presets, ConsoleTheme};
use heronite_i18n::{tr, trn};

use super::home::{draw, picture, shown_name, tabs_line, Status};
use super::{card_rows, cards, column_width, studio_width, SettingsPage, Shown, View};
use crate::console::list::Row;
use crate::console::theme_rows::{self, Arg, Card, Command, StoreOp, PANEL};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, button_sm, caption, card_box, dialog_title, help_text, icon_button, icon_button_sm, tabs, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text, search_box_with};

/// Below this width the editor is one column.
const STUDIO_MIN_W: f32 = 1040.;

/// Opens the editor on the theme in use.
pub(super) fn open(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(s) = d.console_mode.as_mut() else { return };
    s.view = View::Editor;
    s.pending = None;
    s.theme.notice = None;
    s.screen = Shown::Tab(0);
    s.details_of = None;
    s.search.update(cx, |i, cx| i.set_value("", window, cx));
    fill_details(d, window, cx);
    cx.notify();
}

/// Puts the details of the saved theme the one in use is based on in the
/// fields (empty for a built-in one), once per theme: also when the saved
/// themes arrive after the editor opened, or a save gave the theme an id.
fn fill_details(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(st) = Status::of(d) else { return };
    let Some(s) = d.console_mode.as_mut() else { return };
    let id = st.own.as_ref().map(|t| t.id.clone()).unwrap_or_default();
    if s.details_of.as_deref() == Some(id.as_str()) {
        return;
    }
    s.details_of = Some(id);
    let mine = st.own.is_some();
    let own = st.own.unwrap_or_default();
    for (input, value) in [
        (&s.name, if mine { own.name } else { String::new() }),
        (&s.author, if mine { own.author } else { String::new() }),
        (&s.description, if mine { own.description } else { String::new() }),
        (&s.version, if mine { own.version } else { String::new() }),
    ] {
        input.update(cx, |i, cx| i.set_value(value, window, cx));
    }
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
                        s.colour_open = None;
                        s.picker = None;
                    }
                    cx.notify();
                })
                .ok();
        }
    })
}

/// The details typed: name, author, description, version.
fn typed(d: &SettingsPage, cx: &App) -> (String, String, String, String) {
    d.console_mode
        .as_ref()
        .map(|s| {
            let v = |i: &Entity<gpui_kit::component::input::InputState>| i.read(cx).value().trim().to_string();
            (v(&s.name), v(&s.author), v(&s.description), v(&s.version))
        })
        .unwrap_or_default()
}

fn ask_for_a_name(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if let Some(s) = d.console_mode.as_mut() {
        s.theme.notice = Some((tr!("Give the theme a name first.").into(), true));
    }
    cx.notify();
}

/// Saves the theme in use with the details typed: into the saved theme it
/// is based on, else as a new one.
fn save(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(st) = Status::of(d) else { return };
    let (name, author, description, version) = typed(d, cx);
    match st.own {
        Some(t) => {
            let name = if name.is_empty() { t.name.clone() } else { name };
            let app = d.ctx.app.clone();
            // The details belong to the theme: set them on the one in use, then save it.
            if let Some(s) = d.console_mode.as_mut() {
                s.theme.prefs = crate::console::themes::update(&app, |p| {
                    p.theme.description = description;
                    p.theme.version = version;
                });
            }
            theme_rows::store_op(d, StoreOp::SaveDetails { name, author }, window, cx);
        }
        None if name.is_empty() => ask_for_a_name(d, cx),
        None => theme_rows::store_op(d, StoreOp::SaveAs { name, author, description, version }, window, cx),
    }
}

pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    fill_details(d, window, cx);
    let Some(st) = Status::of(d) else { return div() };
    let w = studio_width(window);
    let wide = w >= STUDIO_MIN_W;
    let header = header(d, &st, cx);
    let error = d.error.clone().map(|e| crate::pages::settings::library::notice_line(e, true));
    if !wide {
        let col_w = column_width(window);
        let pic_w = col_w - 34.;
        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(header)
            .child(preview_card(d, &st, pic_w, window, cx))
            .child(details_card(d, &st, cx))
            .children(error)
            .child(options(d, window, cx));
        return div().flex_1().min_h_0().child(
            div().id("cm-studio").size_full().overflow_y_scroll().child(div().max_w(px(880.)).mx_auto().pb(px(24.)).child(body)),
        );
    }
    let side_w = (w * 0.42).clamp(420., 640.);
    let left = div()
        .id("cm-studio-options")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .child(div().pr(px(10.)).pb(px(24.)).flex().flex_col().gap(px(14.)).children(error).child(options(d, window, cx)));
    let right = div()
        .id("cm-studio-side")
        .flex_none()
        .w(px(side_w))
        .h_full()
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .pb(px(24.))
                .child(preview_card(d, &st, side_w - 34., window, cx))
                .child(details_card(d, &st, cx)),
        );
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(header)
        .child(div().flex_1().min_h_0().flex().gap(px(22.)).child(left).child(right))
}

/// Back, the theme's name and state, Undo / Redo, Undo changes, Export, Save.
fn header(d: &mut SettingsPage, st: &Status, cx: &mut Context<SettingsPage>) -> Div {
    let (can_undo, can_redo, notice) = d
        .console_mode
        .as_ref()
        .map(|s| (!s.theme.undo.is_empty(), !s.theme.redo.is_empty(), s.theme.notice.clone()))
        .unwrap_or_default();
    let status = match (&st.own, st.edited, &st.base_name) {
        (Some(_), false, _) => tr!("Saved on this PC.").to_string(),
        (Some(_), true, _) => tr!("Changed since it was saved.").to_string(),
        (None, false, Some(b)) => tr!("{name} is built in: save a copy to make it yours.", name = b.clone()),
        (None, true, Some(b)) => tr!("Changed from {name}. Save it as a theme to keep these changes.", name = b.clone()),
        (None, _, None) => tr!("Not saved as a theme.").to_string(),
    };
    let undo = icon_button("cm-undo", Icon::Undo, 18., can_undo)
        .tooltip(crate::ui::tooltip(tr!("Undo")))
        .when(can_undo, |b| b.on_click(cx.listener(|d, _, _, cx| theme_rows::undo(d, true, cx))));
    let redo = icon_button("cm-redo", Icon::Redo, 18., can_redo)
        .tooltip(crate::ui::tooltip(tr!("Redo")))
        .when(can_redo, |b| b.on_click(cx.listener(|d, _, _, cx| theme_rows::undo(d, false, cx))));
    let revert = (st.edited && st.base_name.is_some()).then(|| {
        button_sm("cm-editor-revert", ButtonKind::Ghost, tr!("Undo all changes")).on_click(cx.listener(|d, _, window, cx| {
            theme_rows::run_press(d, "theme.revert", cx);
            open(d, window, cx);
        }))
    });
    let export = button_sm("cm-export", ButtonKind::Glass, tr!("Export…"))
        .on_click(cx.listener(|d, _, window, cx| theme_rows::store_op(d, StoreOp::Export, window, cx)));
    let save_label = if st.own.is_some() { tr!("Save") } else { tr!("Save as a new theme") };
    let save = button("cm-editor-save", ButtonKind::Primary, save_label).on_click(cx.listener(|d, _, window, cx| save(d, window, cx)));
    let notice_line = notice.map(|(text, error)| {
        div().text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
    });
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .child(back_button(cx))
                .child(div().w(px(1.)).h(px(22.)).bg(theme::ink_a(0.12)))
                .child(dialog_title(st.name()).truncate().max_w(px(360.)))
                .when(st.edited, |d| d.child(super::home::pill(tr!("Changed"), false)))
                .child(div().flex_1())
                .child(undo)
                .child(redo)
                .children(revert)
                .child(export)
                .child(save),
        )
        .child(div().pl(px(4.)).flex().flex_wrap().gap(px(12.)).child(meta_text(status)).children(notice_line))
}

/// The groups (with a dot on those that changed) and the search, then the
/// cards of the group or the options found.
fn options(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let group = s.group.min(PANEL.len() - 1);
    let query = s.search.read(cx).value().trim().to_string();
    let search = s.search.clone();
    let base = theme_rows::base_theme(&s.theme.prefs, &s.theme.data.saved);

    let groups: Vec<(usize, String)> = PANEL
        .iter()
        .enumerate()
        .map(|(i, (name, list))| {
            let changed = base.as_ref().map_or(0, |b| theme_rows::changed_in(&editable(list), &s.theme.prefs, b));
            (i, if changed > 0 { format!("{} \u{2022}", tr(name)) } else { tr(name).to_string() })
        })
        .collect();
    let entity = cx.entity().downgrade();
    let bar = tabs("cm-groups", &groups, if query.is_empty() { group } else { usize::MAX }, move |i, window, cx| {
        entity
            .update(cx, |d, cx| {
                if let Some(s) = d.console_mode.as_mut() {
                    s.group = i;
                    s.open = None;
                    s.colour_open = None;
                    s.picker = None;
                    s.search.update(cx, |i, cx| i.set_value("", window, cx));
                }
                cx.notify();
            })
            .ok();
    });
    let clear = (!query.is_empty()).then(|| {
        icon_button("cm-search-clear", Icon::Close, 14., true).on_click(cx.listener(|d, _, window, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.search.update(cx, |i, cx| i.set_value("", window, cx));
            }
            cx.notify();
        }))
    });
    let search = div().w(px(220.)).flex_none().child(search_box_with(&search, clear.map_or_else(|| div().into_any_element(), |c| c.into_any_element())));
    let top = div().flex().flex_wrap().items_center().gap(px(10.)).child(div().flex_1().min_w_0().child(bar)).child(search);

    let body = if query.is_empty() {
        let list = editable(PANEL[group].1);
        let (body, hidden) = cards(d, &list, window, cx);
        let hidden_note = (hidden > 0).then(|| {
            div().px(px(6.)).pt(px(4.)).child(meta_text(trn!(
                hidden,
                "{n} option is hidden: no tab uses a layout it applies to.",
                "{n} options are hidden: no tab uses a layout they apply to.",
                n = hidden
            )))
        });
        div().flex().flex_col().child(body).children(hidden_note)
    } else {
        found(d, &query, window, cx)
    };
    div().flex().flex_col().gap(px(14.)).child(top).child(body)
}

/// The cards of a group the editor shows: the theme card is the header and
/// startup is not part of a theme.
fn editable(list: &[Card]) -> Vec<Card> {
    list.iter().copied().filter(|c| !matches!(c, Card::Theme | Card::Startup)).collect()
}

/// The options whose name or description holds `query`, under their card's title.
fn found(d: &mut SettingsPage, query: &str, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let q = heronite_data::fuzzy::fold(query);
    let hit = |r: &Row| {
        heronite_data::fuzzy::fold(&r.label).contains(&q) || r.desc.as_deref().is_some_and(|t| heronite_data::fuzzy::fold(t).contains(&q))
    };
    let mut col = div().flex().flex_col().gap(px(12.));
    let mut any = false;
    for (_, list) in PANEL.iter() {
        for c in editable(list) {
            if matches!(c, Card::Tabs | Card::Menu | Card::Colours) {
                // Lists and colours are found by their card's title.
                if !heronite_data::fuzzy::fold(c.title()).contains(&q) {
                    continue;
                }
                let (body, _) = cards(d, &[c], window, cx);
                col = col.child(body);
                any = true;
                continue;
            }
            let theme = d.console_mode.as_ref().map(|s| s.theme.prefs.theme.clone()).unwrap_or_default();
            let rows: Vec<Row> = card_rows(d, c).into_iter().filter(|r| hit(r) && theme_rows::applies(&r.id, &theme)).collect();
            if rows.is_empty() {
                continue;
            }
            any = true;
            let els: Vec<AnyElement> = rows.iter().map(|r| super::form::option_row(d, r, window, cx)).collect();
            col = col.child(div().flex().flex_col().gap(px(6.)).child(caption(c.title())).child(crate::ui::controls::card(els)));
        }
    }
    if !any {
        col = col.child(card_box().py(px(14.)).child(meta_text(tr!("No option matches “{text}”.", text = query.to_string()))));
    }
    col
}

/// The picture of the screen chosen, the screens to choose from under it.
fn preview_card(d: &mut SettingsPage, st: &Status, pic_w: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let shown = d.console_mode.as_ref().map_or(Shown::Tab(0), |s| s.screen);
    let shown = match shown {
        Shown::Tab(i) if i >= st.theme.tabs.len() => Shown::Tab(0),
        s => s,
    };
    let pic = draw(d, &st.theme, shown, pic_w, window);
    let mut chips = div().flex().flex_wrap().gap(px(6.));
    let mut screens: Vec<(Shown, String)> =
        st.theme.tabs.iter().enumerate().map(|(i, t)| (Shown::Tab(i), tr(&t.name).to_string())).collect();
    screens.push((Shown::GamePage, tr!("Game page").to_string()));
    screens.push((Shown::Menu, tr!("Menu").to_string()));
    for (i, (screen, label)) in screens.into_iter().enumerate() {
        let on = screen == shown;
        chips = chips.child(
            div()
                .id(SharedString::from(format!("cm-screen-{i}")))
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| d.bg(theme::accent_tint()).text_color(theme::text_primary()))
                .when(!on, |d| d.text_color(theme::text_muted()).hover(|s| s.bg(theme::ink_a(0.06))))
                .child(label)
                .on_click(cx.listener(move |d, _, _, cx| {
                    if let Some(s) = d.console_mode.as_mut() {
                        s.screen = screen;
                    }
                    cx.notify();
                })),
        );
    }
    card_box()
        .p(px(16.))
        .gap(px(10.))
        .child(pic)
        .child(chips)
        .child(meta_text(tr!("Every change shows here and in console mode at once.")))
}

/// Name, author, description and version: what others see of a shared theme.
fn details_card(d: &mut SettingsPage, st: &Status, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let (name, author, description, version) = (s.name.clone(), s.author.clone(), s.description.clone(), s.version.clone());
    let note = if st.own.is_some() {
        tr!("Saved with the theme and shown to whoever imports it.")
    } else {
        tr!("Give it a name to save it as a theme of your own.")
    };
    card_box()
        .p(px(16.))
        .gap(px(12.))
        .child(div().text_size(px(14.)).font_weight(theme::W_500).text_color(theme::text_primary()).child(tr!("Details")))
        .child(labeled(tr!("Name"), input_box(&name, false)))
        .child(
            div()
                .flex()
                .gap(px(10.))
                .child(div().flex_1().min_w_0().child(labeled(tr!("Author"), input_box(&author, false))))
                .child(div().w(px(110.)).flex_none().child(labeled(tr!("Version"), input_box(&version, false)))),
        )
        .child(labeled(tr!("Description"), input_box(&description, false)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().child(meta_text(note)))
                .child(button_sm("cm-details-save", ButtonKind::Glass, tr!("Save")).on_click(cx.listener(|d, _, window, cx| save(d, window, cx)))),
        )
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
    let (name_input, author_input, description_input) = (s.name.clone(), s.author.clone(), s.description.clone());
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
        let (name, author, description, version) = typed(d, cx);
        if name.is_empty() {
            ask_for_a_name(d, cx);
            return;
        }
        let start = d.console_mode.as_ref().and_then(|s| s.start_from.clone());
        if let Some(id) = start {
            theme_rows::change(d, &Command::new("theme.use", Arg::Pick(id)), cx);
        }
        theme_rows::store_op(d, StoreOp::SaveAs { name, author, description, version }, window, cx);
        if let Some(s) = d.console_mode.as_mut() {
            s.view = View::Editor;
            s.group = 0;
            s.screen = Shown::Tab(0);
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
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(12.))
                            .child(div().w(px(260.)).child(labeled(tr!("Name"), input_box(&name_input, false))))
                            .child(div().w(px(200.)).child(labeled(tr!("Author"), input_box(&author_input, false)))),
                    )
                    .child(labeled(tr!("Description"), input_box(&description_input, false)))
                    .child(div().flex().items_center().gap(px(12.)).child(div().flex_1()).child(cancel).child(create)),
            )
            .children(notice_line),
        )
        .child(div().h(px(4.)))
        .child(div().px(px(6.)).child(meta_text(tr!("It is saved on this PC and put in use; export it later to share it."))))
}
