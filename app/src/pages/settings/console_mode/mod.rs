//! Settings → Console mode: the console themes (built-in, saved, export,
//! import) and every option of console mode. The options are the console
//! rows of `console::theme_rows`, drawn here as cards (`form.rs`) and on the
//! TV as one list, so each is written once; the tabs, shelves and menu have
//! their own editors (`editors.rs`, `thumbs.rs`) and the theme card
//! `theme_card.rs`.
//!
//! The state exists only once the section was shown: a user who never opens
//! it reads no `console.json` and no library for it. Every change goes
//! through `console::themes::update`, which saves off the UI thread and
//! updates the console on screen.

mod editors;
mod form;
mod theme_card;
mod thumbs;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::Categories;
use heronite_data::paths::AppPaths;
use heronite_data::{DataPaths, Library};
use heronite_i18n::tr;

use super::SettingsPage;
use crate::console::browser::PickKind;
use crate::console::layers::{self, HasLayer};
use crate::console::list::{Outcome, Row};
use crate::console::theme_rows::{self, Card, Facts, Host, ThemeState};
use crate::console::{Hint, Layer, Pad};
use crate::ui::controls::{caption, card};
use crate::ui::fields::new_input;

pub struct State {
    pub(super) theme: ThemeState,
    pub(super) fields: HashMap<String, form::Field>,
    /// The select whose list is open.
    pub(super) open: Option<String>,
    /// Name and author for "Save as theme".
    pub(super) name: Entity<InputState>,
    pub(super) author: Entity<InputState>,
    /// "Delete" asked for, waiting for the second click.
    pub(super) confirm_delete: bool,
}

impl State {
    fn new(app: &AppPaths, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        Self {
            theme: ThemeState::new(app),
            fields: HashMap::new(),
            open: None,
            name: new_input(window, cx, "", tr!("Name of the theme")),
            author: new_input(window, cx, "", tr!("Author")),
            confirm_delete: false,
        }
    }
}

