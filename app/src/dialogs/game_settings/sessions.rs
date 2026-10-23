//! The "Sessions" row of the Playtime card: what the play sessions add up
//! to, "Recalculate" (the time played takes that sum, saved with Save) and,
//! expanded, the sessions [`PAGE`] at a time, newest first, to correct,
//! delete or add by hand (`heronite_data::sessions`). Each session change is
//! written at once, one session at a time.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::models::LocalGameSession;
use heronite_data::playtime::{clamp_number, split_minutes};
use heronite_data::sessions::{
    date_text, edit_session, format_session_date, local_instant, parse_date, parse_time, session_from, session_source_label,
    session_start_local, sessions_total_minutes, sorted_newest_first, time_text, MANUAL_SOURCE,
};
use heronite_i18n::{tr, trn};

use super::GameSettingsDialog;
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::fields::{duration_field, input_box, labeled, new_input};
use crate::ui::icon;

/// Sessions per page of the expanded list.
const PAGE: usize = 3;

pub(super) struct SessionsState {
    /// Newest first.
    pub(super) list: Vec<LocalGameSession>,
    /// The list under the row is open.
    expanded: bool,
    /// Page of the list (0 = the newest [`PAGE`]).
    page: usize,
    editor: Option<SessionEditor>,
    /// Index (in `list`) of the session asking "Delete?" in its row.
    deleting: Option<usize>,
    saving: bool,
}

struct SessionEditor {
    /// `None`: a new session.
    original: Option<LocalGameSession>,
    date: Entity<InputState>,
    time: Entity<InputState>,
    hours: Entity<InputState>,
    minutes: Entity<InputState>,
    /// What the fields started with, to keep the seconds of an untouched
    /// start or length.
    initial: (String, String, u64),
}

/// What the editor's fields hold.
enum Draft {
    Ready(LocalGameSession),
    /// An edited session whose fields still say what it was.
    Unchanged,
    /// Why it cannot be saved.
    Invalid(String),
}

pub(super) fn sorted(sessions: &[LocalGameSession]) -> Vec<LocalGameSession> {
    sorted_newest_first(sessions).into_iter().cloned().collect()
}

impl SessionsState {
    pub(super) fn new(sessions: &[LocalGameSession]) -> Self {
        Self { list: sorted(sessions), expanded: false, page: 0, editor: None, deleting: None, saving: false }
    }
}

