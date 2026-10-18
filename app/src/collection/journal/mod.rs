//! The focused game's journal (`heronite_data::journal`): state, loading and
//! the edits. The Journal view (`render`) replaces the Timeline in the focus
//! panel when picked; the Timeline shows each day's count of notes.
//!
//! A note is written in place: its paragraph turns into a borderless text
//! area. Enter (or leaving it) saves, Shift + Enter breaks the line, Esc
//! puts the text back.

mod render;

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::journal::{self, Entry, GameJournal, JournalDay, SOURCE_APP};
use heronite_data::timeline::TimelineDay;
use heronite_i18n::tr;

use super::{CollectionView, ToastAction};

/// Days listed before "Show more".
const DAYS: usize = 8;

/// What the open text area writes.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Target {
    Entry(String),
    /// A new note about this moment (unix ms).
    New(i64),
    LeftOff,
}

pub(super) struct Edit {
    pub target: Target,
    pub state: Entity<TextareaState>,
    _sub: Subscription,
}

#[derive(Default)]
pub(super) struct JournalUi {
    /// Game the journal belongs to (or is loading for).
    key: Option<SharedString>,
    pub data: Option<Arc<GameJournal>>,
    /// Bumped per load, so a late answer of an older one is dropped.
    generation: u64,
    pub edit: Option<Edit>,
    pub search: Option<Entity<InputState>>,
    pub query: String,
    pub only_notes: bool,
    pub days_shown: usize,
    /// The days last built and the timeline and journal they came from
    /// (both are replaced, never changed in place).
    days: Option<(Arc<Vec<TimelineDay>>, Arc<GameJournal>, Arc<Vec<JournalDay>>)>,
    /// `HERONITE_DEBUG_JOURNAL_EDIT` not applied yet.
    debug_edit: Option<Option<String>>,
}

impl JournalUi {
    /// Whether it is the journal of `key`.
    pub fn is_for(&self, key: &SharedString) -> bool {
        self.key.as_ref() == Some(key)
    }

    /// The day a note being written lands on (local).
    pub fn new_note_date(&self) -> Option<chrono::NaiveDate> {
        match self.edit.as_ref().map(|e| &e.target) {
            Some(Target::New(at)) => chrono::Local.timestamp_millis_opt(*at).single().map(|d| d.date_naive()),
            _ => None,
        }
    }
}

