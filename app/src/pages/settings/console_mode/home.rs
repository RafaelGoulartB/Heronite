//! The section's first view: the themes as cards with their pictures (the
//! built-in ones, the user's, and "Create a new theme"), the theme in use
//! with a few quick changes next to its live picture, and the startup
//! options of this PC (not part of a theme).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{parse_hex, presets, ConsoleTheme};
use heronite_i18n::{tr, trn};

use super::preview::{self, Art};
use super::{card_rows, column_width, form, send, SettingsPage, View};
use crate::console::list::{Outcome, Row, Value};
use crate::console::look::ACCENTS;
use crate::console::theme_rows::{self, Arg, Card, Command, StoreOp};
use crate::icons::Icon;
use crate::pages::caption_with;
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, card_box, icon_button_sm, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::icon;

/// The options the "Quick changes" list offers, in its order: the ones that
/// change the look most and need no knowledge of the layouts.
const QUICK: [&str; 8] = [
    "interface.accent",
    "tiles.shape",
    "tiles.size",
    "background.source",
    "background.dim",
    "controls.hints",
    "controls.glyphs",
    "interface.scale",
];

const GAP: f32 = 14.;
/// Corner radius of the pictures.
pub(super) const PIC_R: f32 = 10.;

/// What the views need to know about the theme in use.
pub(super) struct Status {
    pub theme: ConsoleTheme,
    pub based_on: String,
    pub edited: bool,
    /// The saved theme it is based on.
    pub own: Option<ConsoleTheme>,
    /// The name of the theme it started from (`None` once that was deleted).
    pub base_name: Option<String>,
    pub saved: Vec<ConsoleTheme>,
}

impl Status {
    pub fn of(d: &SettingsPage) -> Option<Self> {
        let s = d.console_mode.as_ref()?;
        let (p, saved) = (&s.theme.prefs, &s.theme.data.saved);
        let base = theme_rows::base_theme(p, saved);
        Some(Self {
            theme: p.theme.clone(),
            based_on: p.based_on.clone(),
            edited: theme_rows::is_edited(p, saved),
            own: theme_rows::based_on_saved(p, saved).cloned(),
            base_name: base.map(|b| shown_name(&b)),
            saved: saved.clone(),
        })
    }

    /// The name to show for the theme in use.
    pub fn name(&self) -> String {
        self.base_name.clone().unwrap_or_else(|| tr!("Customised").into())
    }

    /// Its changes are kept nowhere else: picking another theme loses them.
    pub fn unsaved(&self) -> bool {
        self.edited
    }
}

/// A theme's name as shown (the built-in names are translated).
pub(super) fn shown_name(t: &ConsoleTheme) -> String {
    if presets::by_id(&t.id).is_some() {
        tr(&t.name).to_string()
    } else {
        t.name.clone()
    }
}

fn description(t: &ConsoleTheme) -> String {
    if presets::by_id(&t.id).is_some() {
        tr(&t.description).to_string()
    } else {
        t.description.clone()
    }
}

/// "2 tabs · Shelves, Grid": what the theme's screens are.
pub(super) fn tabs_line(t: &ConsoleTheme) -> String {
    let mut layouts: Vec<&str> = Vec::new();
    for tab in &t.tabs {
        let name = tr(theme_rows::layout_name(tab.layout));
        if !layouts.contains(&name) {
            layouts.push(name);
        }
    }
    let n = t.tabs.len();
    tr!("{tabs} · {layouts}", tabs = trn!(n, "{n} tab", "{n} tabs", n = n), layouts = layouts.join(", "))
}

/// The picture of `t`, `w` px wide.
pub(super) fn picture(d: &mut SettingsPage, t: &ConsoleTheme, w: f32, window: &Window) -> Div {
    let scale = window.scale_factor();
    let Some(s) = d.console_mode.as_mut() else { return div() };
    let mut art = Art { samples: &s.samples, covers: s.covers.as_mut(), scale };
    preview::picture(t, w, PIC_R, &mut art)
}

