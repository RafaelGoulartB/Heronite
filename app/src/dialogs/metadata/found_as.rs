//! "Found as": the game most sources found (its title and year, once), then
//! a line per details source with only what differs from it: how it found
//! that game, or the name it found, and the whole match in its tooltip. A
//! match the merge left out (another game than the others found) is amber;
//! a pinned one has a pin; "Change" searches the source for another game. A source
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

const ROW_H: f32 = 30.;

/// The game most of the sources the merge uses found: the title most of
/// them have (letters and digits compared), with its year.
pub(super) struct Agreed {
    title: String,
    year: Option<i32>,
}

/// A title as compared: its letters and digits, lower case.
fn title_key(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

impl Agreed {
    fn of(sources: &[SourceState]) -> Option<Self> {
        let hits: Vec<&heronite_metadata::SourceHit> = sources
            .iter()
            .filter_map(|s| match &s.answer {
                SourceAnswer::Found { hit, used: true, .. } => Some(hit),
                _ => None,
            })
            .collect();
        let count = |key: &str| hits.iter().filter(|h| h.title.as_deref().map(title_key).as_deref() == Some(key)).count();
        // The first source in the order wins a tie.
        let best = hits.iter().rev().filter(|h| h.title.is_some()).max_by_key(|h| count(&title_key(h.title.as_deref().unwrap_or(""))))?;
        let key = title_key(best.title.as_deref()?);
        let year = hits.iter().filter(|h| h.title.as_deref().map(title_key).as_deref() == Some(key.as_str())).find_map(|h| h.year);
        Some(Self { title: best.title.clone()?, year })
    }

    /// Whether `hit` is that game: the same title, and the same year when both have one.
    fn is(&self, hit: &heronite_metadata::SourceHit) -> bool {
        hit.title.as_deref().is_some_and(|t| title_key(t) == title_key(&self.title))
            && (hit.year.is_none() || self.year.is_none() || hit.year == self.year)
    }
}

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

    /// A source's line: its mark and name, then only what is not the game
    /// most sources found: how it found that game ("by its id"), else the
    /// name it found ("as “…”"), left out as another game (amber), or not
    /// found. The whole of it (title, year, platform, how) in the tooltip.
    fn source_row(&self, state: &SourceState, agreed: Option<&Agreed>, cx: &mut Context<Self>) -> Stateful<Div> {
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
        let mut sure = false;
        let (how, full, amber, action): (String, String, bool, Option<&'static str>) = match &state.answer {
            SourceAnswer::Found { hit, used, .. } => {
                let platform = hit.platform.clone().filter(|_| by_platform);
                let by = hit.matched_by.and_then(words::matched_by);
                let full = [hit.title.clone(), hit.year.map(|y| y.to_string()), platform.clone(), by.map(str::to_owned)]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" \u{b7} ");
                let left_out = !used;
                let how = if left_out && !review {
                    tr!("{game} (left out: another game)", game = hit.title.clone().unwrap_or_else(|| full.clone()))
                } else if agreed.is_some_and(|a| a.is(hit)) {
                    sure = true;
                    [by.map(str::to_owned), platform].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ")
                } else {
                    let found = hit.title.as_deref().map(|t| tr!("as \u{201c}{title}\u{201d}", title = t));
                    [found, hit.year.map(|y| y.to_string()), by.map(str::to_owned)]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" \u{b7} ")
                };
                (how, full, left_out, searchable.then_some(tr!("Change")))
            }
            SourceAnswer::NotFound => (tr!("Not found").into(), String::new(), false, searchable.then_some(tr!("Search"))),
            SourceAnswer::Unreachable => (tr!("Could not be reached").into(), String::new(), false, None),
            // A fallback source the others made unnecessary: it can still be searched.
            SourceAnswer::NotAsked { why: NotAsked::Fallback } => {
                (tr!("Not asked: the others found it").into(), String::new(), false, searchable.then_some(tr!("Search")))
            }
            // Folded into the line under the rows.
            SourceAnswer::NotAsked { .. } => (String::new(), String::new(), false, None),
        };
        let pinned = self.pinned(&id);
        let found = matches!(state.answer, SourceAnswer::Found { .. });
        let warn: Hsla = theme::warning().into();
        div()
            .id(SharedString::from(format!("md-found-{id}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(ROW_H))
            .px(px(8.))
            .rounded(px(theme::R_INPUT))
            .when(amber, |d| d.bg(Hsla { a: 0.08, ..warn }))
            .when(!amber, |d| d.hover(|s| s.bg(theme::ink_a(0.04))))
            .when(!full.is_empty(), |d| d.tooltip(crate::ui::tooltip(full)))
            .child(labels::mark(&id, &name, 18.))
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(if found { theme::text_primary() } else { theme::text_muted() })
                    .child(name),
            )
            .when(pinned, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("md-pin-{id}")))
                        .flex_none()
                        .tooltip(crate::ui::tooltip(tr!("Pinned: every update uses this game")))
                        .child(icon(Icon::PushPin, px(11.), theme::accent())),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(3.))
                    .when(sure, |d| d.child(icon(Icon::Check, px(11.), theme::success())))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(if amber {
                                theme::caution_text()
                            } else if sure {
                                theme::success()
                            } else {
                                theme::text_dim()
                            })
                            .child(how),
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
                let agreed = Agreed::of(&p.sources);
                if let Some(a) = &agreed {
                    list = list.child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.))
                            .px(px(4.))
                            .min_w_0()
                            .child(div().min_w_0().truncate().text_size(px(13.)).text_color(theme::text_bright()).child(a.title.clone()))
                            .children(
                                a.year.map(|y| div().flex_none().text_size(px(12.)).text_color(theme::text_muted()).child(y.to_string())),
                            ),
                    );
                }
                let mut rows = div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .p(px(4.))
                    .rounded(px(theme::R_TILE))
                    .bg(theme::card_fill())
                    .border_1()
                    .border_color(theme::card_border());
                for state in p.sources.iter().filter(row) {
                    rows = rows.child(self.source_row(state, agreed.as_ref(), cx));
                }
                list = list.child(rows);
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