impl CollectionView {
    /// Runs on every render: reads the journal of a newly focused game.
    pub(super) fn sync_journal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.journal.search.is_none() {
            let input = crate::ui::fields::new_input(window, cx, "", tr!("Search the journal"));
            cx.subscribe(&input, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.journal.query = input.read(cx).value().trim().to_string();
                    cx.notify();
                }
            })
            .detach();
            self.journal.search = Some(input);
        }
        // `HERONITE_DEBUG_JOURNAL_EDIT=left-off|new|first` opens the text
        // area on that once the journal is read (screenshots).
        let debug = self.journal.debug_edit.get_or_insert_with(|| crate::env::value("HERONITE_DEBUG_JOURNAL_EDIT"));
        if let (Some(what), Some(data)) = (debug.clone(), self.journal.data.clone()) {
            self.journal.debug_edit = Some(None);
            let target = match what.as_str() {
                "left-off" => Some(Target::LeftOff),
                "first" => data.entries.last().map(|e| Target::Entry(e.id.clone())),
                _ => Some(Target::New(Utc::now().timestamp_millis())),
            };
            if let Some(t) = target {
                self.journal_edit(t, window, cx);
            }
        }
        if self.journal.key == self.selected {
            return;
        }
        self.journal.key = self.selected.clone();
        self.journal.data = None;
        self.journal.edit = None;
        self.journal.days_shown = DAYS;
        self.load_journal(cx);
    }

    fn load_journal(&mut self, cx: &mut Context<Self>) {
        self.journal.generation += 1;
        let Some(key) = self.journal.key.clone() else { return };
        let (generation, paths) = (self.journal.generation, self.data_paths.clone());
        cx.spawn(async move |this, cx| {
            let k = key.to_string();
            let data = cx.background_spawn(async move { journal::load(&paths, &k) }).await;
            this.update(cx, |this, cx| {
                if this.journal.generation == generation {
                    this.journal.data = Some(Arc::new(data));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The days of the Journal: the timeline's and those with notes.
    pub(super) fn journal_days(&mut self, key: &SharedString) -> Option<(Arc<Vec<TimelineDay>>, Arc<Vec<JournalDay>>)> {
        let timeline = self.timeline_days(key)?;
        let data = self.journal.data.clone()?;
        if let Some((t, d, days)) = &self.journal.days {
            if Arc::ptr_eq(t, &timeline) && Arc::ptr_eq(d, &data) {
                return Some((timeline, days.clone()));
            }
        }
        let lib = self.lib.clone()?;
        let sessions = lib.get(key).and_then(|g| g.sessions.clone()).unwrap_or_default();
        let days = Arc::new(journal::journal_days(&timeline, &data.entries, &sessions, &chrono::Local));
        self.journal.days = Some((timeline.clone(), data, days.clone()));
        Some((timeline, days))
    }

    pub(super) fn set_journal_view(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.ui.journal_view != on {
            self.ui.journal_view = on;
            self.save_ui();
        }
        cx.notify();
    }

    /// Opens the text area on `target` (saving one already open).
    pub(super) fn journal_edit(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.journal_commit(window, cx);
        let Some(data) = self.journal.data.clone() else { return };
        let text = match &target {
            Target::Entry(id) => data.entries.iter().find(|e| &e.id == id).map(|e| e.text.clone()).unwrap_or_default(),
            Target::New(_) => String::new(),
            Target::LeftOff => data.left_off.as_ref().map(|l| l.text.clone()).unwrap_or_default(),
        };
        let placeholder = match &target {
            Target::LeftOff => tr!("Where to pick it up next time"),
            Target::Entry(_) if text.is_empty() => tr!("A moment marked in-game. What happened?"),
            _ => tr!("What happened? A boss, a secret, where to go back…"),
        };
        let state = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).auto_grow(1, 14).submit_on_enter(true);
            s.set_placeholder(placeholder, window, cx);
            s.set_value(text, window, cx);
            s
        });
        let sub = cx.subscribe_in(&state, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { shift: false, .. } | InputEvent::Blur => this.journal_commit(window, cx),
            _ => {}
        });
        state.update(cx, |s, cx| s.focus(window, cx));
        self.journal.edit = Some(Edit { target, state, _sub: sub });
        cx.notify();
    }

    /// Closes the text area without saving.
    pub(super) fn journal_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.journal.edit.take().is_some() {
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// Saves the open text area and closes it.
    pub(super) fn journal_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.journal.edit.take() else { return };
        window.focus(&self.focus_handle, cx);
        cx.notify();
        let text = edit.state.read(cx).value().trim().to_string();
        let (Some(key), Some(data)) = (self.journal.key.clone(), self.journal.data.clone()) else { return };
        let mut next = (*data).clone();
        let key = key.to_string();
        let write: Box<dyn FnOnce(&heronite_data::DataPaths) -> anyhow::Result<()> + Send> = match edit.target {
            Target::Entry(id) => {
                let Some(e) = next.entries.iter_mut().find(|e| e.id == id) else { return };
                if e.text == text {
                    return;
                }
                e.text = text.clone();
                Box::new(move |p| journal::set_text(p, &key, &id, &text).map(|_| ()))
            }
            Target::New(_) if text.is_empty() => return,
            Target::New(at) => {
                let entry = Entry::new(Utc.timestamp_millis_opt(at).single(), &text, None, SOURCE_APP);
                next.entries.push(entry.clone());
                next.entries.sort_by_key(|e| e.at_ms().unwrap_or(0));
                Box::new(move |p| journal::add(p, &key, &entry))
            }
            Target::LeftOff => {
                if next.left_off.as_ref().map(|l| l.text.as_str()).unwrap_or("") == text {
                    return;
                }
                next.left_off = (!text.is_empty()).then(|| journal::LeftOff { text: text.clone(), ..Default::default() });
                Box::new(move |p| journal::set_left_off(p, &key, &text))
            }
        };
        self.journal.data = Some(Arc::new(next));
        self.write_journal(write, cx);
    }

    /// Deletes a note; the toast offers it back.
    pub(super) fn journal_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let (Some(key), Some(data)) = (self.journal.key.clone(), self.journal.data.clone()) else { return };
        let mut next = (*data).clone();
        let Some(i) = next.entries.iter().position(|e| e.id == id) else { return };
        let entry = next.entries.remove(i);
        self.journal.data = Some(Arc::new(next));
        let (k, id) = (key.to_string(), id.to_string());
        self.write_journal(Box::new(move |p| journal::remove(p, &k, &id).map(|_| ())), cx);
        self.show_toast(tr!("Note deleted"), false, cx);
        self.toast_action = Some(ToastAction::RestoreNote(key.to_string(), Box::new(entry)));
    }

    /// Puts a deleted note back.
    pub(super) fn journal_restore(&mut self, key: String, entry: Entry, cx: &mut Context<Self>) {
        if self.journal.key.as_deref() == Some(key.as_str()) {
            if let Some(data) = self.journal.data.clone() {
                let mut next = (*data).clone();
                next.entries.push(entry.clone());
                next.entries.sort_by_key(|e| e.at_ms().unwrap_or(0));
                self.journal.data = Some(Arc::new(next));
            }
        }
        self.write_journal(Box::new(move |p| journal::add(p, &key, &entry)), cx);
    }

    fn write_journal(&mut self, write: Box<dyn FnOnce(&heronite_data::DataPaths) -> anyhow::Result<()> + Send>, cx: &mut Context<Self>) {
        let paths = self.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { write(&paths) }).await;
            if let Err(err) = res {
                this.update(cx, |this, cx| {
                    this.show_toast(tr!("Could not save the journal: {err}", err = format!("{err:#}")), true, cx);
                    // What is on disk wins.
                    this.load_journal(cx);
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }
}