/// Picks the theme `id`, or asks first when that would lose changes.
pub(super) fn pick(d: &mut SettingsPage, id: &str, cx: &mut Context<SettingsPage>) {
    let Some(st) = Status::of(d) else { return };
    if st.based_on == id && !st.edited {
        return;
    }
    if st.unsaved() {
        if let Some(s) = d.console_mode.as_mut() {
            s.pending = Some(id.to_string());
        }
        cx.notify();
        return;
    }
    theme_rows::change(d, &Command::new("theme.use", Arg::Pick(id.to_string())), cx);
}

/// Opens New theme, starting from the theme in use (or what it is based on
/// when unchanged).
pub(super) fn open_create(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(st) = Status::of(d) else { return };
    let Some(s) = d.console_mode.as_mut() else { return };
    s.view = View::Create;
    s.pending = None;
    s.start_from = (!st.edited && !st.based_on.is_empty()).then(|| st.based_on.clone());
    s.theme.notice = None;
    s.name.update(cx, |i, cx| i.set_value("", window, cx));
    s.author.update(cx, |i, cx| i.set_value("", window, cx));
    cx.notify();
}

pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(st) = Status::of(d) else { return div() };
    let col_w = column_width(window);
    div().flex().flex_col().child(themes(d, &st, col_w, window, cx)).child(in_use(d, &st, col_w, window, cx)).child(startup(d, window, cx))
}

fn themes(d: &mut SettingsPage, st: &Status, col_w: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let cols = if col_w >= 720. { 3 } else { 2 };
    let tile_w = ((col_w - GAP * (cols - 1) as f32) / cols as f32).floor();
    let (pending, confirm) = d.console_mode.as_ref().map(|s| (s.pending.clone(), s.confirm_delete.clone())).unwrap_or_default();

    let mut list: Vec<(ConsoleTheme, bool)> = presets::all().into_iter().map(|t| (t, false)).collect();
    list.extend(st.saved.iter().cloned().map(|t| (t, true)));
    let mut grid = div().flex().flex_wrap().gap(px(GAP));
    for (t, yours) in list {
        let current = t.id == st.based_on;
        let deleting = confirm.as_deref() == Some(t.id.as_str());
        let pic = picture(d, &t, tile_w - 18., window);
        grid = grid.child(theme_card(t, yours, current, current && st.edited, deleting, pic, tile_w, cx));
    }
    grid = grid.child(create_tile(tile_w, cx));

    let import = icon_button_sm("cm-import", ButtonKind::Ghost, Icon::Download, tr!("Import…"), true, {
        let entity = cx.entity().downgrade();
        move |_, window, cx| {
            entity.update(cx, |d, cx| theme_rows::store_op(d, StoreOp::Import, window, cx)).ok();
        }
    });
    div()
        .flex()
        .flex_col()
        .child(caption_with(tr!("Themes"), import))
        .child(div().px(px(6.)).pb(px(10.)).child(meta_text(tr!(
            "A theme is the whole look of console mode: its screens, covers, background and controls. Click one to use it."
        ))))
        .children(pending.map(|id| pending_banner(st, &id, cx)))
        .child(grid)
}

