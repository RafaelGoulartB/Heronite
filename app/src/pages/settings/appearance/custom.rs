//! Settings → Appearance → Custom theme: the six colours of the user's own
//! theme, one picked at a time (a colour picker to drag on, a hex field,
//! suggestions), a contrast check with a fix for the text colours, and the
//! themes saved by name (apply, delete, export, import). Every change
//! applies at once and is saved when the drag ends.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{
    export_theme, hex, import_theme, is_light_colour, load_custom_themes, mix, parse_hex, save_custom_themes, CustomTheme, Palette, Rgb,
    Role, Theme,
};
use heronite_i18n::tr;

use super::look::set_look;
use super::{apply_interface, SettingsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::colour_picker::{ColourPicker, PickerEvent};
use crate::ui::controls::{button, button_sm, row, ButtonKind};
use crate::ui::fields::{input_box, meta_text};
use crate::ui::icon;

/// Contrast below which text is hard to read (WCAG AA).
const READABLE: f32 = 4.5;

/// The editor's state, kept by the Appearance section.
pub(super) struct Editor {
    /// The colour being edited.
    role: Role,
    picker: Entity<ColourPicker>,
    /// Name typed for "Save theme".
    name: Entity<InputState>,
    /// The saved themes; `None` until read.
    saved: Option<Vec<CustomTheme>>,
    saved_task: Option<Task<()>>,
}

impl Editor {
    pub(super) fn new(palette: &Palette, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let role = Role::Panel;
        let picker = cx.new(|cx| ColourPicker::new(palette.get(role), window, cx));
        cx.subscribe(&picker, |d: &mut SettingsPage, _, ev: &PickerEvent, cx| match *ev {
            PickerEvent::Change(c) => set_colour(d, c, false, cx),
            PickerEvent::Commit(c) => set_colour(d, c, true, cx),
        })
        .detach();
        Self { role, picker, name: crate::ui::fields::new_input(window, cx, "", tr!("Theme name")), saved: None, saved_task: None }
    }
}

/// Puts `c` in the edited role, on screen and (when `save`) in the file.
/// The base follows the panels: light panels make a light theme.
fn set_colour(d: &mut SettingsPage, c: Rgb, save: bool, cx: &mut Context<SettingsPage>) {
    let role = d.appearance.custom_editor.role;
    let ui = &mut d.appearance.ui;
    ui.custom.set(role, c);
    ui.custom.light = is_light_colour(ui.custom.panel);
    ui.theme = Theme::Custom;
    apply_interface(d, save, cx);
}

/// Moves the picker to the edited role's colour.
fn sync_controls(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let c = d.appearance.ui.custom.get(d.appearance.custom_editor.role);
    d.appearance.custom_editor.picker.update(cx, |p, cx| p.set(c, window, cx));
}

/// Sets every colour at once (a saved theme, a starting point).
fn set_palette(d: &mut SettingsPage, palette: Palette, window: &mut Window, cx: &mut Context<SettingsPage>) {
    set_look(d, cx, |a| {
        a.custom = palette;
        a.theme = Theme::Custom;
    });
    sync_controls(d, window, cx);
}

fn role_label(r: Role) -> &'static str {
    match r {
        Role::Page => tr!("Page"),
        Role::Panel => tr!("Panels"),
        Role::Lift => tr!("Raised parts"),
        Role::Text => tr!("Text"),
        Role::Muted => tr!("Secondary text"),
        Role::Accent => tr!("Accent"),
    }
}

fn role_note(r: Role) -> &'static str {
    match r {
        Role::Page => tr!("Behind everything, and what fades the game's art behind the Collection."),
        Role::Panel => tr!("Menus, dialogs, the drawer and pages like this one."),
        Role::Lift => tr!("What sits on a panel: the active tab, fields, popovers, round buttons."),
        Role::Text => tr!("Titles and text; lines and faint fills are drawn in it too."),
        Role::Muted => tr!("Captions, descriptions and labels."),
        Role::Accent => tr!("The selection ring, links, active buttons and sliders."),
    }
}

