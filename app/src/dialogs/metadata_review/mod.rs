//! "Review matches": one game at a time, the user says which game of the
//! sources it is. A strip of the games of this review on top; the game on
//! the left (`game.rs`: cover, title, where it comes from, why it is asked);
//! on the right the games the sources proposed, as radio cards, and a search
//! of every source by another name (`choices.rs`); Skip, "In no source" and
//! "Use this match" below, with ↑ ↓ / Enter / S. Loading a game's
//! proposals, searching and saving run off the UI thread (`actions.rs`).
//!
//! A queued game opens with what the sources proposed when it was queued (no
//! network); a game that is not queued ("Fix matches…") asks the sources
//! live. While jobs are made up (`HERONITE_DEBUG_METADATA_JOB`) or through
//! `HERONITE_DEBUG_DIALOG=review…`, everything is made up (`fake.rs`).

mod actions;
mod choices;
mod fake;
mod game;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_images::ImageRef;
use heronite_metadata::{BulkItem, MatchChoice, ReviewReason};

use super::metadata_update::hub::{hub, MetadataHub};
use super::{DialogCtx, DialogEvent};
use crate::icons::Icon;
use crate::services::widgets::Covers;
use crate::theme;
use crate::ui::controls::{action_button, button, kbd, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::new_input;
use crate::ui::icon;

/// A game of the review, as the Collection hands it over.
#[derive(Clone, Debug)]
pub struct ReviewGame {
    pub item: BulkItem,
    /// `runner:appName`, the key of the queue.
    pub key: String,
    /// It waits in the queue (its proposals are read from there).
    pub queued: bool,
    pub cover: Vec<ImageRef>,
    /// Where it comes from, as the Collection words it ("PlayStation ·
    /// DuckStation", "Steam").
    pub origin: String,
}

/// What became of a game of this review.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    Matched,
    InNoSource,
    Skipped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Busy {
    /// "Use this match" fetches and saves.
    Use,
    InNoSource,
}

pub struct MetadataReview {
    hub: Entity<MetadataHub>,
    games: Vec<ReviewGame>,
    answers: Vec<Option<Answer>>,
    at: usize,
    /// Every game was answered or skipped: the end state.
    finished: bool,
    /// Why the game shown is asked; `None` while it loads.
    reason: Option<ReviewReason>,
    /// The game's proposals are being read (asked live when not queued).
    loading: bool,
    load_error: Option<String>,
    choices: Vec<MatchChoice>,
    picked: Option<usize>,
    search: Entity<InputState>,
    searching: bool,
    /// After a search: sources that did not answer, or that nothing was found.
    search_note: Option<String>,
    /// The search field is emptied at the next frame (a game changed off
    /// the UI thread, where the field cannot be touched).
    clear_search: bool,
    busy: Option<Busy>,
    error: Option<String>,
    covers: Covers,
    /// Made up: nothing is asked or written (`fake.rs`).
    fake: Option<fake::FakeReview>,
    load_task: Option<Task<()>>,
    search_task: Option<Task<()>>,
    _sub: Subscription,
}

impl EventEmitter<DialogEvent> for MetadataReview {}

impl MetadataReview {
    /// `fake`: everything is made up, with these cover addresses as the
    /// proposals' pictures (debug only; see `fake.rs`).
    pub fn new(
        ctx: DialogCtx,
        games: Vec<ReviewGame>,
        start: usize,
        fake: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let hub = hub(cx);
        let search = new_input(window, cx, "", tr!("Search another name in every source"));
        let sub = cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.run_search(cx);
            }
        });
        let fake = fake.map(fake::FakeReview::new);
        let covers = Covers::new(&ctx.data_paths, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        let n = games.len();
        let mut this = Self {
            hub,
            answers: vec![None; n],
            at: start.min(n.saturating_sub(1)),
            finished: n == 0,
            games,
            reason: None,
            loading: false,
            load_error: None,
            choices: Vec::new(),
            picked: None,
            search,
            searching: false,
            search_note: None,
            clear_search: false,
            busy: None,
            error: None,
            covers,
            fake,
            load_task: None,
            search_task: None,
            _sub: sub,
        };
        this.load(cx);
        this
    }

    /// `HERONITE_DEBUG_DIALOG=review:<state>` on a made-up review: `done`
    /// (the end state), `empty` (nothing found for the game), `search` (after a search by another name), `busy`
    /// (asking the sources), `saving` ("Use this match" working), `error`
    /// (saving failed offline).
    pub fn debug_state(&mut self, state: &str, window: &mut Window, cx: &mut Context<Self>) {
        match state {
            "done" => {
                for (i, a) in self.answers.iter_mut().enumerate() {
                    *a = Some(if i % 3 == 2 { Answer::Skipped } else { Answer::Matched });
                }
                self.finished = true;
            }
            "search" => {
                let query = self.game().map(|g| fake::search_query(&g.item.title)).unwrap_or_default();
                self.search.update(cx, |s, cx| s.set_value(query, window, cx));
                self.run_search(cx);
            }
            "empty" => {
                if let Some(fake) = &mut self.fake {
                    fake.empty = true;
                }
                self.load(cx);
            }
            "busy" => {
                self.loading = true;
                self.choices.clear();
                self.reason = None;
            }
            "saving" => self.busy = Some(Busy::Use),
            "error" => self.error = Some(heronite_metadata::service::Offline.to_string()),
            _ => {}
        }
        cx.notify();
    }

    fn game(&self) -> Option<&ReviewGame> {
        self.games.get(self.at).filter(|_| !self.finished)
    }

    /// ↑ ↓ pick, Enter uses the pick, S skips; not while the search field
    /// has the keyboard (Enter searches there).
    pub fn on_key(&mut self, ev: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        if m.control || m.alt || m.platform || self.search.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        if self.finished {
            if ks.key == "enter" {
                cx.emit(DialogEvent::Close);
                return true;
            }
            return false;
        }
        match ks.key.as_str() {
            "up" | "down" if !self.choices.is_empty() => {
                let last = self.choices.len() - 1;
                self.picked = Some(match (self.picked, ks.key.as_str()) {
                    (None, _) => 0,
                    (Some(i), "up") => i.saturating_sub(1),
                    (Some(i), _) => (i + 1).min(last),
                });
                cx.notify();
                true
            }
            "enter" => {
                self.use_match(cx);
                true
            }
            "s" if !m.shift => {
                self.skip(cx);
                true
            }
            _ => false,
        }
    }

    /// The end state: what this review did, and Done.
    fn render_end(&self) -> Div {
        let count = |a: Answer| self.answers.iter().filter(|x| **x == Some(a)).count();
        let (matched, none, skipped) = (count(Answer::Matched), count(Answer::InNoSource), count(Answer::Skipped));
        let mut parts = Vec::new();
        if matched > 0 {
            parts.push(trn!(matched, "{n} matched", "{n} matched"));
        }
        if none > 0 {
            parts.push(trn!(none, "{n} in no source", "{n} in no source"));
        }
        if skipped > 0 {
            parts.push(trn!(skipped, "{n} skipped", "{n} skipped"));
        }
        let (title, line) = if self.games.is_empty() {
            (tr!("No game waits for review"), tr!("Games the sources are not sure about wait here.").to_string())
        } else {
            (tr!("All reviewed"), parts.join(" \u{b7} "))
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .h(px(END_H))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(52.))
                    .rounded_full()
                    .bg(Hsla { a: 0.16, ..theme::success().into() })
                    .child(icon(Icon::Check, px(26.), theme::success())),
            )
            .child(div().text_size(px(17.)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).child(title))
            .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(line))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> Div {
        let row = div().flex().flex_1().items_center().gap(px(10.));
        if self.finished {
            return row.child(div().flex_1()).child(
                button("rv-done", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
            );
        }
        let free = self.busy.is_none() && !self.loading;
        let can_use = free && self.picked.and_then(|i| self.choices.get(i)).is_some_and(|c| !c.picks().is_empty());
        // Nothing was found: only a search or "In no source" can answer.
        let nothing = self.choices.is_empty() && !self.loading && !self.searching && self.load_error.is_none();
        let hint = |text: &'static str| div().text_size(px(12.)).text_color(theme::text_dim()).child(text);
        let keys = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(5.))
            .when(!nothing, |d| {
                d.child(kbd("\u{2191}"))
                    .child(kbd("\u{2193}"))
                    .child(hint(tr!("pick")))
                    .child(hint("\u{b7}"))
                    .child(kbd(tr!("Enter")))
                    .child(hint(tr!("use")))
                    .child(hint("\u{b7}"))
            })
            .child(kbd("S"))
            .child(hint(tr!("skip")));
        let use_label = if self.busy == Some(Busy::Use) { tr!("Saving\u{2026}") } else { tr!("Use this match") };
        row.child(keys)
            .child(action_button(
                "rv-skip",
                ButtonKind::Ghost,
                tr!("Skip"),
                self.busy.is_none(),
                cx.listener(|this, _, _, cx| this.skip(cx)),
            ))
            .child(action_button(
                "rv-none",
                if nothing { ButtonKind::Primary } else { ButtonKind::Glass },
                if self.busy == Some(Busy::InNoSource) { tr!("Saving\u{2026}") } else { tr!("In no source") },
                free,
                cx.listener(|this, _, _, cx| this.in_no_source(cx)),
            ))
            .when(!nothing, |d| {
                d.child(action_button("rv-use", ButtonKind::Primary, use_label, can_use, cx.listener(|this, _, _, cx| this.use_match(cx))))
            })
    }
}

/// Height of the body under the strip, so the dialog keeps its size from
/// one game to the next.
const BODY_H: f32 = 430.;
/// Height of the end state.
const END_H: f32 = 220.;

impl Render for MetadataReview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.covers.drop_evicted(window);
        if std::mem::take(&mut self.clear_search) {
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let body = if self.finished {
            self.render_end()
        } else {
            div()
                .flex()
                .flex_col()
                .child(self.render_strip(window, cx))
                .child(div().flex().gap(px(28.)).h(px(BODY_H)).child(self.render_game(window)).child(self.render_choices(window, cx)))
        };
        let footer = self.render_footer(cx);
        let entity = cx.entity().downgrade();
        let area = crate::ui::content_size(window);
        dialog(
            DialogProps {
                id: "metadata-review".into(),
                title: tr!("Review matches").into(),
                width: 900f32.min(f32::from(area.width) * 0.94),
                height: None,
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
