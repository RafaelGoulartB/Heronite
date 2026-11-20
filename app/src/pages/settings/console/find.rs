//! Search in console Settings.
//!
//! Search types on the on-screen keyboard and looks through every section
//! the sidebar lists: its name, group and description, and the label and
//! description of each of its rows (the Heroic panel's sections too). The
//! sections that match come first, then the settings whose name matches,
//! then the ones whose description does.
//!
//! A on a result shows its section with that row focused.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::fuzzy::fold;
use heronite_i18n::tr;

use super::{console_mode, entries, CSec, Entry, Section, SettingsPage};
use crate::console::list::{Outcome, Row, Value};
use crate::console::Pad;
use crate::dialogs::settings_panel::logic::Section as HSection;
use crate::dialogs::settings_panel::GlobalView;

/// Results listed at most.
const MAX_HITS: usize = 80;

#[derive(Default)]
pub(super) struct State {
    query: String,
    hits: Vec<Hit>,
}

/// A result: where it is, and how it reads.
#[derive(Clone)]
struct Hit {
    sec: CSec,
    /// The row to focus; `None`: the section itself.
    row: Option<SharedString>,
    title: SharedString,
    desc: Option<SharedString>,
}

impl Hit {
    fn of_row(sec: CSec, r: &Row) -> Self {
        Hit { sec, row: Some(r.id.clone()), title: format!("{} \u{b7} {}", sec.label(), r.label).into(), desc: r.desc.clone() }
    }
}

/// Whether every word of `words` is in `text` (both folded).
fn has_all(text: &str, words: &[&str]) -> bool {
    !words.is_empty() && words.iter().all(|w| text.contains(w))
}

/// The hits of `words` in `sources`, best first: the sections whose name
/// matches, the rows whose label matches, then the rest.
fn search(words: &[&str], sources: &[(CSec, Vec<Row>)]) -> Vec<Hit> {
    let mut scored: Vec<(u8, Hit)> = Vec::new();
    for (sec, rows) in sources {
        let name = fold(&format!("{} {}", sec.label(), sec.group()));
        let about = fold(sec.blurb());
        if has_all(&name, words) || has_all(&format!("{name} {about}"), words) {
            let rank = if has_all(&name, words) { 0 } else { 2 };
            let hit = Hit { sec: *sec, row: None, title: sec.label().into(), desc: Some(sec.blurb().into()) };
            scored.push((rank, hit));
        }
        let mut heading = String::new();
        for r in rows {
            if r.value == Value::Header {
                heading = fold(&r.label);
                continue;
            }
            if r.label.trim().is_empty() {
                continue;
            }
            let label = fold(&r.label);
            let desc = r.desc.as_deref().map(fold).unwrap_or_default();
            let rank = if has_all(&label, words) {
                1
            } else if has_all(&format!("{label} {desc} {heading}"), words) {
                3
            } else {
                continue;
            };
            scored.push((rank, Hit::of_row(*sec, r)));
        }
    }
    scored.sort_by_key(|(rank, _)| *rank);
    scored.into_iter().map(|(_, h)| h).take(MAX_HITS).collect()
}

impl SettingsPage {
    /// Every section's rows, for Search (the panels and Console mode are
    /// made when they were not shown yet).
    fn console_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<(CSec, Vec<Row>)> {
        let mut out = Vec::new();
        for (e, _) in entries() {
            let Entry::Sec(sec) = e else { continue };
            let rows = match sec {
                CSec::Panel(s) => {
                    let view = s.global_view().unwrap_or(GlobalView::General);
                    let panel = self.panel_for(view, window, cx);
                    panel.read(cx).console_rows_of(HSection::General, view, cx)
                }
                CSec::Defaults(h) => {
                    let panel = self.defaults_panel(h, window, cx);
                    panel.read(cx).console_rows_of(h, GlobalView::General, cx)
                }
                CSec::Rows(Section::Logs) => Vec::new(),
                CSec::Rows(Section::ConsoleMode) => {
                    console_mode::shown(self, window, cx);
                    self.console_rows(sec, cx)
                }
                _ => self.console_shown_rows(sec, window, cx),
            };
            out.push((sec, rows));
        }
        out
    }

