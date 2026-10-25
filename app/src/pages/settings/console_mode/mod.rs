//! Settings → Console mode, in three views:
//! - Home (`home.rs`): the themes as cards with a picture of each, the
//!   theme in use with a few quick changes beside its picture, and the
//!   startup options of this PC;
//! - the editor (`editor.rs`): every option of the theme in use, in the
//!   groups of the console's Customize panel, under its live picture, with
//!   its name, author, save, undo and export;
//! - New theme (`editor.rs`): where to start from and a name.
//!
//! The options are the console rows of `console::theme_rows`, drawn here as
//! cards (`form.rs`) and on the TV as one list, so each is written once; the
//! tabs, shelves and menu have their own editors (`editors.rs`, `thumbs.rs`)
//! and the pictures are drawn by `preview/`.
//!
//! The state exists only once the section was shown: a user who never opens
//! it reads no `console.json` and no library for it. Every change goes
//! through `console::themes::update`, which saves off the UI thread and
//! updates the console on screen.

mod editor;
mod editors;
mod form;
mod home;
mod preview;
mod thumbs;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::Categories;
use heronite_data::organize::Organize;
use heronite_data::paths::AppPaths;
use heronite_data::{DataPaths, Library};
use heronite_i18n::tr;

use super::SettingsPage;
use crate::console::browser::PickKind;
use crate::console::layers::{self, HasLayer};
use crate::console::list::{Outcome, Row};
use crate::console::theme_rows::{self, Card, Facts, Host, ThemeState};
use crate::console::{Hint, Layer, Pad};
use crate::services::widgets::Covers;
use crate::ui::controls::{caption, card, card_box};
use crate::ui::fields::new_input;

/// Which of the section's views is on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum View {
    Home,
    Editor,
    Create,
}

pub struct State {
    pub(super) theme: ThemeState,
    pub(super) fields: HashMap<String, form::Field>,
    /// The select whose list is open.
    pub(super) open: Option<String>,
    /// Name and author of the theme being saved or created.
    pub(super) name: Entity<InputState>,
    pub(super) author: Entity<InputState>,
    pub(super) view: View,
    /// A theme clicked while the one in use has changes no saved theme
    /// holds: it waits for "Switch anyway".
    pub(super) pending: Option<String>,
    /// The saved theme whose delete waits for a second click.
    pub(super) confirm_delete: Option<String>,
    /// The editor's group of cards (an index of `theme_rows::PANEL`).
    pub(super) group: usize,
    /// New theme: the id of the theme it starts from (`None` = the one in use).
    pub(super) start_from: Option<String>,
    /// The games whose art the pictures use.
    samples: Vec<preview::Sample>,
    /// The pictures' textures, freed while the section is not on screen.
    covers: Option<Covers>,
}

impl State {
    fn new(app: &AppPaths, window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        Self {
            theme: ThemeState::new(app),
            fields: HashMap::new(),
            open: None,
            name: new_input(window, cx, "", tr!("Name of the theme")),
            author: new_input(window, cx, "", tr!("Author")),
            view: View::Home,
            pending: None,
            confirm_delete: None,
            group: 0,
            start_from: None,
            samples: Vec::new(),
            covers: None,
        }
    }
}

