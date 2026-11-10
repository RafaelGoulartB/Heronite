//! `CollectionWebImageDialog`: search images (SteamGridDB with an API key,
//! then the web: DuckDuckGo, Bing, Google) and use one as the game's cover or
//! background.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_data::models::CollectionGameArt;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::{ArtImages, ArtKind, MetadataService, WebImage};

use super::art_images::{picture, DialogImages};
use super::metadata_wizard::hub;
use super::DialogCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, caption, checkbox, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{new_input, search_box};
use crate::ui::icon;

const PAGE_SIZE: usize = 20;
/// SteamGridDB's images listed before its "Load more": a row, so the web's
/// results show under them without scrolling.
const STEAMGRID_PAGE: usize = 6;

/// Where a result comes from: its section of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    SteamGrid,
    Web,
}

impl Source {
    fn id(self) -> &'static str {
        match self {
            Source::SteamGrid => "sgdb",
            Source::Web => "web",
        }
    }
}

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
    results: ArtImages,
    /// Images listed in each section before its "Load more".
    visible_steamgrid: usize,
    visible_web: usize,
    selected: Option<(Source, usize)>,
    loading: bool,
    saving: bool,
    error: String,
    request: u64,
}

impl EventEmitter<ArtEvent> for WebImageDialog {}

pub fn default_query(title: &str, kind: ArtKind) -> String {
    let name = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let suffix = if kind == ArtKind::Hero { "wallpaper" } else { "cover" };
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
            results: ArtImages::default(),
            visible_steamgrid: STEAMGRID_PAGE,
            visible_web: PAGE_SIZE,
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
        self.results = ArtImages::default();
        self.visible_steamgrid = STEAMGRID_PAGE;
        self.visible_web = PAGE_SIZE;
        let (service, kind, transparent) = (self.service.clone(), self.kind, self.transparent);
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.search_art_images(&term, kind, transparent) }).await;
            this.update(cx, |this, cx| {
                if this.request != id {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(list) => {
                        if list.steamgrid.is_empty() && list.web.is_empty() {
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

    fn images(&self, from: Source) -> &[WebImage] {
        match from {
            Source::SteamGrid => &self.results.steamgrid,
            Source::Web => &self.results.web,
        }
    }

    fn confirm(&mut self, pick: Option<(Source, usize)>, cx: &mut Context<Self>) {
        let Some(image) = pick.or(self.selected).and_then(|(from, i)| self.images(from).get(i)).cloned() else { return };
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

impl WebImageDialog {
    /// A source's results: its title (when `titled`), its images, and its
    /// "Load more". `None` when it found nothing.
    fn section(&mut self, from: Source, titled: bool, (tw, th): (f32, f32), cx: &mut Context<Self>) -> Option<Div> {
        let total = self.images(from).len();
        if total == 0 {
            return None;
        }
        let (visible, title) = match from {
            Source::SteamGrid => (self.visible_steamgrid, "SteamGridDB"),
            Source::Web => (self.visible_web, tr!("From the web")),
        };
        let mut tiles = div().flex().flex_wrap().gap(px(9.6));
        for i in 0..total.min(visible) {
            tiles = tiles.child(self.tile(from, i, tw, th, cx));
        }
        let more = (visible < total).then(|| {
            div().flex().justify_center().pt(px(10.)).child(
                button(SharedString::from(format!("wi-more-{}", from.id())), ButtonKind::Glass, tr!("Load more")).min_w(px(192.)).on_click(
                    cx.listener(move |this, _, _, cx| {
                        match from {
                            Source::SteamGrid => this.visible_steamgrid += PAGE_SIZE,
                            Source::Web => this.visible_web += PAGE_SIZE,
                        }
                        cx.notify();
                    }),
                ),
            )
        });
        Some(div().flex().flex_col().pb(px(8.)).when(titled, |d| d.child(caption(title).mt(px(4.)))).child(tiles).children(more))
    }

    fn tile(&mut self, from: Source, i: usize, tw: f32, th: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let image = self.images(from)[i].clone();
        let selected = self.selected == Some((from, i));
        let chain = vec![ImageRef::Url(image.thumb_url.clone())];
        let src = self.images.update(cx, |im, _| im.get(&format!("wi:{}", image.thumb_url), chain, tw, th));
        let size = (image.width > 0 && image.height > 0).then(|| format!("{}\u{d7}{}px", image.width, image.height));
        div()
            .id(SharedString::from(format!("wi-item-{}-{i}", from.id())))
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
                this.selected = Some((from, i));
                if matches!(ev, ClickEvent::Mouse(m) if m.up.click_count >= 2) {
                    this.confirm(Some((from, i)), cx);
                }
                cx.notify();
            }))
            .child(picture(src).w(px(tw)).h(px(th)))
            .children(size.map(|s| div().text_size(px(12.)).text_color(theme::text_dim()).child(s)))
    }
}

impl Render for WebImageDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        let kind_label = if self.kind == ArtKind::Hero { tr!("Background") } else { tr!("Cover") };
        // Thumbnails in the shape of what is picked.
        let (tw, th) = match self.kind {
            ArtKind::Cover => (120., 180.),
            ArtKind::CoverSquare => (150., 150.),
            ArtKind::CoverWide | ArtKind::Hero => (240., 90.),
        };

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

        let mut grid = div().id("wi-grid").flex().flex_col().min_h(px(288.)).max_h(px(420.)).overflow_y_scroll().p(px(3.));
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
                        Animation::new(Duration::from_secs(1)).repeat().with_max_fps(30.),
                        |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
                    ))
                    .child(tr!("Searching…")),
            );
        } else {
            // Without SteamGridDB's results the web's need no title.
            let titled = !self.results.steamgrid.is_empty();
            for from in [Source::SteamGrid, Source::Web] {
                grid = grid.children(self.section(from, titled, (tw, th), cx));
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
        let vw = f32::from(crate::ui::content_size(window).width);
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
