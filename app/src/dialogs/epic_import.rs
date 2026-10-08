//! "Import game": pick one of the owned, not installed Epic games; the caller
//! then opens the install dialog in import mode for it.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::json_store;
use heronite_data::HeroicPaths;
use serde_json::Value;

use super::DialogEvent;
use crate::theme;
use crate::ui::controls::{button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{meta_text, new_input};

pub struct EpicImportPicker {
    games: Vec<Value>,
    loaded: bool,
    filter: Entity<InputState>,
    /// Chosen game (read by the host after `DialogEvent::Close`).
    pub picked: Option<Value>,
}

impl EventEmitter<DialogEvent> for EpicImportPicker {}

fn title_of(g: &Value) -> String {
    g.get("title").and_then(Value::as_str).unwrap_or_default().to_string()
}

impl EpicImportPicker {
    pub fn new(heroic: HeroicPaths, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = new_input(window, cx, "", "Search your Epic games");
        cx.subscribe_in(&filter, window, |_, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let games = cx
                .background_spawn(async move {
                    let doc = json_store::read_value(&heroic.legendary_library()).unwrap_or(Value::Null);
                    let mut list: Vec<Value> = doc
                        .get("library")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|g| {
                            !g.get("is_installed").and_then(Value::as_bool).unwrap_or(false)
                                && !g.pointer("/install/is_dlc").and_then(Value::as_bool).unwrap_or(false)
                        })
                        .collect();
                    list.sort_by_key(|g| title_of(g).to_lowercase());
                    list
                })
                .await;
            this.update(cx, |this, cx| {
                this.games = games;
                this.loaded = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
        Self { games: Vec::new(), loaded: false, filter, picked: None }
    }
}

impl Render for EpicImportPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let needle = self.filter.read(cx).value().trim().to_lowercase();
        let matches: Vec<(usize, String)> = self
            .games
            .iter()
            .enumerate()
            .map(|(i, g)| (i, title_of(g)))
            .filter(|(_, t)| needle.is_empty() || t.to_lowercase().contains(&needle))
            .collect();
        let total = matches.len();
        let mut list = div().flex().flex_col().gap(px(2.)).max_h(px(320.)).overflow_hidden();
        for (i, title) in matches.into_iter().take(10) {
            list = list.child(
                div()
                    .id(SharedString::from(format!("epic-pick-{i}")))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(15.))
                    .text_color(rgb(0xcaf3fd))
                    .hover(|s| s.bg(theme::white_a(0.1)))
                    .child(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.picked = this.games.get(i).cloned();
                        cx.emit(DialogEvent::Close);
                    })),
            );
        }
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(meta_text("Choose the game you already have on disk, then pick its folder."))
            .child(
                div()
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .bg(theme::navbar_background())
                    .text_size(px(16.))
                    .child(div().w_full().child(Input::new(&self.filter).appearance(false))),
            )
            .child(if !self.loaded {
                meta_text("Loading...")
            } else if self.games.is_empty() {
                meta_text("No games to import. Refresh your library first.")
            } else {
                div().child(list)
            })
            .child(if total > 10 { meta_text(format!("{} more - type to narrow the list", total - 10)) } else { div() });
        let footer = button("epic-pick-close", ButtonKind::Outline, "Close")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)))
            .into_any_element();
        let entity = cx.entity().downgrade();
        dialog(
            DialogProps { id: "epic-import-pick".into(), title: "Import Game".into(), width: 520., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