/// "Your changes are not saved": shown when a theme is clicked while the
/// one in use has changes.
fn pending_banner(st: &Status, id: &str, cx: &mut Context<SettingsPage>) -> Div {
    let id = id.to_string();
    card_box()
        .mb(px(GAP))
        .py(px(12.))
        .gap(px(10.))
        .border_color(theme::accent_alpha(0.6))
        .child(
            div().flex().items_center().gap(px(10.)).child(icon(Icon::Warning, px(18.), theme::accent())).child(
                div()
                    .flex_1()
                    .text_size(px(14.))
                    .text_color(theme::text_primary())
                    .child(tr!("Your changes to “{name}” are not saved. Another theme replaces them.", name = st.name())),
            ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(button_sm("cm-pending-save", ButtonKind::Primary, tr!("Save them as a theme")).on_click(cx.listener(
                    |d, _, window, cx| {
                        open_create(d, window, cx);
                        if let Some(s) = d.console_mode.as_mut() {
                            s.start_from = None;
                        }
                    },
                )))
                .child(button_sm("cm-pending-switch", ButtonKind::Danger, tr!("Switch anyway")).on_click(cx.listener(
                    move |d, _, _, cx| {
                        if let Some(s) = d.console_mode.as_mut() {
                            s.pending = None;
                        }
                        theme_rows::change(d, &Command::new("theme.use", Arg::Pick(id.clone())), cx);
                    },
                )))
                .child(button_sm("cm-pending-cancel", ButtonKind::Ghost, tr!("Keep my changes")).on_click(cx.listener(|d, _, _, cx| {
                    if let Some(s) = d.console_mode.as_mut() {
                        s.pending = None;
                    }
                    cx.notify();
                }))),
        )
}

fn pill(text: impl Into<SharedString>, strong: bool) -> Div {
    div()
        .flex_none()
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .text_size(px(11.))
        .when(strong, |d| d.bg(theme::accent()).text_color(theme::on_accent()).font_weight(theme::W_BOLD))
        .when(!strong, |d| d.bg(theme::ink_a(0.08)).text_color(theme::text_muted()))
        .child(text.into())
}

fn theme_card(
    t: ConsoleTheme,
    yours: bool,
    current: bool,
    changed: bool,
    deleting: bool,
    pic: Div,
    w: f32,
    cx: &mut Context<SettingsPage>,
) -> Stateful<Div> {
    let id = t.id.clone();
    let name = shown_name(&t);
    let about = description(&t);
    let by = (yours && !t.author.is_empty()).then(|| tr!("by {author}", author = t.author.clone()));
    let mut badges = div().flex().items_center().gap(px(6.));
    if current {
        badges = badges.child(pill(tr!("In use"), true));
    }
    if changed {
        badges = badges.child(pill(tr!("Changed"), false));
    } else if yours && !current {
        badges = badges.child(pill(tr!("Yours"), false));
    }
    let delete = yours.then(|| {
        let id = id.clone();
        div()
            .id(SharedString::from(format!("cm-theme-{}-delete", t.id)))
            .absolute()
            .top(px(16.))
            .right(px(16.))
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme::black_a(0.55))
            .cursor_pointer()
            .hover(|s| s.bg(theme::black_a(0.8)))
            .child(icon(Icon::DeleteOutline, px(15.), theme::white_a(0.85)))
            .on_click(cx.listener(move |d, _, _, cx| {
                cx.stop_propagation();
                if let Some(s) = d.console_mode.as_mut() {
                    s.confirm_delete = Some(id.clone());
                }
                cx.notify();
            }))
    });
    let footer = if deleting {
        let id = id.clone();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(div().w_full().text_size(px(13.)).text_color(theme::text_primary()).child(tr!("Delete “{name}”?", name = name.clone())))
            .child(button_sm(SharedString::from(format!("cm-theme-{id}-delete-yes")), ButtonKind::Danger, tr!("Delete it")).on_click(
                cx.listener(move |d, _, window, cx| {
                    cx.stop_propagation();
                    if let Some(s) = d.console_mode.as_mut() {
                        s.confirm_delete = None;
                    }
                    theme_rows::store_op(d, StoreOp::DeleteSaved(id.clone()), window, cx);
                }),
            ))
            .child(button_sm(SharedString::from(format!("cm-theme-{}-delete-no", t.id)), ButtonKind::Ghost, tr!("Cancel")).on_click(
                cx.listener(|d, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(s) = d.console_mode.as_mut() {
                        s.confirm_delete = None;
                    }
                    cx.notify();
                }),
            ))
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).line_clamp(2).h(px(32.)).child(about))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(theme::text_dim())
                    .truncate()
                    .child(by.map(|b| format!("{} · {b}", tabs_line(&t))).unwrap_or_else(|| tabs_line(&t))),
            )
    };
    div()
        .id(SharedString::from(format!("cm-theme-{}", t.id)))
        .relative()
        .flex()
        .flex_col()
        .gap(px(10.))
        .w(px(w))
        .flex_none()
        .p(px(8.))
        .pb(px(12.))
        .rounded(px(theme::R_CARD))
        .border_1()
        .border_color(if current { theme::accent().into() } else { theme::card_border() })
        .bg(if current { theme::accent_tint() } else { theme::card_fill() })
        .cursor_pointer()
        .hover(|s| s.border_color(theme::accent_alpha(0.7)))
        .on_click(cx.listener(move |d, _, _, cx| {
            if d.console_mode.as_ref().is_some_and(|s| s.confirm_delete.is_some()) {
                return;
            }
            pick(d, &id, cx);
        }))
        .child(pic)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme::font_heading())
                                .font_weight(theme::W_BOLD)
                                .text_size(px(15.))
                                .text_color(theme::text_primary())
                                .child(name),
                        )
                        .child(badges),
                )
                .child(footer),
        )
        .children(delete)
}

