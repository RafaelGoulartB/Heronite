//! The Journal view of the focus panel: a search box, "where I left off",
//! then each day as text (a paragraph per note, its time in the margin) over
//! a mosaic of the day's screenshots. Days without notes take one line.

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::{Sizable, Size};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::format_playtime_minutes;
use heronite_data::fuzzy::fold;
use heronite_data::journal::{Entry, JournalDay};
use heronite_data::timeline::TimelineDay;
use heronite_i18n::{tr, trn};

use super::{Edit, Target};
use crate::collection::focus::panel;
use crate::collection::CollectionView;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, mini_checkbox, ButtonKind};
use crate::ui::icon;

/// Width of the time margin.
const TIME_W: f32 = 54.;
const MARGIN_GAP: f32 = 12.;
/// Screenshots in a day's mosaic.
const MOSAIC: usize = 5;
const MOSAIC_GAP: f32 = 6.;
/// Thumbnails of a day without notes.
const STRIP: usize = 5;
/// Width of the notes' column under which a quiet day's button says only "Note".
const QUIET_NARROW_W: f32 = 520.;
const TEXT: f32 = 14.4;
const LINE: f32 = 22.;

/// "Sunday, July 5" ("…, 2025" outside the current year).
fn long_date(date: NaiveDate, this_year: i32) -> String {
    let weekday = heronite_i18n::weekday_name(date.weekday().num_days_from_monday());
    let month = heronite_i18n::month_name(date.month());
    if date.year() == this_year {
        tr!("{weekday}, {month} {d}", weekday, month, d = date.day())
    } else {
        tr!("{weekday}, {month} {d}, {year}", weekday, month, d = date.day(), year = date.year())
    }
}

fn day_meta(t: Option<&TimelineDay>) -> String {
    let Some(t) = t else { return String::new() };
    let mut parts = Vec::new();
    if t.played_secs > 0. {
        parts.push(format_playtime_minutes((t.played_secs / 60.).round().max(1.) as u64));
    }
    if t.sessions > 0 {
        parts.push(trn!(t.sessions, "{n} session", "{n} sessions"));
    }
    if !t.shots.is_empty() {
        parts.push(trn!(t.shots.len(), "{n} screenshot", "{n} screenshots"));
    }
    parts.join(" · ")
}

fn time_of(e: &Entry) -> String {
    e.at_ms()
        .and_then(|ms| Local.timestamp_millis_opt(ms).single())
        .map(|t| t.format(heronite_i18n::trc!("time format", "%-I:%M %p")).to_string())
        .unwrap_or_default()
}

/// The moment a new note of `day` is about: after everything already on it,
/// so it lands on the same day and reads last.
fn new_note_at(day: &JournalDay, t: Option<&TimelineDay>, entries: &[Entry], shots_ms: &[Option<f64>]) -> i64 {
    let today = Local::now().date_naive();
    if day.date == today {
        return Local::now().timestamp_millis();
    }
    let start = t.and_then(|t| t.first_start).and_then(|time| Local.from_local_datetime(&day.date.and_time(time)).single());
    let mut at = start.map(|s| s.timestamp_millis() + 60_000).unwrap_or_else(|| {
        Local.from_local_datetime(&day.date.and_hms_opt(12, 0, 0).unwrap_or_default()).single().map(|d| d.timestamp_millis()).unwrap_or(0)
    });
    for i in &day.entries {
        at = at.max(entries[*i].at_ms().unwrap_or(0) + 1000);
    }
    if let Some(t) = t {
        for s in &t.shots {
            if let Some(ms) = shots_ms.get(*s).copied().flatten() {
                at = at.max(ms as i64 + 1000);
            }
        }
    }
    at
}

