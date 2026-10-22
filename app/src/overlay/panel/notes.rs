//! The Notes tab: mark this moment, write a note, "where I left off", and
//! the notes of this session and of the last day with notes. Text comes
//! from the on-screen keyboard, a sheet over the panel. Y on a capture of
//! the Screenshots tab (or its viewer) writes that capture's note.
//!
//! Writes go to the game's journal at once (`heronite_data::journal`); the
//! Collection reads it again when told (`journal_changed`).

use std::path::PathBuf;

use chrono::{DateTime, Local, TimeZone, Utc};
use gpui_kit::*;
use heronite_data::journal::{self, Entry, GameJournal, SOURCE_OVERLAY};
use heronite_i18n::tr;

use super::OverlayPanel;
use crate::console::list::Row;
use crate::console::osk::{Osk, OskEvent};
use crate::icons::Icon;

/// Notes listed from the last day with notes.
const LAST_TIME: usize = 4;
/// A note longer than this shows whole under its row when focused.
const FULL_TEXT_AFTER: usize = 36;
/// Characters of "where I left off" shown in its row.
const LEFT_OFF_CHARS: usize = 22;

/// What the open keyboard writes.
#[derive(Clone, Debug)]
pub(super) enum Writing {
    New,
    Edit(String),
    LeftOff,
    /// The note of a capture (taken at unix ms).
    Shot(PathBuf, Option<f64>),
}

/// "21:04".
fn time_label(at: DateTime<Local>) -> String {
    at.format("%H:%M").to_string()
}

fn entry_local(e: &Entry) -> Option<DateTime<Local>> {
    e.at_ms().and_then(|ms| Local.timestamp_millis_opt(ms).single())
}

fn shorten(text: &str, chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(chars) {
        Some((i, _)) => format!("{}…", text[..i].trim_end()),
        None => text.to_owned(),
    }
}

impl OverlayPanel {
    fn journal(&self) -> Option<&GameJournal> {
        self.data.journal.as_ref().map(|(_, j)| j)
    }

    /// When the running session started.
    fn session_start_ms(&self) -> Option<i64> {
        let game = self.game.as_ref()?;
        let elapsed = chrono::Duration::from_std(game.game.started.elapsed()).ok()?;
        Some((Utc::now() - elapsed).timestamp_millis())
    }

    /// The note of the capture at `path`.
    pub(super) fn shot_note(&self, path: &std::path::Path) -> Option<&Entry> {
        self.journal()?.entries.iter().find(|e| e.shot.as_deref().is_some_and(|s| journal::is_shot(s, path)))
    }

    pub(super) fn notes_rows(&self) -> Vec<Row> {
        let Some(j) = self.journal() else {
            return vec![Row::info("none", tr!("Notes are not available for this game"), "").enabled(false)];
        };
        let left = j.left_off.as_ref().map(|l| l.text.clone()).unwrap_or_default();
        let mut rows =
            vec![
                Row::action("moment", tr!("Mark this moment"))
                    .icon(Icon::Flag)
                    .desc(tr!("Saves the time and a screenshot. Write about it later.")),
                Row::action("write", tr!("Write a note")).icon(Icon::Edit).desc(tr!("Opens the on-screen keyboard")),
                Row::text("left-off", tr!("Where I left off"), shorten(&left, LEFT_OFF_CHARS))
                    .icon(Icon::Bookmark)
                    .desc(if left.is_empty() { tr!("What to do when you come back").to_string() } else { left.clone() }),
            ];
        let start = self.session_start_ms().unwrap_or(i64::MAX);
        let now: Vec<&Entry> = j.entries.iter().rev().filter(|e| e.at_ms().is_some_and(|ms| ms >= start)).collect();
        rows.push(Row::header(tr!("This session")));
        if now.is_empty() {
            rows.push(Row::info("none", tr!("Nothing yet"), "Guide + Y").desc(tr!("Guide + Y marks a moment without opening the panel.")));
        }
        rows.extend(now.into_iter().map(|e| self.note_row(e)));
        // The last day with notes before this session.
        let before: Vec<&Entry> = j.entries.iter().rev().filter(|e| e.at_ms().is_some_and(|ms| ms < start)).collect();
        if let Some(day) = before.first().and_then(|e| entry_local(e)).map(|d| d.date_naive()) {
            let label = Local
                .from_local_datetime(&day.and_hms_opt(12, 0, 0).unwrap_or_default())
                .single()
                .map(|d| d.format(tr!("%m/%d")).to_string());
            rows.push(Row::header(tr!("Last time · {day}", day = label.unwrap_or_default())));
            rows.extend(
                before
                    .into_iter()
                    .filter(|e| entry_local(e).is_some_and(|d| d.date_naive() == day))
                    .take(LAST_TIME)
                    .map(|e| self.note_row(e)),
            );
        }
        rows
    }

    /// A row of this tab was pressed; `false` when it is not one.
    pub(super) fn notes_activate(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        match id {
            "moment" => self.with_service(cx, |s, cx| s.mark_moment(cx)),
            "write" => self.open_writing(Writing::New, cx),
            "left-off" => self.open_writing(Writing::LeftOff, cx),
            _ => match id.strip_prefix("note:") {
                Some(note) => self.open_writing(Writing::Edit(note.to_owned()), cx),
                None => return false,
            },
        }
        true
    }

    /// Y on capture `i`: writes its note.
    pub(super) fn capture_note(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.journal().is_none() {
            return;
        }
        if let Some(shot) = self.data.shots.get(i) {
            let what = Writing::Shot(shot.path.clone(), shot.taken_at);
            self.open_writing(what, cx);
        }
    }