/// The last card of the gallery: makes a theme of one's own.
fn create_tile(w: f32, cx: &mut Context<SettingsPage>) -> Stateful<Div> {
    div()
        .id("cm-theme-create")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .w(px(w))
        .min_h(px(w * 9. / 16. + 96.))
        .flex_none()
        .p(px(20.))
        .rounded(px(theme::R_CARD))
        .border_1()
        .border_color(theme::card_border())
        .bg(theme::ink_a(0.02))
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.05)).border_color(theme::accent_alpha(0.7)))
        .on_click(cx.listener(|d, _, window, cx| open_create(d, window, cx)))
        .child(div().size(px(48.)).flex().items_center().justify_center().rounded_full().bg(theme::accent_tint()).child(icon(
            Icon::Add,
            px(26.),
            theme::accent(),
        )))
        .child(
            div()
                .font_family(theme::font_heading())
                .font_weight(theme::W_BOLD)
                .text_size(px(15.))
                .text_color(theme::text_primary())
                .child(tr!("Create a new theme")),
        )
        .child(
            div()
                .max_w(px(240.))
                .text_center()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme::text_dim())
                .child(tr!("Start from any theme and change everything: tabs, layouts, covers, background, colours and controls.")),
        )
}

/// The theme in use: its live picture and actions, the quick changes beside.
fn in_use(d: &mut SettingsPage, st: &Status, col_w: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let wide = col_w >= 700.;
    let pic_w = if wide { (col_w * 0.46).floor() } else { col_w - 34. };
    let pic = picture(d, &st.theme, pic_w, window);
    let notice = d.console_mode.as_ref().and_then(|s| s.theme.notice.clone());

    let status = match (&st.own, st.edited, &st.base_name) {
        (Some(_), false, _) => tr!("Your theme, saved on this PC.").to_string(),
        (Some(_), true, _) => tr!("Changed since it was saved.").to_string(),
        (None, false, Some(_)) => tr!("A built-in theme. Change it here, or make a copy of your own.").to_string(),
        (None, true, Some(b)) => tr!("Changed from {name}. Save it as a theme to keep these changes.", name = b.clone()),
        (None, _, None) => tr!("Not saved as a theme.").to_string(),
    };
    let mut actions = div().flex().flex_wrap().items_center().gap(px(8.)).child(icon_button_sm(
        "cm-all-options",
        ButtonKind::Primary,
        Icon::Tune,
        tr!("All options"),
        true,
        {
            let entity = cx.entity().downgrade();
            move |_, window, cx| {
                entity.update(cx, |d, cx| super::editor::open(d, window, cx)).ok();
            }
        },
    ));
    if st.edited {
        actions = match &st.own {
            Some(t) => actions.child(
                button_sm("cm-save", ButtonKind::Glass, tr!("Save to “{name}”", name = t.name.clone()))
                    .on_click(cx.listener(|d, _, window, cx| theme_rows::store_op(d, StoreOp::Save, window, cx))),
            ),
            None => actions.child(button_sm("cm-save-as", ButtonKind::Glass, tr!("Save as a theme…")).on_click(cx.listener(
                |d, _, window, cx| {
                    open_create(d, window, cx);
                    if let Some(s) = d.console_mode.as_mut() {
                        s.start_from = None;
                    }
                },
            ))),
        };
        if st.base_name.is_some() {
            actions = actions.child(
                button_sm("cm-revert", ButtonKind::Ghost, tr!("Undo changes"))
                    .on_click(cx.listener(|d, _, _, cx| theme_rows::run_press(d, "theme.revert", cx))),
            );
        }
    }
    let notice_line = notice.map(|(text, error)| {
        div().text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::text_muted() }).child(text)
    });
    let left = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .flex_none()
        .w(px(pic_w))
        .child(pic)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(17.))
                        .text_color(theme::text_primary())
                        .truncate()
                        .child(st.name()),
                )
                .when(st.edited, |d| d.child(pill(tr!("Changed"), false))),
        )
        .child(meta_text(status))
        .child(actions)
        .children(notice_line);
    let right = quick_changes(d, window, cx);
    let body = if wide {
        div().flex().items_start().gap(px(22.)).child(left).child(div().flex_1().min_w_0().child(right))
    } else {
        div().flex().flex_col().gap(px(16.)).child(left).child(right)
    };
    div().flex().flex_col().child(caption(tr!("The theme in use"))).child(card_box().p(px(16.)).child(body))
}