/// The section is on screen: creates the state (once) and reads the saved
/// themes and the library's names for the pickers, off the UI thread.
pub(super) fn shown(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    if d.console_mode.is_some() {
        return;
    }
    let state = State::new(&d.ctx.app, window, cx);
    d.console_mode = Some(state);
    theme_rows::load_saved(d, cx);
    let paths = d.ctx.data_paths.clone();
    cx.spawn(async move |this, cx| {
        let facts = cx.background_spawn(async move { facts_of(&paths) }).await;
        this.update(cx, |d, cx| {
            if let Some(s) = d.console_mode.as_mut() {
                s.theme.data.facts = facts;
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// The statuses, categories, platforms, stores and genres of the library.
fn facts_of(paths: &DataPaths) -> Facts {
    let Ok(lib) = Library::load(paths) else { return Facts::default() };
    let systems: HashMap<String, String> = heronite_emulation::catalog::emulator_definitions()
        .iter()
        .flat_map(|d| d.profiles.iter())
        .map(|p| (p.platform_id.clone(), p.platform_name.clone()))
        .collect();
    Facts::build(&lib, &Categories::load(paths), |g| {
        if g.launch_kind.as_deref() == Some("emulator") {
            if let Some(name) = g.meta.as_deref().and_then(|m| m.platform_id.as_deref()).and_then(|id| systems.get(id)) {
                return name.clone();
            }
        }
        match g.game.install.platform.as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("linux") => "Linux".into(),
            Some("osx" | "mac") => "Mac".into(),
            _ => "PC".into(),
        }
    })
}

impl HasLayer for SettingsPage {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.console.layer
    }
}

impl Host for SettingsPage {
    fn app(&self) -> AppPaths {
        self.ctx.app.clone()
    }

    fn theme_state(&mut self) -> Option<&mut ThemeState> {
        self.console_mode.as_mut().map(|s| &mut s.theme)
    }

    fn open_osk(
        &mut self,
        title: &str,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        layers::open_osk(&mut self.console.layer, title, initial, window, cx, done);
    }

    fn open_browser(
        &mut self,
        title: &str,
        kind: PickKind,
        start: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
        picked: impl Fn(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        layers::open_browser(&mut self.console.layer, title, kind, start.as_deref(), window, cx, picked);
    }

    fn open_sheet(
        &mut self,
        title: &str,
        message: Option<String>,
        rows: Vec<Row>,
        window: &mut Window,
        cx: &mut Context<Self>,
        chosen: impl Fn(&mut Self, &str, &mut Window, &mut Context<Self>) + 'static,
    ) {
        layers::open_sheet(&mut self.console.layer, title.to_string(), message, rows, window, cx, chosen);
    }
}

/// Runs an outcome of a row (a click on the desktop, a press on the TV) and
/// shows an error the way the other sections do.
pub(super) fn send(d: &mut SettingsPage, outcome: Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    theme_rows::on_outcome(d, outcome, window, cx);
    sync_error(d);
    cx.notify();
}

fn sync_error(d: &mut SettingsPage) {
    if let Some((text, true)) = d.console_mode.as_ref().and_then(|s| s.theme.notice.clone()) {
        d.error = Some(text.into());
    }
}

// ---------------------------------------------------------------------------
// Desktop.

pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    if d.console_mode.is_none() {
        return div().into_any_element();
    }
    let mut col = div().flex().flex_col().gap(px(12.));
    for c in theme_rows::CARDS {
        let body = match c {
            Card::Theme => theme_card::render(d, window, cx),
            Card::Tabs => editors::tabs(d, window, cx),
            Card::Shelves => editors::shelves(d, window, cx),
            Card::Menu => editors::menu(d, cx),
            _ => {
                let rows = card_rows(d, c);
                let els: Vec<AnyElement> = rows.iter().map(|r| form::option_row(d, r, window, cx)).collect();
                card(els)
            }
        };
        col = col.child(div().flex().flex_col().gap(px(6.)).child(caption(c.title())).child(body));
    }
    SettingsPage::collection_section(col.into_any_element(), d.error.clone())
}

/// The options of `card`, without the lists the editors draw.
pub(super) fn card_rows(d: &SettingsPage, card: Card) -> Vec<Row> {
    let Some(s) = d.console_mode.as_ref() else { return Vec::new() };
    let mut rows = theme_rows::rows(card, &s.theme.ctx());
    rows.retain(|r| !matches!(r.value, crate::console::list::Value::Action | crate::console::list::Value::Header | crate::console::list::Value::Info(_)));
    let listed = |id: &str, prefix: &str| id.strip_prefix(prefix).is_some_and(|n| n.parse::<usize>().is_ok());
    rows.retain(|r| !listed(&r.id, "shelf.") && !listed(&r.id, "menu."));
    rows
}

// ---------------------------------------------------------------------------
// Console mode.

pub(super) fn console_rows(d: &SettingsPage) -> Vec<Row> {
    match d.console_mode.as_ref() {
        Some(s) => theme_rows::all_rows(&s.theme.ctx()),
        None => Vec::new(),
    }
}

pub(super) fn console_apply(d: &mut SettingsPage, outcome: Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    send(d, outcome, window, cx);
}

/// X, LT and the moving of a row; whether the press was used.
pub(super) fn console_extra_pad(d: &mut SettingsPage, pad: Pad, _window: &mut Window, cx: &mut Context<SettingsPage>) -> bool {
    let rows = console_rows(d);
    let focused = d.console.list.focused(&rows).map(|r| r.id.to_string());
    let Some(s) = d.console_mode.as_mut() else { return false };
    match theme_rows::extra_pad(&mut s.theme, pad, focused.as_deref()) {
        theme_rows::Extra::Pass => false,
        theme_rows::Extra::Handled => true,
        theme_rows::Extra::Run(cmd, delta) => {
            if theme_rows::change(d, &cmd, cx) && delta != 0 {
                d.console.list.focus = d.console.list.focus.saturating_add_signed(delta as isize);
            }
            true
        }
    }
}

pub(super) fn console_hints(d: &SettingsPage) -> Vec<Hint> {
    let Some(s) = d.console_mode.as_ref() else { return Vec::new() };
    let rows = console_rows(d);
    let focused = d.console.list.focused(&rows).map(|r| r.id.to_string());
    let extra = theme_rows::extra_hints(&s.theme, focused.as_deref());
    if s.theme.moving {
        return extra;
    }
    let mut h = vec![crate::console::hint(crate::console::Btn::A, tr!("Select")), crate::console::hint(crate::console::Btn::LeftRight, tr!("Change"))];
    h.extend(extra);
    h
}
