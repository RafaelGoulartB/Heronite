//! Settings → Appearance → Profiles: the built-in looks and the user's own,
//! applied with one click; "Save as profile", export and import.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::appearance::{export_look, import_look, load_profiles, presets, save_profiles, Accent, Appearance, Profile};
use heronite_i18n::tr;

use super::look::set_look;
use super::{set_sliders, Field, SettingsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, button_sm, card, ButtonKind};
use crate::ui::fields::{input_box, meta_text};
use crate::ui::icon;

pub(super) fn preset_name(id: &str) -> &'static str {
    match id {
        "cinema" => tr!("Cinema"),
        "minimal" => tr!("Minimal"),
        "contrast" => tr!("High contrast"),
        _ => tr!("Standard"),
    }
}

fn preset_note(id: &str) -> &'static str {
    match id {
        "cinema" => tr!("The art comes first"),
        "minimal" => tr!("Solid and quiet"),
        "contrast" => tr!("Easy to read"),
        _ => tr!("Today's look"),
    }
}

/// The user's profiles, read off the UI thread the first time the section shows.
pub(super) fn ensure_loaded(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.appearance.profiles.is_some() || d.appearance.profiles_task.is_some() {
        return;
    }
    let app = d.ctx.app.clone();
    d.appearance.profiles_task = Some(cx.spawn(async move |this, cx| {
        let list = cx.background_spawn(async move { load_profiles(&app) }).await;
        this.update(cx, |d, cx| {
            d.appearance.profiles = Some(list);
            cx.notify();
        })
        .ok();
    }));
}

/// Puts `look` on screen and saves it (the sliders follow).
pub(super) fn apply_look(d: &mut SettingsPage, look: Appearance, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let values = [
        (Field::CollectionOpacity, look.collection_opacity),
        (Field::PanelOpacity, look.interface_opacity),
        (Field::PanelBlur, look.interface_blur),
    ];
    set_sliders(&d.appearance, &values, window, cx);
    set_look(d, cx, |a| *a = look);
}

fn store(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let list = d.appearance.profiles.clone().unwrap_or_default();
    let app = d.ctx.app.clone();
    cx.background_spawn(async move {
        if let Err(err) = save_profiles(&app, &list) {
            log::warn!("saving appearance profiles: {err:#}");
        }
    })
    .detach();
    cx.notify();
}

/// Adds `look` under `name`, replacing a profile of the same name.
fn add_profile(d: &mut SettingsPage, name: String, look: Appearance, cx: &mut Context<SettingsPage>) {
    let list = d.appearance.profiles.get_or_insert_with(Vec::new);
    match list.iter_mut().find(|p| p.name.eq_ignore_ascii_case(&name)) {
        Some(p) => p.look = look,
        None => list.push(Profile { name, look }),
    }
    store(d, cx);
}

/// A tile: a band in the look's colours, its name and a note.
fn tile(id: SharedString, look: &Appearance, name: SharedString, note: SharedString, active: bool) -> Stateful<Div> {
    let (tr_, tg, tb) = look.solid_tint.rgb();
    let (ar, ag, ab) = look.accent.rgb().unwrap_or((240, 138, 44));
    let band = div().h(px(40.)).w_full().bg(linear_gradient(
        120.,
        linear_color_stop(theme::rgb_a(tr_, tg, tb, 1.), 0.2),
        linear_color_stop(theme::rgb_a(ar, ag, ab, if look.accent == Accent::Game { 0.75 } else { 1. }), 1.),
    ));
    div()
        .id(id)
        .flex()
        .flex_col()
        .w(px(150.))
        .flex_none()
        .rounded(px(theme::R_ITEM))
        .overflow_hidden()
        .border_1()
        .border_color(if active { theme::accent().into() } else { theme::card_border() })
        .bg(theme::white_a(0.03))
        .cursor_pointer()
        .hover(|s| s.bg(theme::white_a(0.06)))
        .child(band)
        .child(div().px(px(10.)).pt(px(7.)).text_size(px(13.)).text_color(theme::text_primary()).truncate().child(name))
        .child(div().px(px(10.)).pb(px(8.)).text_size(px(11.5)).text_color(theme::text_dim()).truncate().child(note))
}

pub(super) fn profiles_group(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    ensure_loaded(d, cx);
    let current = d.appearance.ui;
    let mut tiles = div().flex().flex_wrap().gap(px(10.));
    let mut matched = false;
    for p in presets() {
        let active = p.look == current;
        matched |= active;
        let look = p.look;
        tiles = tiles.child(
            tile(format!("preset-{}", p.id).into(), &p.look, preset_name(p.id).into(), preset_note(p.id).into(), active)
                .on_click(cx.listener(move |d, _, window, cx| apply_look(d, look, window, cx))),
        );
    }
    for (i, p) in d.appearance.profiles.clone().unwrap_or_default().into_iter().enumerate() {
        let active = p.look == current;
        matched |= active;
        let look = p.look;
        tiles = tiles.child(
            div()
                .relative()
                .child(
                    tile(format!("profile-{i}").into(), &p.look, p.name.clone().into(), tr!("Yours").into(), active)
                        .on_click(cx.listener(move |d, _, window, cx| apply_look(d, look, window, cx))),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("profile-{i}-delete")))
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
                            if let Some(list) = d.appearance.profiles.as_mut() {
                                if i < list.len() {
                                    list.remove(i);
                                }
                            }
                            store(d, cx);
                        })),
                ),
        );
    }
    let save = button_sm("profile-save", ButtonKind::Glass, tr!("Save as profile")).on_click(cx.listener(|d, _, window, cx| {
        let name = d.appearance.profile_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        d.appearance.profile_name.update(cx, |s, cx| s.set_value("", window, cx));
        let look = d.appearance.ui;
        add_profile(d, name, look, cx);
    }));
    let export = button("profile-export", ButtonKind::Glass, tr!("Export…")).on_click(cx.listener(|d, _, _, cx| export(d, cx)));
    let import = button("profile-import", ButtonKind::Glass, tr!("Import…")).on_click(cx.listener(|_, _, _, cx| import(cx)));
    card([
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .py(px(14.))
            .child(meta_text(if matched {
                tr!("Applies everything on this page except the Collection background's sliders.")
            } else {
                tr!("Custom look. Save it to come back to it later.")
            }))
            .child(tiles)
            .into_any_element(),
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(12.))
            .child(div().w(px(220.)).child(input_box(&d.appearance.profile_name, false)))
            .child(save)
            .child(div().flex_1())
            .child(export)
            .child(import)
            .into_any_element(),
    ])
}

/// Writes the look on screen to a file the user picks.
fn export(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let look = d.appearance.ui;
    let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| std::path::PathBuf::from("."));
    let rx = cx.prompt_for_new_path(&dir, Some("heronite-look.json"));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(path))) = rx.await else { return };
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let res = cx.background_spawn(async move { export_look(&path, &name, &look) }).await;
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

/// Reads a look from a file, keeps it as a profile and applies it.
fn import(cx: &mut Context<SettingsPage>) {
    let rx =
        cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        let res = cx.background_spawn(async move { import_look(&path) }).await;
        this.update_in(cx, |d, window, cx| match res {
            Ok((name, look)) => {
                add_profile(d, name, look, cx);
                apply_look(d, look, window, cx);
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