/// Colours to start from for `role`, for a light or a dark theme.
fn suggestions(role: Role, light: bool) -> Vec<Rgb> {
    let hexes: &[&str] = match (role, light) {
        (Role::Page | Role::Panel | Role::Lift, true) => {
            &["#e8eaee", "#ece7de", "#eee6d6", "#e3eae2", "#e1e8f1", "#e9e4ef", "#f0e4e6", "#f2f2f0"]
        }
        (Role::Page | Role::Panel | Role::Lift, false) => {
            &["#161b24", "#1a1a1c", "#10162a", "#221828", "#14201a", "#241c16", "#0f1e26", "#261419"]
        }
        (Role::Text, true) => &["#20262f", "#2d261e", "#1f2a24", "#2a2238", "#111111", "#3a3f47"],
        (Role::Text, false) => &["#e8edf4", "#f0e9dc", "#e2efe6", "#ece4f6", "#ffffff", "#c9d1dc"],
        (Role::Muted, true) => &["#475262", "#62574a", "#4a5a50", "#5a4f6a", "#555555"],
        (Role::Muted, false) => &["#9aa6b6", "#b3a896", "#98ab9f", "#aa9cbc", "#a0a0a0"],
        (Role::Accent, true) => &["#196479", "#126a44", "#864f00", "#a7304e", "#5e41d3", "#b54708", "#0b6e6a", "#7a3cc2"],
        (Role::Accent, false) => &["#a5dceb", "#7ee0b5", "#ffc46b", "#ff8fa3", "#b9a6ff", "#ffab70", "#6fdcd2", "#d4a5ff"],
    };
    let shift = |c: Rgb| match (role, light) {
        // The page a step darker than a panel, raised parts a step lighter.
        (Role::Page, true) => mix(c, (0, 0, 0), 0.04),
        (Role::Page, false) => mix(c, (0, 0, 0), 0.45),
        (Role::Lift, true) => mix(c, (255, 255, 255), 0.4),
        (Role::Lift, false) => mix(c, (255, 255, 255), 0.1),
        _ => c,
    };
    hexes.iter().filter_map(|h| parse_hex(h)).map(shift).collect()
}

/// A square of `c` with a faint edge.
fn swatch(c: Rgb, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.28))
        .bg(theme::rgb_a(c.0, c.1, c.2, 1.))
        .border_1()
        .border_color(theme::ink_a(0.18))
}

/// "4.9:1" in the dim colour, in the danger colour below [`READABLE`].
fn contrast_chip(ratio: f32) -> Div {
    let low = ratio < READABLE;
    div()
        .flex_none()
        .px(px(7.))
        .py(px(1.))
        .rounded_full()
        .text_size(px(11.5))
        .bg(if low { theme::danger_fill() } else { theme::ink_a(0.06) })
        .text_color(if low { theme::danger_text() } else { theme::text_dim() })
        .child(format!("{ratio:.1}:1"))
}

/// The rows of the custom theme in the Colour card, under Theme, while it
/// is the theme in use.
pub(super) fn rows(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    if d.appearance.ui.theme != Theme::Custom {
        return Vec::new();
    }
    ensure_loaded(d, cx);
    vec![editor(d, cx), saved_row(d, cx)]
}