impl GameSettingsDialog {
    /// Opens the editor on the session at `index`, or on a new one (now,
    /// one hour).
    fn edit_session(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let locale = self.ctx.lib.date_locale;
        let original = index.and_then(|i| self.sessions.list.get(i)).cloned();
        let (start, seconds) = match &original {
            Some(s) => (session_start_local(s).unwrap_or_else(chrono::Local::now), s.elapsed_seconds.get().max(0.0) as u64),
            None => (chrono::Local::now(), 3600),
        };
        let (date, time) = (date_text(&start, locale), time_text(chrono::Timelike::hour(&start), chrono::Timelike::minute(&start), locale));
        let minutes = seconds / 60;
        let (h, m) = split_minutes(minutes);
        let editor = SessionEditor {
            date: new_input(window, cx, &date, &date),
            time: new_input(window, cx, &time, &time),
            hours: new_input(window, cx, &h.to_string(), "0"),
            minutes: new_input(window, cx, &m.to_string(), "0"),
            initial: (date, time, minutes),
            original,
        };
        for input in [&editor.date, &editor.time, &editor.hours, &editor.minutes] {
            cx.subscribe_in(input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        if index.is_none() {
            self.sessions.page = 0;
        }
        self.sessions.expanded = true;
        self.sessions.editor = Some(editor);
        self.sessions.deleting = None;
        cx.notify();
    }

    /// `HERONITE_DEBUG_SESSION=open|page:<n>|new|invalid|edit:<row>|delete:<row>|recalc`:
    /// the Sessions list starts open (on a page), with the editor
    /// (`invalid`: holding a date that does not exist) or a delete question
    /// open, or the time played recalculated (captures).
    pub(super) fn debug_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = crate::env::value("HERONITE_DEBUG_SESSION") else { return };
        let row = |p: &str| which.strip_prefix(p).and_then(|i| i.parse::<usize>().ok()).filter(|i| *i < self.sessions.list.len());
        if which == "new" || which == "invalid" {
            self.edit_session(None, window, cx);
            if let Some(e) = self.sessions.editor.as_ref().filter(|_| which == "invalid") {
                e.date.update(cx, |s, cx| s.set_value("31/02/2026", window, cx));
            }
        } else if which == "recalc" {
            self.recalculate_playtime(window, cx);
        } else if which == "open" {
            self.sessions.expanded = true;
        } else if let Some(page) = which.strip_prefix("page:").and_then(|p| p.parse::<usize>().ok()) {
            self.sessions.expanded = true;
            self.sessions.page = page.min(self.sessions.list.len().saturating_sub(1) / PAGE);
        } else if let Some(i) = row("edit:") {
            self.sessions.page = i / PAGE;
            self.edit_session(Some(i), window, cx);
        } else if let Some(i) = row("delete:") {
            self.sessions.expanded = true;
            self.sessions.page = i / PAGE;
            self.sessions.deleting = Some(i);
        }
    }

    fn session_draft(&self, cx: &App) -> Option<Draft> {
        let e = self.sessions.editor.as_ref()?;
        let locale = self.ctx.lib.date_locale;
        let text = |i: &Entity<InputState>| i.read(cx).value().trim().to_string();
        let (date, time) = (text(&e.date), text(&e.time));
        let minutes = clamp_number(&text(&e.hours), None) * 60 + clamp_number(&text(&e.minutes), Some(59));
        let Some(day) = parse_date(&date, locale) else {
            let example = date_text(&chrono::Local::now(), locale);
            return Some(Draft::Invalid(tr!("Type the date like {example}.", example)));
        };
        let Some(clock) = parse_time(&time) else {
            return Some(Draft::Invalid(tr!("Type the time as hours:minutes, like {example}.", example = time_text(21, 30, locale))));
        };
        if minutes == 0 {
            return Some(Draft::Invalid(tr!("A session lasts at least one minute.").into()));
        }
        let (initial_date, initial_time, initial_minutes) = &e.initial;
        let old = e.original.as_ref();
        // A start or a length left as it was keeps its exact value (the
        // fields only show minutes).
        let same_start = old.filter(|_| date == *initial_date && time == *initial_time);
        let start = match same_start.and_then(|o| chrono::DateTime::parse_from_rfc3339(&o.started_at).ok()) {
            Some(t) => t.with_timezone(&chrono::Utc),
            None => match local_instant(day.and_time(clock)) {
                Some(t) => t,
                None => return Some(Draft::Invalid(tr!("That time does not exist on that day (clock change).").into())),
            },
        };
        let seconds = match old {
            Some(o) if minutes == *initial_minutes => o.elapsed_seconds.get().max(0.0) as u64,
            _ => minutes * 60,
        };
        if start > chrono::Utc::now() {
            return Some(Draft::Invalid(tr!("The session cannot start in the future.").into()));
        }
        let mut session = session_from(start, seconds, old.map_or(MANUAL_SOURCE, |o| o.source.as_str()));
        if let Some(o) = old {
            if same_start.is_some() {
                session.started_at = o.started_at.clone();
                if seconds == o.elapsed_seconds.get().max(0.0) as u64 {
                    return Some(Draft::Unchanged);
                }
            }
            session.game_action_name = o.game_action_name.clone();
            session.extra = o.extra.clone();
        }
        Some(Draft::Ready(session))
    }

    fn save_session(&mut self, cx: &mut Context<Self>) {
        let Some(Draft::Ready(session)) = self.session_draft(cx) else { return };
        let old = self.sessions.editor.as_ref().and_then(|e| e.original.clone());
        self.write_session(old, Some(session), cx);
    }

    /// Writes one change off the UI thread; the Collection reloads it.
    fn write_session(&mut self, old: Option<LocalGameSession>, new: Option<LocalGameSession>, cx: &mut Context<Self>) {
        if self.sessions.saving {
            return;
        }
        self.sessions.saving = true;
        self.error.clear();
        let (paths, app_name) = (self.ctx.data_paths.clone(), self.game.app_name.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { edit_session(&paths, &app_name, old.as_ref(), new) }).await;
            this.update(cx, |this, cx| {
                this.sessions.saving = false;
                match res {
                    Ok(list) => {
                        this.sessions.list = sorted(&list);
                        this.sessions.page = this.sessions.page.min(list.len().saturating_sub(1) / PAGE);
                        this.sessions.editor = None;
                        this.sessions.deleting = None;
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The time-played fields take what the sessions add up to (saved with Save).
    fn recalculate_playtime(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (h, m) = split_minutes(sessions_total_minutes(&self.sessions.list));
        let d = &self.details;
        d.hours.update(cx, |s, cx| s.set_value(h.to_string(), window, cx));
        d.minutes.update(cx, |s, cx| s.set_value(m.to_string(), window, cx));
        cx.notify();
    }

    /// The "Sessions" row and, when expanded, its list (no separator: they
    /// read as one row of the Playtime card).
    pub(super) fn sessions_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.sessions.list.len();
        let total = sessions_total_minutes(&self.sessions.list);
        let matches = total == self.playtime(cx);
        let desc = if count == 0 {
            tr!("No sessions recorded yet.").to_string()
        } else if matches {
            trn!(count, "{n} session, the same as the time played.", "{n} sessions, the same as the time played.")
        } else {
            trn!(count, "{n} session adds up to {time}.", "{n} sessions add up to {time}.", time = format_playtime_minutes(total))
        };
        let expanded = self.sessions.expanded;
        let toggle = cx.listener(|this, _, _, cx| {
            this.sessions.expanded = !this.sessions.expanded;
            cx.notify();
        });
        let chevron = div()
            .id("gs-sessions-chevron")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.08)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.sessions.expanded = !this.sessions.expanded;
                cx.notify();
            }))
            .child(
                icon(Icon::ExpandMore, px(20.), theme::text_muted())
                    .when(expanded, |i| i.with_transformation(Transformation::rotate(percentage(0.5)))),
            );
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(11.))
            .child(
                div().id("gs-sessions-toggle").flex().flex_1().min_w_0().items_center().cursor_pointer().on_click(toggle).child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(px(1.))
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(tr!("Sessions")))
                        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(desc)),
                ),
            )
            .when(count > 0 && !matches, |d| {
                d.child(
                    button_sm("gs-playtime-recalc", ButtonKind::Glass, tr!("Recalculate"))
                        .on_click(cx.listener(|this, _, window, cx| this.recalculate_playtime(window, cx))),
                )
            })
            .child(chevron);
        div().flex().flex_col().child(head).when(expanded, |d| d.child(self.sessions_list(cx))).into_any_element()
    }

    /// The open list: one page of sessions (or the editor in place of one),
    /// then "Add session" and the pager.
    fn sessions_list(&self, cx: &mut Context<Self>) -> Div {
        let count = self.sessions.list.len();
        let start = (self.sessions.page * PAGE).min(count);
        let end = (start + PAGE).min(count);
        let mut rows = div().flex().flex_col().p(px(4.)).rounded(px(12.)).bg(theme::recess_a(0.18));
        let adding = self.sessions.editor.as_ref().is_some_and(|e| e.original.is_none());
        if adding {
            rows = rows.child(self.session_editor(cx));
        }
        for i in start..end {
            let s = &self.sessions.list[i];
            let editing = self
                .sessions
                .editor
                .as_ref()
                .and_then(|e| e.original.as_ref())
                .is_some_and(|o| o.started_at == s.started_at && o.source == s.source);
            rows = rows.child(if editing { self.session_editor(cx).into_any_element() } else { self.session_row(i, cx) });
        }
        if count == 0 && !adding {
            rows = rows.child(
                div()
                    .px(px(10.))
                    .py(px(9.))
                    .text_size(px(13.))
                    .text_color(theme::text_dim())
                    .child(tr!("They are recorded each time the game runs here.")),
            );
        }
        let free = !self.sessions.saving && !adding;
        let add = div()
            .id("gs-session-add")
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(13.))
            .text_color(theme::accent())
            .child(icon(Icon::Add, px(16.), theme::accent()))
            .child(tr!("Add session"))
            .when(free, |d| {
                d.cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .on_click(cx.listener(|this, _, window, cx| this.edit_session(None, window, cx)))
            })
            .when(!free, |d| d.opacity(0.45));
        let pages = count.div_ceil(PAGE);
        let page = self.sessions.page;
        let pager = div().flex().items_center().gap(px(2.)).when(pages > 1, |d| {
            d.child(div().mr(px(6.)).text_size(px(12.)).text_color(theme::text_dim()).child(tr!(
                "{from}–{to} of {total}",
                from = start + 1,
                to = end,
                total = count
            )))
            .child(self.page_button("gs-session-prev", Icon::ChevronLeft, page > 0, page.saturating_sub(1), cx))
            .child(self.page_button("gs-session-next", Icon::ChevronRight, page + 1 < pages, page + 1, cx))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pb(px(12.))
            .child(rows)
            .child(div().flex().items_center().justify_between().h(px(28.)).px(px(4.)).child(add).child(pager))
    }

    fn page_button(&self, id: &'static str, ic: Icon, enabled: bool, to: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        bare_icon(id, ic, enabled).when(enabled, |b| {
            b.on_click(cx.listener(move |this, _, _, cx| {
                this.sessions.page = to;
                this.sessions.deleting = None;
                cx.notify();
            }))
        })
    }

    fn session_row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.sessions.list[i];
        let when = format_session_date(&s.started_at, self.ctx.lib.date_locale);
        let base = || div().flex().items_center().gap(px(8.)).min_h(px(36.)).pl(px(10.)).pr(px(4.)).rounded(px(9.));
        if self.sessions.deleting == Some(i) {
            let old = s.clone();
            return base()
                .bg(theme::danger_fill())
                .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(tr!("Delete the session of {date}?", date = when)))
                .child(button_sm(SharedString::from(format!("gs-session-keep-{i}")), ButtonKind::Ghost, tr!("Cancel")).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.sessions.deleting = None;
                        cx.notify();
                    }),
                ))
                .child(
                    button_sm(SharedString::from(format!("gs-session-del-{i}")), ButtonKind::Danger, tr!("Delete"))
                        .on_click(cx.listener(move |this, _, _, cx| this.write_session(Some(old.clone()), None, cx))),
                )
                .into_any_element();
        }
        let length = format_playtime_minutes(s.elapsed_seconds.get().max(0.0) as u64 / 60);
        let free = !self.sessions.saving;
        base()
            .hover(|s| s.bg(theme::ink_a(0.04)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_baseline()
                    .gap(px(6.))
                    .child(div().flex_none().text_size(px(13.)).text_color(theme::text_primary()).child(when))
                    .child(
                        div().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(session_source_label(&s.source)),
                    ),
            )
            .child(div().flex_none().mr(px(4.)).text_size(px(13.)).text_color(theme::text_muted()).child(length))
            .child(
                bare_icon(SharedString::from(format!("gs-session-edit-{i}")), Icon::Edit, free)
                    .when(free, |b| b.on_click(cx.listener(move |this, _, window, cx| this.edit_session(Some(i), window, cx)))),
            )
            .child(bare_icon(SharedString::from(format!("gs-session-rm-{i}")), Icon::DeleteOutline, free).when(free, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| {
                    this.sessions.deleting = Some(i);
                    cx.notify();
                }))
            }))
            .into_any_element()
    }

    /// The fields of one session, in place of its row (or on top, for a new one).
    fn session_editor(&self, cx: &mut Context<Self>) -> Div {
        let Some(e) = self.sessions.editor.as_ref() else { return div() };
        let draft = self.session_draft(cx);
        let ends = match &draft {
            Some(Draft::Ready(s)) => Some(s.ended_at.as_str()),
            Some(Draft::Unchanged) => e.original.as_ref().map(|o| o.ended_at.as_str()),
            _ => None,
        };
        let note = match (&draft, ends) {
            (Some(Draft::Invalid(why)), _) => div().text_color(theme::danger_text()).child(why.clone()),
            (_, Some(end)) if !end.is_empty() => {
                div().text_color(theme::text_dim()).child(tr!("Ends {when}.", when = format_session_date(end, self.ctx.lib.date_locale)))
            }
            _ => div(),
        };
        let fields = div()
            .flex()
            .flex_wrap()
            .gap(px(10.))
            .child(div().flex_1().min_w(px(120.)).child(labeled(tr!("Date"), input_box(&e.date, false))))
            .child(div().w(px(104.)).flex_none().child(labeled(tr!("Start time"), input_box(&e.time, false))))
            .child(div().flex_none().child(labeled(tr!("Length"), duration_field(&e.hours, &e.minutes))));
        let can_save = matches!(draft, Some(Draft::Ready(_))) && !self.sessions.saving;
        let saving = self.sessions.saving;
        let buttons = div()
            .flex()
            .flex_none()
            .gap(px(6.))
            .child(button_sm("gs-session-cancel", ButtonKind::Ghost, tr!("Cancel")).when(!saving, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    this.sessions.editor = None;
                    cx.notify();
                }))
            }))
            .child(
                button_sm("gs-session-save", ButtonKind::Primary, if saving { tr!("Saving…") } else { tr!("Save") })
                    .when(can_save, |b| b.on_click(cx.listener(|this, _, _, cx| this.save_session(cx))))
                    .when(!can_save, |b| b.opacity(0.45)),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(10.))
            .rounded(px(9.))
            .bg(theme::ink_a(0.04))
            .child(fields)
            .child(div().flex().items_center().gap(px(10.)).child(div().flex_1().min_w_0().text_size(px(12.)).child(note)).child(buttons))
    }
}

/// A 28px round icon button without fill (the list's edit, delete and pager).
fn bare_icon(id: impl Into<ElementId>, ic: Icon, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded_full()
        .child(icon(ic, px(16.), theme::text_muted()))
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(theme::ink_a(0.08))))
        .when(!enabled, |d| d.opacity(0.35))
}