/// The section is on screen: creates the state (once) and reads the saved
/// themes and the library's names for the pickers, off the UI thread.
pub(super) fn shown(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    if d.console_mode.is_some() {
        return;
    }
    let mut state = State::new(&d.ctx.app, window, cx);
    // Console mode draws its own list: the pictures are for the desktop page.
    if !crate::console::active() {
        state.covers = Some(Covers::new(&d.ctx.data_paths, &d.ctx.app, cx, |p: &mut SettingsPage| {
            p.console_mode.as_mut().and_then(|s| s.covers.as_mut()).expect("made before any picture is drawn")
        }));
    }
    // `HERONITE_DEBUG_CONSOLE_SETTINGS=editor[:<group>]|create|pending:<theme id>`
    // opens that view (screenshots).
    match crate::env::value("HERONITE_DEBUG_CONSOLE_SETTINGS").as_deref().map(|v| v.split_once(':').unwrap_or((v, ""))) {
        Some(("editor", group)) => {
            state.view = View::Editor;
            state.group = group.parse().unwrap_or(0);
        }
        Some(("create", _)) => state.view = View::Create,
        Some(("pending", id)) => state.pending = Some(id.to_string()),
        _ => {}
    }
    d.console_mode = Some(state);
    library_changed(d);
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

/// The Collection handed over its library: the pictures pick their games again.
pub(super) fn library_changed(d: &mut SettingsPage) {
    let samples = d.storage.library_art().map(|(lib, chains, stages)| preview::samples(lib, chains, stages));
    if let (Some(s), Some(samples)) = (d.console_mode.as_mut(), samples) {
        s.samples = samples;
    }
}

/// The section left the screen: the pictures' textures are freed (they load
/// again from the disk cache).
pub(super) fn hidden(d: &mut SettingsPage, window: &mut Window) {
    if let Some(covers) = d.console_mode.as_mut().and_then(|s| s.covers.as_mut()) {
        covers.release(window);
    }
}

/// The statuses, categories, platforms, stores and genres of the library.
fn facts_of(paths: &DataPaths) -> Facts {
    let Ok(lib) = Library::load(paths) else { return Facts::default() };
    let extras = crate::collection::organize::system_extras(&lib);
    let org = Organize { extras: Some(&extras), ..Default::default() };
    Facts::build(&lib, &Categories::load(paths), |g| org.platform(g))
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

    fn start_options_changed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(crate::pages::PageEvent::ConsoleStartOptionsChanged);
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
    let Some(view) = d.console_mode.as_ref().map(|s| s.view) else {
        return div().into_any_element();
    };
    let body = match view {
        View::Home => home::render(d, window, cx),
        View::Editor => editor::render(d, window, cx),
        View::Create => editor::render_create(d, window, cx),
    };
    SettingsPage::collection_section(body.into_any_element(), d.error.clone())
}

/// The cards of `cards` as the desktop draws them: the editors of the lists,
/// the other options as rows. Options that no tab of the theme uses are left
/// out; `hidden` counts them.
pub(super) fn cards(d: &mut SettingsPage, cards: &[Card], window: &mut Window, cx: &mut Context<SettingsPage>) -> (Div, usize) {
    let mut col = div().flex().flex_col().gap(px(12.));
    let mut hidden = 0;
    let theme = d.console_mode.as_ref().map(|s| s.theme.prefs.theme.clone()).unwrap_or_default();
    for c in cards.iter().copied() {
        let body = match c {
            Card::Tabs => editors::tabs(d, window, cx),
            // A list of shelves no screen shows is noise: a line says where they show.
            Card::Shelves if !theme_rows::uses_shelves(&theme) => card_box().py(px(12.)).child(crate::ui::fields::meta_text(tr!(
                "Shelves show in the Shelves, Cinema, Spotlight and Series X layouts; no tab uses one."
            ))),
            Card::Shelves => editors::shelves(d, window, cx),
            Card::Menu => editors::menu(d, cx),
            _ => {
                let rows = card_rows(d, c);
                let total = rows.len();
                let rows: Vec<Row> = rows.into_iter().filter(|r| theme_rows::applies(&r.id, &theme)).collect();
                hidden += total - rows.len();
                if rows.is_empty() {
                    continue;
                }
                let els: Vec<AnyElement> = rows.iter().map(|r| form::option_row(d, r, window, cx)).collect();
                card(els)
            }
        };
        col = col.child(div().flex().flex_col().gap(px(6.)).child(caption(c.title())).child(body));
    }
    (col, hidden)
}

/// The window's room for the section's column (the page caps it at 880px).
pub(super) fn column_width(window: &Window) -> f32 {
    // The sheet's inset (12 + 12), the section list (230) and the body's padding (36 + 36).
    (f32::from(window.viewport_size().width) - 24. - 230. - 72.).clamp(480., 880.)
}

/// The options of `card`, without the lists the editors draw.
pub(super) fn card_rows(d: &SettingsPage, card: Card) -> Vec<Row> {
    let Some(s) = d.console_mode.as_ref() else { return Vec::new() };
    let mut rows = theme_rows::rows(card, &s.theme.ctx());
    rows.retain(|r| {
        !matches!(r.value, crate::console::list::Value::Action | crate::console::list::Value::Header | crate::console::list::Value::Info(_))
    });
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

/// X, LT and the moving of a row; whether the press was used.
pub(super) fn console_extra_pad(d: &mut SettingsPage, pad: Pad, cx: &mut Context<SettingsPage>) -> bool {
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
    let mut h = vec![
        crate::console::hint(crate::console::Btn::A, tr!("Select")),
        crate::console::hint(crate::console::Btn::LeftRight, tr!("Change")),
    ];
    h.extend(extra);
    h
}