/// The [`QUICK`] options, each with its label above its control.
fn quick_changes(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let Some(s) = d.console_mode.as_ref() else { return div() };
    let all = theme_rows::all_rows(&s.theme.ctx());
    let accent = s.theme.prefs.theme.interface.accent.clone();
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().text_size(px(14.)).text_color(theme::text_primary()).font_weight(theme::W_500).child(tr!("Quick changes")))
        .child(div().pb(px(6.)).child(meta_text(tr!("The changes everyone makes. They show in console mode at once."))));
    for id in QUICK {
        let Some(r) = all.iter().find(|r| r.id.as_ref() == id) else { continue };
        let control =
            if id == "interface.accent" { Some(accent_dots(r, &accent, cx).into_any_element()) } else { form::control(d, r, window, cx) };
        let Some(control) = control else { continue };
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .min_h(px(40.))
                .child(div().w(px(116.)).flex_none().text_size(px(13.)).text_color(theme::text_muted()).child(r.label.clone()))
                .child(div().flex_1().min_w_0().flex().child(control)),
        );
    }
    col
}

/// The focus colours as dots; a colour of one's own shows as one more.
fn accent_dots(r: &Row, accent: &str, cx: &mut Context<SettingsPage>) -> Div {
    let Value::Choice { options, index } = &r.value else { return div() };
    let custom = parse_hex(accent).map(|(r, g, b)| theme::rgb_a(r, g, b, 1.));
    let mut row = div().flex().flex_wrap().items_center().gap(px(8.));
    let count = ACCENTS.len() + usize::from(custom.is_some());
    for i in 0..count {
        let selected = i == *index;
        let fill: Background = match ACCENTS.get(i) {
            // The colour of the focused game: a blend stands for "any colour".
            Some(("game", _)) => linear_gradient(135., linear_color_stop(rgb(0xf7c46c), 0.), linear_color_stop(rgb(0xb9a4fb), 1.)),
            Some((_, c)) => rgb(*c).into(),
            None => custom.unwrap_or(rgb(0xeef2f7)).into(),
        };
        let options = options.clone();
        let name = options.get(i).cloned().unwrap_or_default();
        row = row.child(
            div()
                .id(SharedString::from(format!("cm-accent-{i}")))
                .size(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(if selected { theme::text_primary().into() } else { gpui_kit::transparent_black() })
                .cursor_pointer()
                .tooltip(crate::ui::tooltip(name))
                .child(div().size(px(22.)).rounded_full().bg(fill).border_1().border_color(theme::ink_a(0.15)))
                .when(i < ACCENTS.len(), |d| {
                    d.on_click(cx.listener(move |d, _, window, cx| {
                        send(
                            d,
                            Outcome::Changed("interface.accent".into(), Value::Choice { options: options.clone(), index: i }),
                            window,
                            cx,
                        );
                    }))
                }),
        );
    }
    row
}

fn startup(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> Div {
    let rows = card_rows(d, Card::Startup);
    let els: Vec<AnyElement> = rows.iter().map(|r| form::option_row(d, r, window, cx)).collect();
    div()
        .flex()
        .flex_col()
        .child(caption(Card::Startup.title()))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!("Not part of a theme: these stay on this PC."))))
        .child(card(els))
}
