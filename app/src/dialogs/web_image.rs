//! `CollectionWebImageDialog`: search images on the web (DuckDuckGo, Bing,
//! Google) and use one as the game's cover or background.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::models::CollectionGameArt;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::{ArtKind, MetadataService, WebImage};

use super::art_images::{picture, DialogImages};
use super::metadata_wizard::hub;
use super::DialogCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, checkbox, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{new_input, search_box};
use crate::ui::icon;

const PAGE_SIZE: usize = 20;

/// Events of the nested art pickers (web / screenshot) toward Game settings.
pub enum ArtEvent {
    Changed(CollectionGameArt),
    Close,
}

pub struct WebImageDialog {
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    app_name: String,
    runner: String,
    kind: ArtKind,
    query: Entity<InputState>,
    transparent: bool,
    results: Vec<WebImage>,
    visible: usize,
    selected: Option<usize>,
    loading: bool,
    saving: bool,
    error: String,
    request: u64,
}

impl EventEmitter<ArtEvent> for WebImageDialog {}

pub fn default_query(title: &str, kind: ArtKind) -> String {
    let name = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let suffix = if kind == ArtKind::Cover { "cover" } else { "wallpaper" };
    if name.is_empty() {
        suffix.to_string()
    } else {
        format!("\"{name}\" {suffix}")
    }
}

impl WebImageDialog {
    pub fn new(ctx: DialogCtx, game: Game, title: String, kind: ArtKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(&ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let query = new_input(window, cx, &default_query(&title, kind), "");
        cx.subscribe_in(&query, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        })
        .detach();
        let runner = if game.runner == "zoom" { "sideload".to_string() } else { game.runner.clone() };
        let mut this = Self {
            service,
            images,
            app_name: game.app_name.clone(),
            runner,
            kind,
            query,
            transparent: false,
            results: Vec::new(),
            visible: PAGE_SIZE,
            selected: None,
            loading: false,
            saving: false,
            error: String::new(),
            request: 0,
        };
        this.search(cx);
        this
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let term = self.query.read(cx).value().split_whitespace().collect::<Vec<_>>().join(" ");
        if term.is_empty() {
            return;
        }
        self.request += 1;
        let id = self.request;
        self.loading = true;
        self.error.clear();
        self.selected = None;
        self.results.clear();
        self.visible = PAGE_SIZE;
        let (service, transparent) = (self.service.clone(), self.transparent);
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.search_web_images(&term, transparent) }).await;
            this.update(cx, |this, cx| {
                if this.request != id {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(list) => {
                        if list.is_empty() {
                            this.error = tr!("No images found. Try another search.").into();
                        }
                        this.results = list;
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

    fn confirm(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(image) = index.or(self.selected).and_then(|i| self.results.get(i)).cloned() else { return };
        if self.saving {
            return;
        }
        self.saving = true;
        self.error.clear();
        let (service, runner, app, kind) = (self.service.clone(), self.runner.clone(), self.app_name.clone(), self.kind);
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move { service.set_art_from_url(&runner, &app, kind, &image.image_url, Some(&image.thumb_url)) })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(art) => {
                        cx.emit(ArtEvent::Changed(art));
                        cx.emit(ArtEvent::Close);
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
}

impl Render for WebImageDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let kind_label = if self.kind == ArtKind::Cover { tr!("Cover") } else { tr!("Background") };
        let (tw, th) = if self.kind == ArtKind::Cover { (120., 180.) } else { (240., 90.) };

        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(action_button(
                "wi-search",
                ButtonKind::Glass,
                tr!("Search"),
                !self.loading && !self.saving,
                cx.listener(|this, _, _, cx| this.search(cx)),
            ))
            .child(search_box(&self.query))
            .child(checkbox("wi-transparent", self.transparent, tr!("Transparent")).on_click(cx.listener(|this, _, _, cx| {
                if !this.loading && !this.saving {
                    this.transparent = !this.transparent;
                    this.search(cx);
                }
            })));

        let mut grid = div().id("wi-grid").flex().flex_wrap().gap(px(9.6)).min_h(px(288.)).max_h(px(380.)).overflow_y_scroll().p(px(3.));
        if self.loading {
            grid = grid.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(9.6))
                    .text_size(px(14.))
                    .text_color(theme::text_muted())
                    .child(icon(Icon::FaSpinner, px(16.), theme::text_muted()).with_animation(
                        "wi-spin",
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
                    ))
                    .child(tr!("Searching…")),
            );
        } else {
            let shown = self.results.len().min(self.visible);
            for i in 0..shown {
                let image = self.results[i].clone();
                let selected = self.selected == Some(i);
                let chain = vec![ImageRef::Url(image.thumb_url.clone())];
                let src = self.images.update(cx, |im, _| im.get(&format!("wi:{}", image.thumb_url), chain, tw, th));
                let size = (image.width > 0 && image.height > 0).then(|| format!("{}\u{d7}{}px", image.width, image.height));
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("wi-item-{i}")))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.8))
                        .p(px(6.4))
                        .rounded(px(12.))
                        .border_2()
                        .border_color(if selected { theme::accent().into() } else { gpui_kit::transparent_black() })
                        .bg(if selected { theme::ink_a(0.06) } else { theme::recess_a(0.28) })
                        .hover(|s| s.border_color(theme::accent()))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                            this.selected = Some(i);
                            if matches!(ev, ClickEvent::Mouse(m) if m.up.click_count >= 2) {
                                this.confirm(Some(i), cx);
                            }
                            cx.notify();
                        }))
                        .child(picture(src).w(px(tw)).h(px(th)))
                        .children(size.map(|s| div().text_size(px(12.)).text_color(theme::text_dim()).child(s))),
                );
            }
        }

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(help_text(tr!(
                "Pick an image from the web. Collection saves it like a file you selected yourself. {kind_label}",
                kind_label
            )))
            .child(toolbar)
            .child(grid);
        if !self.loading && self.visible < self.results.len() {
            body = body.child(div().flex().justify_center().child(
                button("wi-more", ButtonKind::Glass, tr!("Load more")).min_w(px(192.)).on_click(cx.listener(|this, _, _, cx| {
                    this.visible += PAGE_SIZE;
                    cx.notify();
                })),
            ));
        }
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
                "wi-cancel",
                ButtonKind::Ghost,
                tr!("Cancel"),
                !self.saving,
                cx.listener(|_, _, _, cx| cx.emit(ArtEvent::Close)),
            ))
            .child(action_button(
                "wi-select",
                ButtonKind::Primary,
                if self.saving { tr!("Downloading…") } else { tr!("Select") },
                self.selected.is_some() && !self.loading && !self.saving,
                cx.listener(|this, _, _, cx| this.confirm(None, cx)),
            ));
        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);
        dialog(
            DialogProps {
                id: "collection-web-image".into(),
                title: tr!("Search images").into(),
                width: 960f32.min(vw * 0.96),
                height: None,
            },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(ArtEvent::Close)).ok();
            },
        )
    }
}