impl CollectionView {
    pub(in crate::collection) fn journal_section(&mut self, key: &SharedString, main_w: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let header = self.timeline_header(key, main_w, cx);
        let Some(data) = self.journal.data.clone() else {
            return Some(div().flex().flex_col().child(header).child(panel(false).h(px(80.))).into_any_element());
        };
        let (timeline, days) = self.journal_days(key)?;
        let mut days = (*days).clone();
        let this_year = Local::now().year();
        let shots_ms: Vec<Option<f64>> =
            if self.shots.game.as_ref() == Some(key) { self.shots.items.iter().map(|s| s.taken_at).collect() } else { Vec::new() };

        // A note being written on a day not listed yet (today, mostly).
        let new_date = self.journal.new_note_date();
        if let Some(date) = new_date.filter(|d| !days.iter().any(|x| x.date == *d)) {
            let i = days.iter().position(|x| x.date < date).unwrap_or(days.len());
            days.insert(i, JournalDay { date, timeline: None, entries: Vec::new() });
        }

        let query = fold(&self.journal.query);
        let editing_new = |d: &JournalDay| new_date == Some(d.date);
        let matches = |d: &JournalDay| {
            if !query.is_empty() {
                return d.entries.iter().any(|i| fold(&data.entries[*i].text).contains(&query));
            }
            !self.journal.only_notes || !d.entries.is_empty() || editing_new(d)
        };
        let shown: Vec<JournalDay> = days.iter().filter(|d| matches(d)).cloned().collect();
        let more = shown.len().saturating_sub(self.journal.days_shown);
        let content_w = main_w - 2. * 16.8 - TIME_W - MARGIN_GAP;

        let mut list = div().flex().flex_col();
        for (n, day) in shown.iter().take(self.journal.days_shown).enumerate() {
            let t = day.timeline.and_then(|i| timeline.get(i));
            let at = new_note_at(day, t, &data.entries, &shots_ms);
            let el = if day.entries.is_empty() && !editing_new(day) {
                self.journal_quiet_day(n, day, t, at, this_year, content_w, cx)
            } else {
                self.journal_day(n, day, t, &data.entries, at, this_year, content_w, cx)
            };
            list = list.child(el);
        }
        if shown.is_empty() {
            let text = if !query.is_empty() {
                tr!("No notes match the search.")
            } else {
                tr!("No notes yet. Mark a moment in-game or write one here.")
            };
            list = list.child(div().py(px(18.)).text_size(px(13.)).text_color(theme::white_a(0.5)).child(text));
        }
        let footer = (more > 0).then(|| {
            button_sm("journal-more", ButtonKind::Ghost, tr!("Show {n} more", n = more.min(super::DAYS * 2))).on_click(cx.listener(
                |this, _, _, cx| {
                    this.journal.days_shown += super::DAYS * 2;
                    cx.notify();
                },
            ))
        });

        Some(
            div()
                .flex()
                .flex_col()
                .child(header)
                .child(
                    panel(false)
                        .gap(px(4.))
                        .child(self.journal_tools(cx))
                        .child(self.journal_left_off(&data, cx))
                        .child(list)
                        .children(footer.map(|b| div().flex().pt(px(6.)).child(b))),
                )
                .into_any_element(),
        )
    }

