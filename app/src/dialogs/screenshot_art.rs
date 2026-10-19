//! `CollectionScreenshotArtDialog`: pick one of the game's screenshots as its
//! cover or background.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::Game;
use heronite_i18n::tr;
use heronite_media::{list_screenshots, GameQuery, Screenshot, ThumbnailCache};
use heronite_metadata::{ArtKind, MetadataService};

use super::metadata_wizard::hub;
use super::web_image::ArtEvent;
use super::DialogCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, help_text, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::icon;

const PAGE_SIZE: usize = 24;
const ART_EXT: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "avif"];

pub struct ScreenshotArtDialog {
    service: Arc<MetadataService>,
    cache: ThumbnailCache,
    app_name: String,
    runner: String,
    kind: ArtKind,
    items: Vec<Screenshot>,
    folder_configured: bool,
    visible: usize,
    selected: Option<usize>,
    loading: bool,
    saving: bool,
    error: String,
    thumbs: HashMap<PathBuf, Arc<RenderImage>>,
    requested: HashSet<PathBuf>,
}

impl EventEmitter<ArtEvent> for ScreenshotArtDialog {}

impl ScreenshotArtDialog {
    pub fn new(ctx: DialogCtx, game: Game, kind: ArtKind, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let settings = &ctx.lib.settings;
        let cache = ThumbnailCache::new(ctx.data_paths.screenshot_thumbnails_dir(), settings.screenshots.cache_limit_mb.get());
        let folder = settings.screenshots_folder().map(|p| p.to_path_buf());
        let query = GameQuery::from_game(&game);
        let runner = if game.runner == "zoom" { "sideload".to_string() } else { game.runner.clone() };
        let this = Self {
            service,
            cache,
            app_name: game.app_name.clone(),
            runner,
            kind,
            items: Vec::new(),
            folder_configured: folder.is_some(),
            visible: PAGE_SIZE,
            selected: None,
            loading: folder.is_some(),
            saving: false,
            error: String::new(),
            thumbs: HashMap::new(),
            requested: HashSet::new(),
        };
        if let Some(folder) = folder {
            cx.spawn(async move |this, cx| {
                let result = cx.background_spawn(async move { list_screenshots(&folder, &query) }).await;
                this.update(cx, |this, cx| {
                    this.loading = false;
                    let mut items: Vec<Screenshot> = result
                        .items
                        .into_iter()
                        .filter(|s| {
                            std::path::Path::new(&s.name)
                                .extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| ART_EXT.contains(&e.to_ascii_lowercase().as_str()))
                        })
                        .collect();
                    items.reverse();
                    this.items = items;
                    this.load_thumbs(cx);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        this
    }

    /// One preview at a time, in grid order: the cache blocks while it makes
    /// a preview, so a task per tile would hold every background thread.
    fn load_thumbs(&mut self, cx: &mut Context<Self>) {
        let shown = self.items.len().min(self.visible);
        let paths: Vec<_> = self.items[..shown].iter().map(|i| i.path.clone()).filter(|p| self.requested.insert(p.clone())).collect();
        if paths.is_empty() {
            return;
        }
        let cache = self.cache.clone();
        cx.spawn(async move |this, cx| {
            for path in paths {
                let (c, p) = (cache.clone(), path.clone());
                let Ok(decoded) = cx.background_spawn(async move { c.thumbnail(&p) }).await else { continue };
                let frame = image::Frame::new(decoded.into_image_buffer());
                let img = Arc::new(RenderImage::new(smallvec::smallvec![frame]));
                let alive = this
                    .update(cx, |this, cx| {
                        this.thumbs.insert(path, img);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn confirm(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(item) = index.or(self.selected).and_then(|i| self.items.get(i)).cloned() else { return };
        if self.saving {
            return;
        }
        self.saving = true;
        self.error.clear();
        let (service, runner, app, kind) = (self.service.clone(), self.runner.clone(), self.app_name.clone(), self.kind);
        cx.spawn(async move |this, cx| {
            let res =
                cx.background_spawn(async move { service.set_art_from_file(&runner, &app, kind, &item.path.to_string_lossy()) }).await;
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

impl Render for ScreenshotArtDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut grid = div().id("sa-grid").flex().flex_wrap().gap(px(9.6)).min_h(px(288.)).max_h(px(380.)).overflow_y_scroll().p(px(3.));
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
                        "sa-spin",
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
                    ))
                    .child(tr!("Loading…")),
            );
        } else if self.items.is_empty() {
            grid =
                grid.child(div().w_full().flex().items_center().justify_center().text_size(px(14.)).text_color(theme::text_muted()).child(
                    if self.folder_configured {
                        tr!("No screenshots found for this game.")
                    } else {
                        tr!("Set a screenshots folder in Collection settings first.")
                    },
                ));
        } else {
            let shown = self.items.len().min(self.visible);
            for i in 0..shown {
                let item = &self.items[i];
                let selected = self.selected == Some(i);
                let thumb = self.thumbs.get(&item.path).cloned();
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("sa-item-{i}")))
                        .relative()
                        .w(px(212.))
                        .h(px(119.25 + 4.))
                        .overflow_hidden()
                        .rounded(px(12.))
                        .border_2()
                        .border_color(if selected { theme::accent().into() } else { gpui_kit::transparent_black() })
                        .hover(|s| s.border_color(theme::accent()))
                        .bg(theme::black_a(0.28))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                            this.selected = Some(i);
                            if matches!(ev, ClickEvent::Mouse(m) if m.up.click_count >= 2) {
                                this.confirm(Some(i), cx);
                            }
                            cx.notify();
                        }))
                        .children(thumb.map(|t| img(ImageSource::Render(t)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover))),
                );
            }
        }

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(help_text(if self.kind == ArtKind::Hero {
                tr!("Pick a screenshot of this game to use as the background.")
            } else {
                tr!("Pick a screenshot of this game to use as the cover.")
            }))
            .child(grid);
        if !self.loading && self.visible < self.items.len() {
            body = body.child(div().flex().justify_center().child(
                button("sa-more", ButtonKind::Glass, tr!("Load more")).min_w(px(192.)).on_click(cx.listener(|this, _, _, cx| {
                    this.visible += PAGE_SIZE;
                    this.load_thumbs(cx);
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
                "sa-cancel",
                ButtonKind::Ghost,
                tr!("Cancel"),
                !self.saving,
                cx.listener(|_, _, _, cx| cx.emit(ArtEvent::Close)),
            ))
            .child(action_button(
                "sa-select",
                ButtonKind::Primary,
                if self.saving { tr!("Saving…") } else { tr!("Select") },
                self.selected.is_some() && !self.loading && !self.saving,
                cx.listener(|this, _, _, cx| this.confirm(None, cx)),
            ));
        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);
        dialog(
            DialogProps {
                id: "collection-screenshot-art".into(),
                title: tr!("Select screenshot").into(),
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
