//! "Found as": a row per details source and what it matched. A match the
//! merge left out (another game than the others found) is amber; a pinned
//! one has a pin; "Change" searches the source for another game. A source
//! asked only as a fallback, which the others made unnecessary, keeps its
//! row with "Search"; the other sources that were not asked fold into one
//! muted line per reason. Below, "Search
//! every source again"; above, the busy line, the offline line and the hint
//! of a missing IGDB key.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::{NotAsked, SourceAnswer, SourceState};

use super::{Busy, MetadataDialog};
use crate::dialogs::metadata_update::words;
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::theme;
use crate::ui::controls::{icon_button_sm, ButtonKind};
use crate::ui::icon;

const ROW_H: f32 = 58.;

/// A muted line, or an accent link, in the column.
fn line(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(text.into())
}

impl MetadataDialog {
    fn link(&self, id: SharedString, label: &'static str, source: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let free = self.busy.is_none();
        div()
            .id(id)
            .flex_none()
            .text_size(px(12.5))
            .text_color(theme::accent())
            .when(free, |d| {
                d.cursor_pointer()
                    .hover(|s| s.text_color(theme::text_bright()))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_change(&source, window, cx)))
            })
            .when(!free, |d| d.opacity(0.5))
            .child(label)
    }

    fn source_row(&self, state: &SourceState, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = state.source.clone();
        let name = words::source_name(&self.service, &id);
        let sources = self.service.sources();
        let descriptor = sources.get(&id);
        let searchable = descriptor.is_some_and(|d| d.searchable);
        // A source of emulated games matches on a platform: it shows which.
        let by_platform = descriptor.is_some_and(|d| {
            matches!(d.applies_to, heronite_metadata::sources::AppliesTo::Emulated | heronite_metadata::sources::AppliesTo::Platforms(_))
        });
        let review = self.preview.as_ref().is_some_and(|p| p.review.is_some());
        let (detail, amber, action): (String, bool, Option<&'static str>) = match &state.answer {
            SourceAnswer::Found { hit, used, .. } => {
                let platform = hit.platform.clone().filter(|_| by_platform);
                let mut text = [hit.title.clone(), hit.year.map(|y| y.to_string()), platform]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" \u{b7} ");
                if let Some(by) = hit.matched_by.and_then(words::matched_by) {
                    text = format!("{text} \u{b7} {by}");
                }
                let left_out = !used;
                let text = if left_out && !review { tr!("{game} (left out: another game)", game = text.as_str()) } else { text };
                (text, left_out, searchable.then_some(tr!("Change")))
            }
            SourceAnswer::NotFound => (tr!("Not found").into(), false, searchable.then_some(tr!("Search"))),
            SourceAnswer::Unreachable => (tr!("Could not be reached").into(), false, None),
            // A fallback source the others made unnecessary: it can still be searched.
            SourceAnswer::NotAsked { why: NotAsked::Fallback } => {
                (tr!("Not asked: the others found it").into(), false, searchable.then_some(tr!("Search")))
            }
            // Folded into the line under the rows.
            SourceAnswer::NotAsked { .. } => (String::new(), false, None),
        };
        let pinned = self.pinned(&id);
        let found = matches!(state.answer, SourceAnswer::Found { .. });
        let warn: Hsla = theme::warning().into();
        div()
            .id(SharedString::from(format!("md-found-{id}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(ROW_H))
            .px(px(12.))
            .rounded(px(theme::R_TILE))
            .border_1()
            .when(amber, |d| d.bg(Hsla { a: 0.07, ..warn }).border_color(Hsla { a: 0.45, ..warn }))
            .when(!amber, |d| d.bg(theme::card_fill()).border_color(theme::card_border()))
            .when(!found, |d| d.opacity(0.85))
            .child(labels::mark(&id, &name, 28.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .min_w_0()
                            .child(div().truncate().text_size(px(13.5)).text_color(theme::text_primary()).child(name))
                            .when(pinned, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("md-pin-{id}")))
                                        .flex_none()
                                        .tooltip(crate::ui::tooltip(tr!("Pinned: every update uses this game")))
                                        .child(icon(Icon::PushPin, px(12.), theme::accent())),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(15.))
                            .max_h(px(30.))
                            .overflow_hidden()
                            .text_color(if amber { theme::caution_text() } else { theme::text_muted() })
                            .child(detail),
                    ),
            )
            .children(action.map(|label| self.link(SharedString::from(format!("md-change-{id}")), label, id.clone(), cx)))
    }

    /// The sources not asked, one line per reason ("Off: Lutris").
    fn not_asked(&self) -> Vec<Div> {
        let Some(p) = &self.preview else { return Vec::new() };
        let names = |why: NotAsked| {
            p.sources
                .iter()
                .filter(|s| matches!(s.answer, SourceAnswer::NotAsked { why: w } if w == why))
                .map(|s| words::source_name(&self.service, &s.source))
                .collect::<Vec<_>>()
        };
        [NotAsked::NotForThisGame, NotAsked::Off, NotAsked::NeedsKey, NotAsked::NeedsDownload, NotAsked::Failing, NotAsked::NotChosen]
            .into_iter()
            .filter_map(|why| {
                let list = names(why);
                if list.is_empty() {
                    return None;
                }
                let sources = list.join(", ");
                Some(line(match why {
                    NotAsked::NotForThisGame => tr!("Not for this game: {sources}", sources),
                    NotAsked::Off => tr!("Off: {sources}", sources),
                    NotAsked::NeedsKey => tr!("Needs a key: {sources}", sources),
                    NotAsked::NeedsDownload => tr!("Needs a download: {sources}", sources),
                    NotAsked::Failing => tr!("Stopped after errors: {sources}", sources),
                    NotAsked::NotChosen | NotAsked::Fallback => tr!("Not asked: {sources}", sources),
                }))
            })
            .collect()
    }

    pub(super) fn render_found_as(&mut self, cx: &mut Context<Self>) -> Div {
        let mut list = div().id("md-found-list").flex().flex_col().gap(px(6.)).flex_1().min_h_0().overflow_y_scroll();
        let preview = self.preview.clone();
        match &preview {
            None if self.busy == Some(Busy::Preview) => {
                list = list.child(busy_line(tr!("Asking every source\u{2026}")));
            }
            None => {}
            Some(p) if p.offline => {
                list = list.child(
                    line(heronite_metadata::service::Offline.to_string()).px(px(4.)).text_color(theme::caution_text()).text_size(px(12.5)),
                );
            }
            Some(p) => {
                let row = |s: &&SourceState| {
                    !matches!(s.answer, SourceAnswer::NotAsked { .. }) || s.answer == SourceAnswer::NotAsked { why: NotAsked::Fallback }
                };
                for state in p.sources.iter().filter(row) {
                    list = list.child(self.source_row(state, cx));
                }
                let folded = self.not_asked();
                if !folded.is_empty() {
                    list = list.child(div().flex().flex_col().gap(px(3.)).px(px(4.)).pt(px(4.)).children(folded));
                }
            }
        }
        let key_hint =
            preview.as_ref().is_some_and(|p| !p.igdb_configured && !p.offline).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .px(px(12.))
                    .py(px(9.))
                    .rounded(px(theme::R_INPUT))
                    .bg(theme::accent_tint())
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_primary()).child(tr!(
                        "IGDB is not configured. Add a Twitch Client ID and Secret in Collection settings for the richest data."
                    )))
                    .child(
                        div()
                            .id("md-igdb-key")
                            .text_size(px(12.))
                            .text_color(theme::accent())
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::text_bright()))
                            .child(tr!("Open Settings \u{2192} Metadata"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hub.update(cx, |h, cx| h.request_settings(cx));
                                cx.emit(DialogEvent::Close);
                            })),
                    )
            });
        let review = preview.as_ref().is_some_and(|p| p.review.is_some() && !p.candidates.is_empty()).then(|| {
            line(tr!("The sources are not sure which game this is: nothing is picked for you. Pick the values, or change a match."))
                .px(px(4.))
                .text_color(theme::caution_text())
        });
        let error = self.error.clone().map(|e| line(e).px(px(4.)).text_color(theme::danger_text()));
        let again = icon_button_sm(
            "md-again",
            ButtonKind::Glass,
            Icon::Refresh,
            if self.busy == Some(Busy::Preview) { tr!("Searching\u{2026}") } else { tr!("Search every source again") },
            self.busy.is_none(),
            cx.listener(|this, _, _, cx| this.run_preview(cx)),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .min_h_0()
            .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(tr!("Found as")))
            .children(key_hint)
            .children(review)
            .child(list)
            .children(error)
            .child(again.flex_none())
    }
}

/// A spinner and a line, while something is asked.
pub(super) fn busy_line(text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(12.))
        .rounded(px(theme::R_TILE))
        .bg(theme::card_fill())
        .border_1()
        .border_color(theme::card_border())
        .child(icon(Icon::FaSpinner, px(13.), theme::accent()))
        .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(text))
}