/// Starting points, the list of colours and the editor of the one picked.
fn editor(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = d.appearance.ui.custom;
    let e = &d.appearance.custom_editor;
    let start = div()
        .flex()
        .gap(px(8.))
        .child(
            button_sm("custom-from-light", ButtonKind::Glass, tr!("Light colours"))
                .on_click(cx.listener(|d, _, window, cx| set_palette(d, Palette::LIGHT, window, cx))),
        )
        .child(
            button_sm("custom-from-dark", ButtonKind::Glass, tr!("Dark colours"))
                .on_click(cx.listener(|d, _, window, cx| set_palette(d, Palette::DARK, window, cx))),
        );

    let mut list = div().flex().flex_col().gap(px(4.)).w(px(250.)).flex_none();
    for role in Role::ALL {
        let on = role == e.role;
        let c = p.get(role);
        list = list.child(
            div()
                .id(SharedString::from(format!("custom-role-{}", role.as_str())))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(theme::R_ITEM))
                .border_1()
                .border_color(if on { theme::accent_tint_border() } else { gpui_kit::transparent_black() })
                .when(on, |d| d.bg(theme::accent_tint()))
                .when(!on, |d| d.hover(|s| s.bg(theme::ink_a(0.05))))
                .cursor_pointer()
                .on_click(cx.listener(move |d, _, window, cx| {
                    d.appearance.custom_editor.role = role;
                    sync_controls(d, window, cx);
                    cx.notify();
                }))
                .child(swatch(c, 26.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(13.5)).text_color(theme::text_primary()).child(role_label(role)))
                        .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(hex(c))),
                )
                .children(p.contrast_of(role).map(contrast_chip)),
        );
    }

    let c = p.get(e.role);
    let mut suggested = div().flex().flex_wrap().gap(px(8.));
    for (i, sc) in suggestions(e.role, p.light).into_iter().enumerate() {
        suggested = suggested.child(
            swatch(sc, 26.)
                .id(SharedString::from(format!("custom-suggest-{i}")))
                .cursor_pointer()
                .when(sc == c, |d| d.border_2().border_color(theme::text_bright()))
                .hover(|s| s.border_color(theme::ink_a(0.5)))
                .on_click(cx.listener(move |d, _, window, cx| {
                    set_colour(d, sc, true, cx);
                    sync_controls(d, window, cx);
                })),
        );
    }
    let mut pane = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(15.)).text_color(theme::text_primary()).child(role_label(e.role)))
                .child(meta_text(role_note(e.role))),
        )
        .child(div().max_w(px(340.)).child(e.picker.clone()))
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Suggestions")))
        .child(suggested);
    if let Some(ratio) = p.contrast_of(e.role) {
        let role = e.role;
        let low = ratio < READABLE;
        let line = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(icon(if low { Icon::Warning } else { Icon::Check }, px(16.), if low { theme::danger_text() } else { theme::success() }))
            .child(div().flex_1().text_size(px(12.5)).text_color(theme::text_muted()).child(if low {
                tr!("Contrast on the panels is {ratio}:1, hard to read (aim for 4.5:1).", ratio = format!("{ratio:.1}"))
            } else {
                tr!("Contrast on the panels is {ratio}:1, easy to read.", ratio = format!("{ratio:.1}"))
            }))
            .when(low, |l| {
                l.child(button_sm("custom-fix", ButtonKind::Glass, tr!("Make it readable")).on_click(cx.listener(
                    move |d, _, window, cx| {
                        let fixed = d.appearance.ui.custom.readable(role);
                        set_colour(d, fixed, true, cx);
                        sync_controls(d, window, cx);
                    },
                )))
            });
        pane = pane.child(line);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .py(px(14.))
        .child(row(tr!("Custom theme"), Some(tr!("Pick each colour, or start again from the light or the dark theme's.")), start).py_0())
        .child(div().flex().gap(px(20.)).child(list).child(pane))
        .into_any_element()
}

