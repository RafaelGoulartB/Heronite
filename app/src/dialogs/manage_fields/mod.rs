//! `ManageFieldsDialog`: the library fields manager. Lists the genres,
//! companies, series, themes, features and platforms of the Collection with
//! their games, suggests names that look like one, and renames, merges or
//! removes them (`heronite_data::field_names`).
//! The changes are remembered for metadata downloaded later; "Remembered
//! changes" lists them and forgets one.

mod actions;
mod render;

use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::field_names::{self, Field, FieldValue, NamesFile, Rule, FIELDS};
use heronite_data::{DataPaths, Library};
use heronite_i18n::tr;

use super::{DialogCtx, DialogEvent};
use crate::ui::dialog::{dialog_scrolled, DialogProps};
use crate::ui::fields::new_input;

/// A rename, merge or removal, for the Collection's own copies of the names
/// (the View menu's filters and presets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamesChanged {
    pub field: Field,
    pub from: Vec<String>,
    /// `None` = removed.
    pub to: Option<String>,
}

/// The names of one field and the groups that look like one name.
#[derive(Default)]
struct FieldData {
    values: Vec<FieldValue>,
    /// Indexes into `values`, most used first.
    similar: Vec<Vec<usize>>,
}

/// Everything the dialog shows, read off the UI thread.
#[derive(Default)]
struct Overview {
    /// In [`FIELDS`] order.
    fields: Vec<FieldData>,
    rules: Vec<Rule>,
}

impl Overview {
    /// Reads the library (hidden games too) and `names.json`.
    fn load(paths: &DataPaths) -> anyhow::Result<Self> {
        let lib = Library::load_with(paths, true)?;
        let names = NamesFile::load(paths)?;
        let fields = FIELDS
            .iter()
            .map(|&field| {
                let values = field_names::values(&lib.games, field);
                let similar = field_names::similar_groups(field, &values, names.apart(field));
                FieldData { values, similar }
            })
            .collect();
        Ok(Self { fields, rules: names.rules() })
    }

    fn field(&self, field: Field) -> &FieldData {
        let i = FIELDS.iter().position(|f| *f == field).unwrap_or(0);
        &self.fields[i]
    }
}

/// What the body shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Names,
    Remembered,
}

pub struct ManageFieldsDialog {
    paths: DataPaths,
    field: Field,
    pane: Pane,
    data: Option<Arc<Overview>>,
    search: Entity<InputState>,
    /// Keys of the selected names of `field`, in the order they were picked.
    selected: Vec<String>,
    /// Name of a merge of the selected names.
    merge_name: Entity<InputState>,
    /// The name being renamed (key) and its input.
    renaming: Option<(String, Entity<InputState>)>,
    /// Keys whose removal waits for a confirmation.
    confirm_remove: Option<Vec<String>>,
    /// Every suggestion is listed, not only the first ones.
    all_similar: bool,
    /// The body's scroll, for its scrollbar.
    scroll: ScrollHandle,
    busy: bool,
    notice: Option<SharedString>,
    error: Option<SharedString>,
}

impl EventEmitter<DialogEvent> for ManageFieldsDialog {}
impl EventEmitter<NamesChanged> for ManageFieldsDialog {}

impl ManageFieldsDialog {
    pub fn new(ctx: DialogCtx, field: Option<Field>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = new_input(window, cx, "", tr!("Search names"));
        cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.confirm_remove = None;
                cx.notify();
            }
        })
        .detach();
        let merge_name = new_input(window, cx, "", tr!("Name"));
        cx.subscribe_in(&merge_name, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.merge_selected(cx);
            }
        })
        .detach();
        let mut this = Self {
            paths: ctx.data_paths,
            field: field.unwrap_or(Field::Company),
            pane: Pane::Names,
            data: None,
            search,
            selected: Vec::new(),
            merge_name,
            renaming: None,
            confirm_remove: None,
            all_similar: false,
            scroll: ScrollHandle::new(),
            busy: false,
            notice: None,
            error: None,
        };
        // Screenshots: the hidden window does not repaint late results.
        if crate::env::flag("HERONITE_DEBUG_SYNC") {
            match Overview::load(&this.paths) {
                Ok(data) => this.data = Some(Arc::new(data)),
                Err(e) => this.error = Some(format!("{e:#}").into()),
            }
            this.debug_state(window, cx);
        } else {
            this.reload(cx);
        }
        this
    }

    /// `HERONITE_DEBUG_FIELDS=select:<name>,<name>|search:<text>|rules|similar`
    /// opens the dialog with names selected or a search typed (`_` for
    /// spaces), on the remembered changes, or with every suggestion listed.
    fn debug_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = crate::env::value("HERONITE_DEBUG_FIELDS") else { return };
        match value.split_once(':') {
            Some(("select", names)) => {
                for name in names.split(',') {
                    let key = heronite_data::text::facet_key(&name.replace('_', " "));
                    if self.value(&key).is_some() {
                        self.selected.push(key);
                    }
                }
                self.sync_merge_name(window, cx);
            }
            Some(("search", text)) => {
                let text = text.replace('_', " ");
                self.search.update(cx, |s, cx| s.set_value(text, window, cx));
            }
            _ if value == "rules" => self.pane = Pane::Remembered,
            _ if value == "similar" => self.all_similar = true,
            _ => {}
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { Overview::load(&paths) }).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(data) => this.data = Some(Arc::new(data)),
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn values(&self) -> &[FieldValue] {
        self.data.as_ref().map(|d| d.field(self.field).values.as_slice()).unwrap_or(&[])
    }

    fn value(&self, key: &str) -> Option<&FieldValue> {
        self.values().iter().find(|v| v.key == key)
    }

    fn set_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        if self.field == field {
            return;
        }
        self.field = field;
        self.pane = Pane::Names;
        self.selected.clear();
        self.renaming = None;
        self.confirm_remove = None;
        self.all_similar = false;
        self.notice = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    fn toggle_selected(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selected.iter().position(|k| k == key) {
            self.selected.remove(i);
        } else {
            self.selected.push(key.to_owned());
        }
        self.confirm_remove = None;
        self.sync_merge_name(window, cx);
        cx.notify();
    }

    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        self.confirm_remove = None;
        cx.notify();
    }

    /// The merge keeps the most used of the selected names by default.
    fn sync_merge_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let best = self.selected.iter().filter_map(|k| self.value(k)).max_by_key(|v| v.games).map(|v| v.name.clone());
        if let Some(name) = best {
            self.merge_name.update(cx, |s, cx| s.set_value(name, window, cx));
        }
    }
}

impl Render for ManageFieldsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.render_body(cx);
        let footer = self.render_footer(cx);
        // The body's size is known only after it is laid out: draw once more
        // so the scrollbar shows from the start.
        if self.scroll.bounds().size.height <= px(0.) {
            let view = cx.entity_id();
            cx.defer(move |cx| cx.notify(view));
        }
        let entity = cx.entity().downgrade();
        dialog_scrolled(
            DialogProps { id: "manage-fields".into(), title: tr!("Library fields").into(), width: 760., height: None },
            &self.scroll,
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
