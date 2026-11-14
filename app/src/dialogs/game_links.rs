//! "Links" of one game, from the title of the focus panel's Links: the
//! links found by themselves (its store pages, the sites its Steam app id
//! gives) and the ones of its metadata (`websites`), which can be removed;
//! a field adds one (a web address, `https://` when it has none). Each
//! change is saved at once; the Collection reloads (`DialogEvent::Changed`).

use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::game_links::{classify, is_shown, GameLink};
use heronite_i18n::tr;
use heronite_metadata::{DetailsPatch, MetadataService};

use super::DialogEvent;
use crate::collection::links::{link_icon, link_name};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, help_text, icon_button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{input_box, new_input, on_commit};
use crate::ui::{icon, tooltip};

pub struct GameLinksDialog {
    title: String,
    runner: String,
    app_name: String,
    /// The game's own links (its metadata's `websites`), in order.
    websites: Vec<String>,
    /// Links found by themselves, not removable here.
    found: Vec<GameLink>,
    input: Entity<InputState>,
    error: Option<SharedString>,
    saving: bool,
    service: Arc<MetadataService>,
}

impl EventEmitter<DialogEvent> for GameLinksDialog {}

impl GameLinksDialog {
    /// `found`: every link the panel shows for the game; those among
    /// `websites` are listed as the game's own.
    pub fn new(
        game: &heronite_data::CollectionGame,
        found: Vec<GameLink>,
        service: Arc<MetadataService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let websites: Vec<String> = game.metadata.as_ref().map(|m| m.websites.clone()).unwrap_or_default();
        let found = found.into_iter().filter(|l| !websites.iter().any(|w| w.trim() == l.url)).collect();
        let input = new_input(window, cx, "", "https://");
        on_commit(&input, window, cx, |this: &mut Self, value, window, cx| {
            // Blur commits too: only Enter (or Add) adds.
            if !value.is_empty() && this.input.read(cx).focus_handle(cx).is_focused(window) {
                this.add(window, cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        Self {
            title: game.title.clone(),
            runner: game.runner.clone(),
            app_name: game.app_name.clone(),
            websites,
            found,
            input,
            error: None,
            saving: false,
            service,
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let typed = self.input.read(cx).value().to_string();
        let Some(link) = classify(&typed) else {
            self.error = Some(tr!("Type a web address, like https://example.com").into());
            cx.notify();
            return;
        };
        let known = |u: &str| u.trim_end_matches('/').eq_ignore_ascii_case(link.url.trim_end_matches('/'));
        if self.websites.iter().any(|w| known(w)) || self.found.iter().any(|l| known(&l.url)) {
            self.error = Some(tr!("This link is already there.").into());
            cx.notify();
            return;
        }
        self.input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        let mut next = self.websites.clone();
        next.push(link.url);
        self.save(next, cx);
    }

    fn remove(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let mut next = self.websites.clone();
        if i < next.len() {
            next.remove(i);
            self.save(next, cx);
        }
    }

    /// Writes `websites` as the game's; the list on screen follows once saved.
    fn save(&mut self, websites: Vec<String>, cx: &mut Context<Self>) {
        self.error = None;
        self.saving = true;
        let patch = DetailsPatch {
            app_name: self.app_name.clone(),
            runner: self.runner.clone(),
            websites: Some(websites.clone()),
            ..Default::default()
        };
        let service = self.service.clone();
        let asked = websites.len();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.update_game_details(&patch) }).await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(r) => {
                        if r.metadata.websites.len() < asked {
                            this.error = Some(tr!("A game keeps at most {n} links.", n = r.metadata.websites.len()).into());
                        }
                        this.websites = r.metadata.websites;
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(e) => {
                        log::warn!("links of {}: {e:#}", this.app_name);
                        this.error = Some(tr!("Could not save the links: {err}", err = format!("{e:#}")).into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn row(&self, id: SharedString, link: &GameLink, removable: Option<usize>, cx: &mut Context<Self>) -> Div {
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(7.))
            .px(px(10.))
            .rounded(px(10.))
            .hover(|s| s.bg(theme::ink_a(0.05)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded(px(9.))
                    .bg(theme::focus_card_fill(true))
                    .border_1()
                    .border_color(theme::ink_a(0.1))
                    .child(icon(link_icon(link.kind), px(17.), theme::text_primary())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(link_name(link)))
                    .child(div().text_size(px(12.)).text_color(theme::text_muted()).truncate().child(link.url.clone())),
            );
        row = match removable {
            Some(i) => row
                .children((!is_shown(link)).then(|| {
                    div()
                        .flex_none()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded_full()
                        .bg(theme::ink_a(0.06))
                        .text_size(px(11.5))
                        .text_color(theme::text_muted())
                        .child(tr!("Not shown in Links"))
                }))
                .child(
                    icon_button(id, Icon::DeleteOutline, 18., !self.saving)
                        .tooltip(tooltip(tr!("Remove")))
                        .on_click(cx.listener(move |this, _, _, cx| this.remove(i, cx))),
                ),
            None => row.child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_full()
                    .bg(theme::ink_a(0.06))
                    .text_size(px(11.5))
                    .text_color(theme::text_muted())
                    .child(tr!("Found by itself")),
            ),
        };
        row
    }
}

impl Render for GameLinksDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col();
        let own: Vec<(usize, GameLink)> = self.websites.iter().enumerate().filter_map(|(i, w)| classify(w).map(|l| (i, l))).collect();
        for (i, link) in &own {
            list = list.child(self.row(SharedString::from(format!("link-rm-{i}")), link, Some(*i), cx));
        }
        for (i, link) in self.found.clone().iter().enumerate() {
            list = list.child(self.row(SharedString::from(format!("link-found-{i}")), link, None, cx));
        }
        if own.is_empty() && self.found.is_empty() {
            list = list.child(help_text(tr!("No links yet.")));
        }
        let add =
            div().flex().items_center().gap(px(8.)).child(div().flex_1().min_w_0().child(input_box(&self.input, self.saving))).child(
                button("link-add", ButtonKind::Primary, tr!("Add")).on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(help_text(tr!(
                "Store pages, wikis, guides, the official site: each shows as its icon in Links. The store pages and the sites of its Steam app id are found by themselves."
            )))
            .child(add)
            .children(self.error.clone().map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e)))
            .child(list);
        let footer = div()
            .flex()
            .child(button("links-done", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))));
        let entity = cx.entity().downgrade();
        let vh = f32::from(crate::ui::content_size(window).height);
        dialog(
            DialogProps {
                id: "game-links".into(),
                title: tr!("Links of {title}", title = self.title.as_str()).into(),
                width: 560.,
                height: Some(620f32.min(vh * 0.8)),
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}