    fn open_writing(&mut self, what: Writing, cx: &mut Context<Self>) {
        let Some(j) = self.journal() else { return };
        let (title, initial) = match &what {
            Writing::New => (tr!("New note").to_string(), String::new()),
            Writing::Edit(id) => {
                let e = j.entries.iter().find(|e| &e.id == id);
                let title = match e.and_then(entry_local) {
                    Some(at) if e.is_some_and(|e| e.is_moment() && e.clip.is_some()) => {
                        tr!("Replay at {time}", time = time_label(at)).to_string()
                    }
                    Some(at) if e.is_some_and(Entry::is_moment) => tr!("Moment at {time}", time = time_label(at)).to_string(),
                    _ => tr!("Edit note").to_string(),
                };
                (title, e.map(|e| e.text.clone()).unwrap_or_default())
            }
            Writing::LeftOff => (tr!("Where I left off").to_string(), j.left_off.as_ref().map(|l| l.text.clone()).unwrap_or_default()),
            Writing::Shot(path, _) => {
                (tr!("Note on this screenshot").to_string(), self.shot_note(path).map(|e| e.text.clone()).unwrap_or_default())
            }
        };
        let image = match &what {
            Writing::Edit(id) => j.entries.iter().find(|e| &e.id == id).and_then(|e| self.note_image(e)),
            Writing::Shot(path, _) => self.thumb(path),
            Writing::New | Writing::LeftOff => None,
        };
        let osk = cx.new(|_| Osk::new(title, &initial).compact().image(image));
        let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
            let what = this.writing.take();
            this.osk = None;
            if let (OskEvent::Done(text), Some(what)) = (ev, what) {
                this.save_writing(what, text.trim().to_owned(), cx);
            }
            cx.notify();
        });
        self.osk = Some((osk, sub));
        self.writing = Some(what);
        cx.notify();
    }

    fn save_writing(&mut self, what: Writing, text: String, cx: &mut Context<Self>) {
        let Some((key, j)) = self.data.journal.as_mut() else { return };
        let key = key.clone();
        // The panel's copy changes at once; the file follows.
        let write: Box<dyn FnOnce(&heronite_data::DataPaths) -> anyhow::Result<()> + Send> = match what {
            Writing::New if text.is_empty() => return,
            Writing::New => {
                let entry = Entry::new(None, &text, None, SOURCE_OVERLAY);
                j.entries.push(entry.clone());
                let key = key.clone();
                Box::new(move |p| journal::add(p, &key, &entry))
            }
            Writing::Edit(id) => {
                let Some(e) = j.entries.iter_mut().find(|e| e.id == id) else { return };
                e.text = text.clone();
                let key = key.clone();
                Box::new(move |p| journal::set_text(p, &key, &id, &text).map(|_| ()))
            }
            Writing::LeftOff => {
                j.left_off = (!text.is_empty()).then(|| journal::LeftOff { text: text.clone(), ..Default::default() });
                let key = key.clone();
                Box::new(move |p| journal::set_left_off(p, &key, &text))
            }
            Writing::Shot(path, taken) => {
                let stored = path.to_string_lossy().into_owned();
                match j.entries.iter_mut().find(|e| e.shot.as_deref().is_some_and(|s| journal::is_shot(s, &path))) {
                    Some(e) => {
                        e.text = text.clone();
                        let (key, id) = (key.clone(), e.id.clone());
                        Box::new(move |p| journal::set_text(p, &key, &id, &text).map(|_| ()))
                    }
                    None if text.is_empty() => return,
                    None => {
                        let at = taken.and_then(|ms| Utc.timestamp_millis_opt(ms as i64).single());
                        let entry = Entry::new(at, &text, Some(stored), SOURCE_OVERLAY);
                        j.entries.push(entry.clone());
                        j.entries.sort_by_key(|e| e.at_ms().unwrap_or(0));
                        let key = key.clone();
                        Box::new(move |p| journal::add(p, &key, &entry))
                    }
                }
            }
        };
        let paths = self.paths.clone();
        if !crate::overlay::moment::may_write(&paths) {
            log::info!("overlay debug: journal change not written");
            return;
        }
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { write(&paths) }).await;
            this.update(cx, |this, cx| match res {
                Ok(()) => this.with_service(cx, move |s, cx| s.journal_changed(key, cx)),
                Err(e) => {
                    this.error = Some(tr!("Could not save the note: {err}", err = format!("{e:#}")).into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

impl OverlayPanel {
    /// The preview of the capture a note is about, once made.
    fn note_image(&self, e: &Entry) -> Option<PathBuf> {
        e.shot.as_deref().and_then(|s| self.thumb(&self.shot_source(s)))
    }

    /// A note of the lists: a marked moment asks for its words; the preview
    /// of a capture stands where the time would be.
    fn note_row(&self, e: &Entry) -> Row {
        let at = entry_local(e).map(time_label).unwrap_or_default();
        let id = SharedString::from(format!("note:{}", e.id));
        let image = self.note_image(e);
        if e.is_moment() {
            let (label, icon) = match e.clip {
                Some(_) => (tr!("Replay at {time}", time = at), Icon::Videocam),
                None => (tr!("Moment at {time}", time = at), Icon::Flag),
            };
            return Row::info(id, label, "").icon(icon).desc(tr!("No words yet. Press A to write them.")).image(image);
        }
        let icon = if e.clip.is_some() { Icon::Videocam } else { Icon::Notes };
        let row = Row::info(id, e.text.clone(), at).icon(icon).image(image);
        // The label is one line: a longer note shows whole when focused.
        if e.text.chars().count() > FULL_TEXT_AFTER {
            row.desc(e.text.clone())
        } else {
            row
        }
    }
}
