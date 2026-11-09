//! Importing a theme file on the desktop: the file (chosen, or dropped on
//! the themes) is read and shown first, with its picture, who made it and
//! how it stands with the saved themes; nothing is saved or put in use
//! until the user says so, so a theme on screen with unsaved changes is
//! never replaced by surprise.

use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{store, ConsoleTheme};
use heronite_i18n::tr;

use super::home::{pick, picture, tabs_line, Status, PIC_R};
use super::{SettingsPage, View};
use crate::console::theme_rows::{self, Arg, Command};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, card_box, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::icon;

/// A theme file read and waiting for the user.
pub(in crate::pages::settings) struct Import {
    /// As read, its picture written to the cache so it can be shown.
    pub theme: ConsoleTheme,
    pub matched: store::Match,
}

/// Asks for a theme file.
pub(super) fn choose(cx: &mut Context<SettingsPage>) {
    let rx =
        cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr!("Select").into()) });
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        this.update(cx, |d, cx| read(d, path, cx)).ok();
    })
    .detach();
}

/// Reads `path` off the UI thread and shows it.
pub(super) fn read(d: &mut SettingsPage, path: PathBuf, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let res = cx
            .background_spawn(async move {
                let mut t = store::read(&path)?;
                store::unpack_picture(&app.cache.join("console-theme-import"), &mut t)?;
                let matched = store::compare(&t, &store::list(&app));
                anyhow::Ok(Import { theme: t, matched })
            })
            .await;
        this.update(cx, |d, cx| {
            let Some(s) = d.console_mode.as_mut() else { return };
            match res {
                Ok(imp) => {
                    s.importing = Some(imp);
                    s.view = View::Home;
                    s.theme.notice = None;
                    d.error = None;
                }
                Err(e) => d.error = Some(tr!("That file is not a console theme: {error}", error = format!("{e:#}")).into()),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Saves the theme read (replacing the saved one with its id when
/// `replace`) and, when `use_it`, puts it in use.
fn finish(d: &mut SettingsPage, use_it: bool, replace: bool, cx: &mut Context<SettingsPage>) {
    let Some(imp) = d.console_mode.as_mut().and_then(|s| s.importing.take()) else { return };
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let res = cx
            .background_spawn({
                let app = app.clone();
                async move { store::add(&app, &imp.theme, replace).map(|t| (t, store::list(&app))) }
            })
            .await;
        this.update(cx, |d, cx| {
            match res {
                Ok((t, list)) => {
                    let name = t.name.clone();
                    // The theme replaced was in use and unchanged: the new version shows.
                    let replaced_in_use = replace
                        && d.console_mode
                            .as_ref()
                            .is_some_and(|s| s.theme.prefs.based_on == t.id && !theme_rows::is_edited(&s.theme.prefs, &s.theme.data.saved));
                    if let Some(s) = d.console_mode.as_mut() {
                        s.theme.data.saved = list;
                    }
                    if use_it || replaced_in_use {
                        theme_rows::change(d, &Command::new("theme.use", Arg::Pick(t.id.clone())), cx);
                    }
                    if let Some(s) = d.console_mode.as_mut() {
                        s.theme.notice = Some((tr!("“{name}” is with your themes.", name = name), false));
                    }
                }
                Err(e) => d.error = Some(format!("{e:#}").into()),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn cancel(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if let Some(s) = d.console_mode.as_mut() {
        s.importing = None;
    }
    cx.notify();
}

/// The card of the theme read, above the themes.
pub(super) fn render(d: &mut SettingsPage, st: &Status, col_w: f32, window: &mut Window, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let (t, matched) = d.console_mode.as_ref()?.importing.as_ref().map(|i| (i.theme.clone(), i.matched.clone()))?;
    let wide = col_w >= 700.;
    let pic_w = if wide { (col_w * 0.38).floor() } else { col_w - 34. };
    let pic = picture(d, &t, pic_w, window);
    let saved_name = |id: &str| st.saved.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();

    let (note, buttons): (String, Vec<Stateful<Div>>) = match &matched {
        store::Match::Same(id) => {
            let id = id.clone();
            (
                tr!("You already have this theme: “{name}”.", name = saved_name(&id)),
                vec![button_sm("cm-import-use-saved", ButtonKind::Primary, tr!("Use it")).on_click(cx.listener(move |d, _, _, cx| {
                    cancel(d, cx);
                    pick(d, &id, cx);
                }))],
            )
        }
        store::Match::SameId(id) => (
            tr!("Another version of “{name}” is with your themes.", name = saved_name(id)),
            vec![
                button_sm("cm-import-replace", ButtonKind::Primary, tr!("Replace mine"))
                    .on_click(cx.listener(|d, _, _, cx| finish(d, false, true, cx))),
                button_sm("cm-import-both", ButtonKind::Glass, tr!("Keep both"))
                    .on_click(cx.listener(|d, _, _, cx| finish(d, false, false, cx))),
            ],
        ),
        store::Match::New => (
            String::new(),
            vec![
                button_sm("cm-import-use", ButtonKind::Primary, tr!("Add and use it"))
                    .on_click(cx.listener(|d, _, _, cx| finish(d, true, false, cx))),
                button_sm("cm-import-add", ButtonKind::Glass, tr!("Only add it"))
                    .on_click(cx.listener(|d, _, _, cx| finish(d, false, false, cx))),
            ],
        ),
    };
    let warn = (st.unsaved() && matches!(matched, store::Match::New | store::Match::Same(_)))
        .then(|| meta_text(tr!("Your changes to “{name}” are not saved: using another theme replaces them.", name = st.name())));
    let by = (!t.author.trim().is_empty()).then(|| tr!("by {author}", author = t.author.clone()));
    let version = (!t.version.trim().is_empty()).then(|| tr!("version {version}", version = t.version.clone()));
    let meta = [by, version, Some(tabs_line(&t))].into_iter().flatten().collect::<Vec<_>>().join(" · ");

    let info =
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(Icon::Download, px(16.), theme::accent()))
                    .child(meta_text(tr!("A theme to import"))),
            )
            .child(
                div()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(19.))
                    .text_color(theme::text_primary())
                    .truncate()
                    .child(t.name.clone()),
            )
            .child(meta_text(meta))
            .when(!t.description.trim().is_empty(), |d| {
                d.child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(t.description.clone()))
            })
            .when(!note.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(theme::text_primary()).child(note)))
            .children(warn)
            .child(
                div().flex().flex_wrap().gap(px(8.)).pt(px(4.)).children(buttons).child(
                    button_sm("cm-import-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|d, _, _, cx| cancel(d, cx))),
                ),
            );
    let pic = div().flex_none().rounded(px(PIC_R)).child(pic);
    let body = if wide {
        div().flex().items_start().gap(px(20.)).child(pic).child(info)
    } else {
        div().flex().flex_col().gap(px(14.)).child(pic).child(info)
    };
    Some(card_box().mb(px(14.)).p(px(16.)).border_color(theme::accent_alpha(0.6)).child(body))
}