    fn journal_tools(&mut self, cx: &mut Context<Self>) -> Div {
        let only = self.journal.only_notes;
        let search = self.journal.search.as_ref().map(crate::ui::fields::search_box);
        let new = add_button("journal-new".into(), tr!("Note"))
            .h(px(30.))
            .on_click(cx.listener(|this, _, window, cx| this.journal_edit(Target::New(Local::now().timestamp_millis()), window, cx)));
        // Narrow (the grid drawer): the switch goes under the search.
        div().flex().flex_wrap().items_center().gap_x(px(14.)).gap_y(px(8.)).pb(px(6.)).children(search).child(new).child(
            div()
                .id("journal-only")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .text_size(px(12.8))
                .text_color(theme::white_a(0.7))
                .child(mini_checkbox("journal-only-sw", only, false).mt(px(0.)))
                .child(tr!("Only days with notes"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.journal.only_notes = !this.journal.only_notes;
                    cx.notify();
                })),
        )
    }

    /// "Where I left off": its text, or a line to write it.
    fn journal_left_off(&mut self, data: &heronite_data::journal::GameJournal, cx: &mut Context<Self>) -> Div {
        let editing = self.journal.edit.as_ref().filter(|e| e.target == Target::LeftOff);
        let caption = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::accent())
            .child(icon(Icon::Bookmark, px(13.), theme::accent()))
            .child(div().flex_1().child(tr!("Where I left off").to_uppercase()))
            .when(editing.is_none() && data.left_off.is_some(), |d| {
                d.child(
                    div()
                        .id("journal-left-off-clear")
                        .flex_none()
                        .size(px(22.))
                        .my(px(-4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .opacity(0.6)
                        .hover(|s| s.opacity(1.).bg(theme::white_a(0.08)))
                        .child(icon(Icon::DeleteOutline, px(15.), theme::white_a(0.7)))
                        .on_click(cx.listener(|this, _, _, cx| this.journal_clear_left_off(cx))),
                )
            });
        let body: AnyElement = match (editing, &data.left_off) {
            (Some(edit), _) => editor(edit, TEXT - 0.8, cx).into_any_element(),
            (None, Some(l)) => div()
                .id("journal-left-off")
                .cursor_text()
                .text_size(px(TEXT - 0.8))
                .line_height(px(LINE - 2.))
                .text_color(theme::white_a(0.92))
                .child(l.text.clone())
                .on_click(cx.listener(|this, _, window, cx| this.journal_edit(Target::LeftOff, window, cx)))
                .into_any_element(),
            (None, None) => div()
                .id("journal-left-off")
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(theme::white_a(0.45))
                .hover(|s| s.text_color(theme::white_a(0.75)))
                .child(tr!("Write where you stopped, to pick it up next time"))
                .on_click(cx.listener(|this, _, window, cx| this.journal_edit(Target::LeftOff, window, cx)))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mb(px(8.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(10.))
            .bg(theme::accent_wash())
            .border_1()
            .border_color(theme::accent_wash_border())
            .child(caption)
            .child(body)
    }

    /// A day with notes: its date, its notes, its screenshots.
    fn journal_day(
        &mut self,
        n: usize,
        day: &JournalDay,
        t: Option<&TimelineDay>,
        entries: &[Entry],
        at: i64,
        this_year: i32,
        content_w: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group: SharedString = format!("jday-{n}").into();
        let head = div()
            .flex()
            .items_baseline()
            .gap(px(10.))
            .child(
                div()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(15.2))
                    .child(long_date(day.date, this_year)),
            )
            .child(div().text_size(px(12.5)).text_color(theme::white_a(0.5)).child(day_meta(t)))
            .child(div().flex_1())
            .child(
                add_button(SharedString::from(format!("jday-add-{n}")), tr!("Note"))
                    .opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
                    .on_click(cx.listener(move |this, _, window, cx| this.journal_edit(Target::New(at), window, cx))),
            );
        let mut col = div()
            .id(group.clone())
            .group(group)
            .flex()
            .flex_col()
            .gap(px(2.))
            .py(px(14.))
            .border_t_1()
            .border_color(theme::white_a(if n == 0 { 0. } else { 0.07 }))
            .child(head.mb(px(4.)));
        for i in &day.entries {
            col = col.child(self.journal_paragraph(&entries[*i], cx));
        }
        let new_here = self.journal.edit.as_ref().filter(|_| self.journal.new_note_date() == Some(day.date));
        if let Some(edit) = new_here {
            let now = Local
                .timestamp_millis_opt(at)
                .single()
                .map(|d| d.format(heronite_i18n::trc!("time format", "%-I:%M %p")).to_string())
                .unwrap_or_default();
            col = col.child(margin_row(div().text_color(theme::accent()).child(now), editor(edit, TEXT, cx)));
        }
        if let Some(t) = t {
            col = col.children(self.journal_mosaic(n, t, entries, content_w, cx));
        }
        col.into_any_element()
    }

    fn journal_paragraph(&mut self, e: &Entry, cx: &mut Context<Self>) -> Div {
        let id = e.id.clone();
        let group: SharedString = format!("jp-{id}").into();
        let time = div()
            .flex()
            .justify_end()
            .items_center()
            .gap(px(3.))
            .text_color(if e.is_moment() { theme::console_btn_y().into() } else { theme::white_a(0.42) })
            .when(e.is_moment(), |d| d.child(icon(Icon::Flag, px(11.), theme::console_btn_y())))
            .child(time_of(e));
        let editing = self.journal.edit.as_ref().filter(|ed| ed.target == Target::Entry(id.clone()));
        if let Some(edit) = editing {
            return margin_row(time, editor(edit, TEXT, cx));
        }
        let shot = e.shot.as_deref().and_then(|s| self.shots.items.iter().position(|x| heronite_data::journal::is_shot(s, &x.path)));
        let thumb = shot.and_then(|i| {
            let path = self.shots.items[i].path.clone();
            self.shots.thumb(&path).map(|src| (i, src))
        });
        let text = if e.is_moment() {
            div().text_color(theme::white_a(0.38)).child(tr!("A moment marked in-game. Click to write about it."))
        } else {
            div().text_color(theme::white_a(0.9)).child(e.text.clone())
        };
        let edit_id = id.clone();
        let body = div()
            .relative()
            .flex()
            .items_start()
            .gap(px(10.))
            .child(
                div()
                    .id(SharedString::from(format!("jp-text-{id}")))
                    .flex_1()
                    .min_w_0()
                    .cursor_text()
                    .rounded(px(4.))
                    .child(text)
                    .on_click(cx.listener(move |this, _, window, cx| this.journal_edit(Target::Entry(edit_id.clone()), window, cx))),
            )
            .children(thumb.map(|(i, src)| {
                div()
                    .id(SharedString::from(format!("jp-shot-{id}")))
                    .flex_none()
                    .w(px(56.))
                    .h(px(32.))
                    .mt(px(-4.))
                    .rounded(px(5.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(img(src).size_full().object_fit(ObjectFit::Cover))
                    .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(i, window, cx)))
            }))
            .child(
                div()
                    .id(SharedString::from(format!("jp-del-{id}")))
                    .flex_none()
                    .size(px(24.))
                    .mt(px(-1.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .opacity(0.)
                    .group_hover(group.clone(), |s| s.opacity(1.))
                    .hover(|s| s.bg(theme::white_a(0.08)))
                    .child(icon(Icon::DeleteOutline, px(15.), theme::white_a(0.6)))
                    .on_click(cx.listener(move |this, _, _, cx| this.journal_delete(&id, cx))),
            );
        margin_row(time, body).group(group)
    }

    /// The day's screenshots: the first large, the next ones beside it; a
    /// note about one shows over it.
    fn journal_mosaic(&mut self, n: usize, t: &TimelineDay, entries: &[Entry], content_w: f32, cx: &mut Context<Self>) -> Option<Div> {
        if t.shots.is_empty() {
            return None;
        }
        let small_w = ((content_w.min(640.) - 3. * MOSAIC_GAP) / 4.).max(60.);
        let small_h = small_w * 9. / 16.;
        let big = (2. * small_w + MOSAIC_GAP, 2. * small_h + MOSAIC_GAP);
        let mut tiles = Vec::new();
        for (j, &idx) in t.shots.iter().take(MOSAIC).enumerate() {
            let Some(path) = self.shots.items.get(idx).map(|s| s.path.clone()) else { continue };
            let src = self.shots.thumb(&path);
            let note = entries
                .iter()
                .find(|e| !e.is_moment() && e.shot.as_deref().is_some_and(|s| heronite_data::journal::is_shot(s, &path)))
                .map(|e| e.text.clone());
            let (w, h) = if j == 0 { big } else { (small_w, small_h) };
            let mut tile = crate::collection::screenshots::thumb_tile(SharedString::from(format!("jm-{n}-{j}")), src, true)
                .flex_none()
                .w(px(w))
                .h(px(h))
                .on_click(cx.listener(move |this, _, window, cx| this.shots_open_view(idx, window, cx)));
            if let Some(note) = note.filter(|_| j == 0 || small_w >= 120.) {
                tile = tile.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .px(px(8.))
                        .py(px(5.))
                        .bg(theme::black_a(0.62))
                        .text_size(px(11.5))
                        .line_height(px(15.))
                        .text_color(white())
                        .truncate()
                        .child(note),
                );
            }
            tiles.push(tile);
        }
        let mut tiles = tiles.into_iter();
        let first = tiles.next()?;
        let rest: Vec<_> = tiles.collect();
        let grid = div().flex().flex_wrap().gap(px(MOSAIC_GAP)).w(px(2. * small_w + MOSAIC_GAP)).children(rest);
        Some(div().flex().gap(px(MOSAIC_GAP)).mt(px(10.)).ml(px(TIME_W + MARGIN_GAP)).child(first).child(grid))
    }

    /// A day without notes, on one line.
    fn journal_quiet_day(
        &mut self,
        n: usize,
        day: &JournalDay,
        t: Option<&TimelineDay>,
        at: i64,
        this_year: i32,
        content_w: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group: SharedString = format!("jq-{n}").into();
        let narrow = content_w < QUIET_NARROW_W;
        let label = if day.date.year() == this_year {
            tr!("{month} {d}", month = heronite_i18n::month_short(day.date.month()), d = day.date.day()).to_string()
        } else {
            tr!("{month} {d}, {year}", month = heronite_i18n::month_short(day.date.month()), d = day.date.day(), year = day.date.year())
                .to_string()
        };
        // Clipped, not pushed out, when the line is short.
        let mut strip = div().flex().flex_1().min_w_0().overflow_hidden().gap(px(3.));
        if let Some(t) = t {
            for &idx in t.shots.iter().take(STRIP) {
                let Some(path) = self.shots.items.get(idx).map(|s| s.path.clone()) else { continue };
                if let Some(src) = self.shots.thumb(&path) {
                    strip = strip.child(
                        div()
                            .flex_none()
                            .w(px(34.))
                            .h(px(19.))
                            .rounded(px(3.))
                            .overflow_hidden()
                            .opacity(0.75)
                            .child(img(src).size_full().object_fit(ObjectFit::Cover)),
                    );
                }
            }
        }
        div()
            .id(group.clone())
            .group(group.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(8.))
            .border_t_1()
            .border_color(theme::white_a(if n == 0 { 0. } else { 0.07 }))
            .text_size(px(12.5))
            .text_color(theme::white_a(0.45))
            .child(
                div()
                    .w(px(TIME_W + MARGIN_GAP))
                    .flex_none()
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(13.))
                    .text_color(theme::white_a(0.65))
                    .child(label),
            )
            .child(div().flex_none().child(day_meta(t)))
            .child(strip)
            .child(
                add_button(SharedString::from(format!("jq-add-{n}")), if narrow { tr!("Note") } else { tr!("Write about this day") })
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .on_click(cx.listener(move |this, _, window, cx| this.journal_edit(Target::New(at), window, cx))),
            )
            .into_any_element()
    }
}

/// The time in the margin, then the text.
fn margin_row(time: Div, body: impl IntoElement) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(MARGIN_GAP))
        .py(px(3.))
        .text_size(px(TEXT))
        .line_height(px(LINE))
        .child(time.flex_none().w(px(TIME_W)).h(px(LINE)).text_size(px(11.5)).line_height(px(LINE)))
        .child(div().flex_1().min_w_0().child(body))
}

