//! "Change match" of the How Long To Beat section: search HowLongToBeat and
//! pick the game the times are of, or say the game is not on HowLongToBeat.
//! The choice is kept (`local_library/hltb_overrides.json`) until "Match
//! automatically".

use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::hltb::format_hours;
use heronite_i18n::tr;
use heronite_metadata::hltb::{seconds_to_hours, SearchHit};
use heronite_metadata::MetadataService;

use super::metadata_wizard::hub;
use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{action_button, button, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{input_box, new_input};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Search,
    Save,
}

pub struct HltbMatchDialog {
    service: Arc<MetadataService>,
    /// The Collection title (the cache key).
    title: String,
    /// The HowLongToBeat game shown now.
    current: Option<u64>,
    /// A match the user picked earlier (or "not on HowLongToBeat").
    has_choice: bool,
    search: Entity<InputState>,
    hits: Vec<SearchHit>,
    searched: bool,
    busy: Option<Busy>,
    error: String,
}

impl EventEmitter<DialogEvent> for HltbMatchDialog {}

impl HltbMatchDialog {
    pub fn new(title: String, current: Option<u64>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let search = new_input(window, cx, &title, tr!("Search HowLongToBeat"));
        cx.subscribe_in(&search, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.run_search(cx);
            }
        })
        .detach();
        let has_choice = service.how_long_to_beat_choice(&title).is_some();
        let mut this =
            Self { service, title, current, has_choice, search, hits: Vec::new(), searched: false, busy: None, error: String::new() };
        this.run_search(cx);
        this
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let typed = self.search.read(cx).value().trim().to_string();
        let query = if typed.is_empty() { self.title.clone() } else { typed };
        self.busy = Some(Busy::Search);
        self.error.clear();
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.search_how_long_to_beat(&query) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                this.searched = true;
                match res {
                    Ok(hits) => this.hits = hits,
                    Err(err) => {
                        log::info!("HowLongToBeat search: {err:#}");
                        this.hits.clear();
                        this.error = tr!("Couldn't reach HowLongToBeat. Try again later.").into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Saves `hit` (`None`: not on HowLongToBeat) and closes.
    fn choose(&mut self, hit: Option<SearchHit>, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Save);
        let (service, title) = (self.service.clone(), self.title.clone());
        self.save(cx, move || service.set_how_long_to_beat_match(&title, hit.as_ref()).map(|_| ()));
    }

    /// Drops the picked match; the next look searches by name again.
    fn automatic(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(Busy::Save);
        let (service, title) = (self.service.clone(), self.title.clone());
        self.save(cx, move || {
            service.clear_how_long_to_beat_match(&title)?;
            service.how_long_to_beat(&title, None).map(|_| ())
        });
    }

    fn save(&mut self, cx: &mut Context<Self>, job: impl FnOnce() -> anyhow::Result<()> + Send + 'static) {
        self.error.clear();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { job() }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok(()) => cx.emit(DialogEvent::ChangedAndClose),
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn render_hit(&self, i: usize, hit: &SearchHit, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.current == Some(hit.game_id);
        let mut name = hit.game_name.clone();
        if let Some(year) = hit.release_world {
            name = format!("{name} ({year})");
        }
        let kind = (!hit.is_game()).then(|| hit.game_type.clone().unwrap_or_default().to_uppercase());
        let times = [(tr!("Main Story"), hit.comp_main), (tr!("Main + Extras"), hit.comp_plus), (tr!("Completionist"), hit.comp_100)]
            .map(|(label, secs)| format!("{label} {}", format_hours(seconds_to_hours(secs))))
            .join("  \u{b7}  ");
        let pick = hit.clone();
        div()
            .id(("hltb-hit", i))
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(10.))
            .border_1()
            .border_color(if selected { theme::accent().into() } else { theme::ink_a(0.1) })
            .bg(if selected { theme::accent_alpha(0.16).into() } else { theme::recess_a(0.2) })
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.06)))
            .on_click(cx.listener(move |this, _, _, cx| this.choose(Some(pick.clone()), cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(name))
                            .children(kind.map(|k| {
                                div()
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .bg(theme::ink_a(0.08))
                                    .text_size(px(10.))
                                    .text_color(theme::ink_a(0.7))
                                    .child(k)
                            })),
                    )
                    .child(div().text_size(px(12.)).text_color(theme::ink_a(0.6)).child(times)),
            )
            .children(selected.then(|| div().text_size(px(12.)).text_color(theme::accent()).child(tr!("Shown now"))))
    }
}

impl Render for HltbMatchDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy.is_some();
        let search_row =
            div().flex().items_center().gap(px(8.)).child(div().flex_1().child(input_box(&self.search, false))).child(action_button(
                "hltb-search",
                ButtonKind::Glass,
                if self.busy == Some(Busy::Search) { tr!("Searching…") } else { tr!("Search") },
                !busy,
                cx.listener(|this, _, _, cx| this.run_search(cx)),
            ));
        let mut list = div().flex().flex_col().gap(px(6.));
        for (i, hit) in self.hits.iter().enumerate() {
            list = list.child(self.render_hit(i, hit, cx));
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(help_text(tr!("Pick the game the times are of. Your pick is kept when the times are refreshed.")))
            .child(search_row);
        if self.searched && self.hits.is_empty() && self.error.is_empty() {
            body = body.child(div().text_size(px(13.)).text_color(theme::ink_a(0.6)).child(tr!("No games found. Try another name.")));
        }
        body = body.child(list);
        if !self.error.is_empty() {
            body = body.child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(theme::R_INPUT))
                    .bg(theme::danger_fill())
                    .text_size(px(13.))
                    .text_color(theme::danger_text())
                    .child(self.error.clone()),
            );
        }
        let footer = div()
            .flex()
            .gap(px(16.))
            .child(action_button(
                "hltb-none",
                ButtonKind::Glass,
                tr!("Not on HowLongToBeat"),
                !busy,
                cx.listener(|this, _, _, cx| this.choose(None, cx)),
            ))
            .when(self.has_choice, |d| {
                d.child(action_button(
                    "hltb-auto",
                    ButtonKind::Glass,
                    tr!("Match automatically"),
                    !busy,
                    cx.listener(|this, _, _, cx| this.automatic(cx)),
                ))
            })
            .child(button("hltb-close", ButtonKind::Glass, tr!("Close")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))));
        let entity = cx.entity().downgrade();
        let (vw, vh) = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        dialog(
            DialogProps {
                id: "hltb-match".into(),
                title: tr!("HowLongToBeat match").into(),
                width: 640f32.min(vw * 0.9) + 64.,
                height: Some(560f32.min(vh * 0.75)),
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