/// The saved themes as tiles (click: apply), "Save theme", export and import.
fn saved_row(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let current = d.appearance.ui.custom;
    let mut tiles = div().flex().flex_wrap().gap(px(10.));
    for (i, t) in d.appearance.custom_editor.saved.clone().unwrap_or_default().into_iter().enumerate() {
        let p = t.palette;
        let band = div().flex().h(px(34.)).w_full().children(
            [p.page, p.panel, p.lift, p.muted, p.text, p.accent].map(|c| div().flex_1().h_full().bg(theme::rgb_a(c.0, c.1, c.2, 1.))),
        );
        tiles = tiles.child(
            div()
                .id(SharedString::from(format!("custom-saved-{i}")))
                .relative()
                .flex()
                .flex_col()
                .w(px(150.))
                .rounded(px(theme::R_ITEM))
                .overflow_hidden()
                .border_1()
                .border_color(if p == current { theme::accent().into() } else { theme::card_border() })
                .bg(theme::ink_a(0.03))
                .cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.06)))
                .on_click(cx.listener(move |d, _, window, cx| set_palette(d, p, window, cx)))
                .child(band)
                .child(div().px(px(10.)).py(px(7.)).text_size(px(13.)).text_color(theme::text_primary()).truncate().child(t.name.clone()))
                .child(
                    div()
                        .id(SharedString::from(format!("custom-saved-{i}-delete")))
                        .absolute()
                        .top(px(6.))
                        .right(px(6.))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme::black_a(0.55))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::black_a(0.8)))
                        .child(icon(Icon::Close, px(14.), theme::white_a(0.85)))
                        .on_click(cx.listener(move |d, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(list) = d.appearance.custom_editor.saved.as_mut() {
                                if i < list.len() {
                                    list.remove(i);
                                }
                            }
                            store(d, cx);
                        })),
                ),
        );
    }
    let save = button_sm("custom-save", ButtonKind::Glass, tr!("Save theme")).on_click(cx.listener(|d, _, window, cx| {
        let name = d.appearance.custom_editor.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        d.appearance.custom_editor.name.update(cx, |s, cx| s.set_value("", window, cx));
        let palette = d.appearance.ui.custom;
        add(d, name, palette, cx);
    }));
    let export = button("custom-export", ButtonKind::Glass, tr!("Export…")).on_click(cx.listener(|d, _, _, cx| export(d, cx)));
    let import = button("custom-import", ButtonKind::Glass, tr!("Import…")).on_click(cx.listener(|_, _, _, cx| import(cx)));
    let has_saved = d.appearance.custom_editor.saved.as_ref().is_some_and(|l| !l.is_empty());
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .py(px(14.))
        .child(meta_text(if has_saved {
            tr!("Your saved themes. Click one to use it.")
        } else {
            tr!("Save these colours under a name to come back to them, or share them as a file.")
        }))
        .when(has_saved, |d| d.child(tiles))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(220.)).child(input_box(&d.appearance.custom_editor.name, false)))
                .child(save)
                .child(div().flex_1())
                .child(export)
                .child(import),
        )
        .into_any_element()
}

/// The saved themes, read off the UI thread the first time the section shows.
fn ensure_loaded(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let e = &mut d.appearance.custom_editor;
    if e.saved.is_some() || e.saved_task.is_some() {
        return;
    }
    let app = d.ctx.app.clone();
    e.saved_task = Some(cx.spawn(async move |this, cx| {
        let list = cx.background_spawn(async move { load_custom_themes(&app) }).await;
        this.update(cx, |d, cx| {
            d.appearance.custom_editor.saved = Some(list);
            cx.notify();
        })
        .ok();
    }));
}

/// Adds (or replaces, by name) a saved theme and writes the list.
fn add(d: &mut SettingsPage, name: String, palette: Palette, cx: &mut Context<SettingsPage>) {
    let list = d.appearance.custom_editor.saved.get_or_insert_with(Vec::new);
    match list.iter_mut().find(|t| t.name == name) {
        Some(t) => t.palette = palette,
        None => list.push(CustomTheme { name, palette }),
    }
    store(d, cx);
}

/// Writes the saved themes off the UI thread.
fn store(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let list = d.appearance.custom_editor.saved.clone().unwrap_or_default();
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let res = cx.background_spawn(async move { save_custom_themes(&app, &list) }).await;
        this.update(cx, |d, cx| {
            if let Err(err) = res {
                d.error = Some(format!("{err:#}").into());
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Writes the colours on screen to a file the user picks.
fn export(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let palette = d.appearance.ui.custom;
    let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| std::path::PathBuf::from("."));
    let rx = cx.prompt_for_new_path(&dir, Some("heronite-theme.json"));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(path))) = rx.await else { return };
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let res = cx.background_spawn(async move { export_theme(&path, &name, &palette) }).await;
        if let Err(err) = res {
            this.update(cx, |d, cx| {
                d.error = Some(format!("{err:#}").into());
                cx.notify();
            })
            .ok();
        }
    })
    .detach();
}

/// Reads a theme from a file, keeps it and uses it.
fn import(cx: &mut Context<SettingsPage>) {
    let rx =
        cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        let res = cx.background_spawn(async move { import_theme(&path) }).await;
        this.update_in(cx, |d, window, cx| match res {
            Ok((name, palette)) => {
                add(d, name, palette, cx);
                set_palette(d, palette, window, cx);
            }
            Err(err) => {
                d.error = Some(format!("{err:#}").into());
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}