/// The open text area: borderless, an accent bar on its left, the keys under it.
fn editor(edit: &Edit, size: f32, cx: &mut Context<CollectionView>) -> Div {
    let hint = |k: &'static str, what: &'static str| div().flex().items_center().gap(px(5.)).child(crate::ui::controls::kbd(k)).child(what);
    // The bar sits in the margin's gap, so the text stays where it was read.
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .ml(px(-MARGIN_GAP))
        .pl(px(MARGIN_GAP - 2.))
        .border_l_2()
        .border_color(theme::accent())
        .text_size(px(size))
        .line_height(px(LINE))
        .text_color(white())
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
            if ev.keystroke.key == "escape" {
                cx.stop_propagation();
                this.journal_cancel(window, cx);
            }
        }))
        // The text area pads its text (8px across, 2px down at this size).
        .child(div().mx(px(-8.)).my(px(-2.)).child(Textarea::new(&edit.state).appearance(false).with_size(Size::Size(px(size / 0.875)))))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(12.))
                .text_size(px(11.5))
                .line_height(px(16.))
                .text_color(theme::white_a(0.42))
                .child(hint("Enter", tr!("save")))
                .child(hint("Shift Enter", tr!("new line")))
                .child(hint("Esc", tr!("cancel"))),
        )
}

fn add_button(id: SharedString, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .h(px(24.))
        .pl(px(6.))
        .pr(px(10.))
        .rounded_full()
        .cursor_pointer()
        .bg(theme::white_a(0.06))
        .hover(|s| s.bg(theme::white_a(0.12)))
        .text_size(px(12.))
        .text_color(theme::white_a(0.75))
        .child(icon(Icon::Add, px(14.), theme::white_a(0.75)))
        .child(label)
}
