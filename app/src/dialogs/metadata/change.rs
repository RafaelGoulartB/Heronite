//! "Change": which game of one source this is. A search of that source only
//! (by the game's title to start, Enter searches again) and its hits;
//! picking one pins it and asks the sources again with that game;
//! "Automatic" lets the source find the game by itself again.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::service::SourceSearch;
use heronite_metadata::{SourceAnswer, SourceHit};

use super::found_as::busy_line;
use super::MetadataDialog;
use crate::dialogs::metadata_update::words;
use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::{new_input, search_box};
use crate::ui::icon;

pub(super) struct Change {
    pub(super) source: String,
    input: Entity<InputState>,
    searching: bool,
    hits: Vec<SourceHit>,
    note: Option<String>,
    task: Option<Task<()>>,
    _sub: Subscription,
}

impl MetadataDialog {
    /// Opens "Change" for `source` and searches it for the game's title.
    pub(super) fn open_change(&mut self, source: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = new_input(window, cx, &self.title, tr!("Search this source"));
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.search_change(cx);
            }
        });
        self.change =
            Some(Change { source: source.to_owned(), input, searching: false, hits: Vec::new(), note: None, task: None, _sub: sub });
        self.search_change(cx);
    }

    fn search_change(&mut self, cx: &mut Context<Self>) {
        let Some(change) = &mut self.change else { return };
        let typed = change.input.read(cx).value().trim().to_string();
        let query = if typed.is_empty() { self.title.clone() } else { typed };
        let source = change.source.clone();
        change.searching = true;
        change.note = None;
        if let Some(fake) = &self.fake {
            let found = fake.search(&source, &query);
            self.change_found(&query, found, cx);
            return;
        }
        let service = self.service.clone();
        let task = cx.spawn(async move |this, cx| {
            let q = query.clone();
            let found = cx.background_spawn(async move { service.search_sources(&q, Some(&[source])) }).await;
            this.update(cx, |this, cx| this.change_found(&query, found, cx)).ok();
        });
        if let Some(change) = &mut self.change {
            change.task = Some(task);
        }
        cx.notify();
    }

    fn change_found(&mut self, query: &str, found: SourceSearch, cx: &mut Context<Self>) {
        let Some(change) = &mut self.change else { return };
        change.searching = false;
        change.hits = found.choices.into_iter().flat_map(|c| c.hits).filter(|h| h.source == change.source && h.id.is_some()).collect();
        change.note = if !found.unreachable.is_empty() {
            Some(tr!("Could not reach this source. Check your connection and try again.").to_string())
        } else if !found.needs_key.is_empty() {
            Some(tr!("This source needs a key: add it in Settings \u{2192} Metadata.").to_string())
        } else if found.asked.is_empty() {
            Some(tr!("This source is off.").to_string())
        } else if change.hits.is_empty() {
            Some(tr!("Nothing found for \u{201c}{query}\u{201d}.", query))
        } else {
            None
        };
        cx.notify();
    }

    /// The id of the game `source` matched in the preview.
    fn matched_id(&self, source: &str) -> Option<String> {
        let p = self.preview.as_ref()?;
        p.sources.iter().find(|s| s.source == source).and_then(|s| match &s.answer {
            SourceAnswer::Found { hit, .. } => hit.id.clone(),
            _ => None,
        })
    }

    fn hit_row(&self, i: usize, hit: &SourceHit, current: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let line = [hit.year.map(|y| y.to_string()), hit.alt_title.clone(), hit.platform.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        let (source, id) = (hit.source.clone(), hit.id.clone());
        div()
            .id(("md-hit", i))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(10.))
            .cursor_pointer()
            .when(current, |d| d.bg(theme::accent_tint()))
            .hover(|s| s.bg(theme::ink_a(0.06)))
            .on_click(cx.listener(move |this, _, _, cx| this.pin(&source, id.clone(), cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div().truncate().text_size(px(13.)).text_color(theme::text_primary()).child(hit.title.clone().unwrap_or_default()),
                    )
                    .when(!line.is_empty(), |d| d.child(div().truncate().text_size(px(11.5)).text_color(theme::text_dim()).child(line))),
            )
            .when(current, |d| d.child(icon(Icon::Check, px(14.), theme::accent())))
    }

    pub(super) fn render_change(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(change) = &self.change else { return div() };
        let source = change.source.clone();
        let name = words::source_name(&self.service, &source);
        let (input, searching, hits, note) = (change.input.clone(), change.searching, change.hits.clone(), change.note.clone());
        let current = self.matched_id(&source);
        let pinned = self.pinned(&source);
        let back = div()
            .id("md-change-back")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(26.))
            .rounded_full()
            .bg(theme::ink_a(0.08))
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.14)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.change = None;
                cx.notify();
            }))
            .child(icon(Icon::ChevronLeft, px(16.), theme::text_primary()));
        let mut list = div().id("md-hits").flex().flex_col().gap(px(2.)).flex_1().min_h_0().overflow_y_scroll();
        if pinned {
            let auto = source.clone();
            list = list.child(
                div()
                    .id("md-hit-auto")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::ink_a(0.06)))
                    .on_click(cx.listener(move |this, _, _, cx| this.pin(&auto, None, cx)))
                    .child(icon(Icon::AutoAwesome, px(14.), theme::accent()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(tr!("Automatic")))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(theme::text_dim())
                                    .child(tr!("Let {source} find the game by itself", source = name.as_str())),
                            ),
                    ),
            );
        }
        if searching {
            list = list.child(busy_line(tr!("Searching\u{2026}")));
        } else {
            for (i, hit) in hits.iter().enumerate() {
                let on = current.is_some() && hit.id == current;
                list = list.child(self.hit_row(i, hit, on, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .min_h_0()
            .child(
                div().flex().items_center().gap(px(8.)).child(back).child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(theme::text_primary())
                        .child(tr!("Which game of {source} is it?", source = name.as_str())),
                ),
            )
            .child(search_box(&input).flex_none().w_full())
            .children(note.map(|n| div().px(px(4.)).text_size(px(12.)).text_color(theme::caution_text()).child(n)))
            .child(list)
    }
}