    /// Searches for `query` (the results replace the last ones).
    pub(super) fn console_search(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        let folded = fold(&query);
        let words: Vec<&str> = folded.split_whitespace().collect();
        let hits = if words.is_empty() { Vec::new() } else { search(&words, &self.console_sources(window, cx)) };
        self.console.find.query = query;
        self.console.find.hits = hits;
    }

    /// Search's rows: the field, then the results.
    pub(super) fn console_find_rows(&self) -> Vec<Row> {
        let f = &self.console.find;
        let mut rows = Vec::new();
        let mut field = Row::text("find:query", tr!("Search settings"), f.query.clone());
        if f.query.is_empty() {
            field = field.verb(tr!("Search"));
        }
        rows.push(field);
        if !f.query.is_empty() {
            rows.push(Row::header(if f.hits.is_empty() { tr!("No settings match.") } else { tr!("Results") }));
        }
        rows.extend(f.hits.iter().enumerate().map(|(i, h)| {
            let row = Row::action(SharedString::from(format!("find:{i}")), h.title.clone());
            match &h.desc {
                Some(d) => row.desc(d.clone()),
                None => row,
            }
        }));
        rows
    }

    /// A press on Search's rows.
    pub(super) fn console_find_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let rows = self.console_find_rows();
        match self.console.list.on_pad(pad, &rows) {
            Outcome::Activate(id) if id.as_ref() == "find:query" => self.console_search_keyboard(window, cx),
            Outcome::Activate(id) => {
                if let Some(i) = id.strip_prefix("find:").and_then(|n| n.parse::<usize>().ok()) {
                    self.console_open_hit(i, window, cx);
                }
            }
            Outcome::Moved | Outcome::Changed(..) => {}
            Outcome::Ignored | Outcome::Edge(_) => return false,
        }
        true
    }

    /// The keyboard for Search; Done searches and focuses the first result.
    pub(super) fn console_search_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.console.find.query.clone();
        self.console_osk(tr!("Search settings"), &query, window, cx, |this, text, window, cx| {
            this.console_search(text.trim().to_string(), window, cx);
            this.console.content = true;
            this.console.list.reset();
            let rows = this.console_find_rows();
            if !this.console.find.hits.is_empty() {
                this.console.list.focus_id(&rows, "find:0");
            }
        });
    }

    /// Shows the section of result `i` with its row focused.
    fn console_open_hit(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.console.find.hits.get(i).cloned() else { return };
        let Some(nav) = entries().iter().position(|(e, _)| *e == Entry::Sec(hit.sec)) else { return };
        self.console.nav = nav;
        self.console_shown(window, cx);
        self.console.content = true;
        let Some(id) = hit.row else { return };
        match hit.sec {
            CSec::Panel(_) | CSec::Defaults(_) => {
                if let Some(panel) = self.console_panel(hit.sec, window, cx) {
                    panel.update(cx, |p, cx| p.console_focus_row(&id, cx));
                }
            }
            sec => {
                let rows = self.console_shown_rows(sec, window, cx);
                self.console.list.focus_id(&rows, &id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{search, CSec, Section};
    use crate::console::list::Row;

    #[test]
    fn sections_rank_before_rows_and_every_word_must_match() {
        let sources = vec![
            (CSec::Rows(Section::Library), vec![Row::header("Hidden"), Row::toggle("show-hidden", "Show hidden games", true)]),
            (CSec::Rows(Section::Screenshots), vec![Row::toggle("x", "Keep captures", true).desc("Library of shots")]),
        ];
        let hits = search(&["library"], &sources);
        assert_eq!(hits[0].row, None, "the Library section itself first");
        assert!(hits.iter().any(|h| h.row.as_deref() == Some("x")), "a description matches too");
        // The row's own label ranks before the section whose description has the words.
        let hits = search(&["show", "hidden"], &sources);
        assert_eq!(hits[0].row.as_deref(), Some("show-hidden"));
        assert!(hits[1..].iter().all(|h| h.row.is_none()));
        assert!(search(&["show", "nothing"], &sources).is_empty());
    }
}
