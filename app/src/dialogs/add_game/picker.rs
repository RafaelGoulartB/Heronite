//! Image search used by the Add/Edit game dialog: Heroic's SteamGridDB
//! picker slot, served by our web image search (DuckDuckGo / Bing / Google).
//! Unlike `web_image::WebImageDialog` it saves nothing: the chosen URL goes
//! back to the dialog, which stores it in the game's `art_square` /
//! `art_cover` when the game is saved.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::{MetadataService, WebImage};

use crate::dialogs::art_images::{picture, DialogImages};
use crate::dialogs::metadata_wizard::hub;
use crate::dialogs::DialogCtx;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{new_input, search_box};
use crate::ui::icon;

mod console;

const PAGE_SIZE: usize = 20;

/// Which slot the image is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Square,
    Cover,
}

pub enum PickerEvent {
    Picked(Slot, String),
    Close,
}

pub struct ImagePicker {
    service: Arc<MetadataService>,
    images: Entity<DialogImages>,
    slot: Slot,
    query: Entity<InputState>,
    results: Vec<WebImage>,
    visible: usize,
    selected: Option<usize>,
    loading: bool,
    error: String,
    request: u64,
    /// Console mode: the grid scroll and the keyboard on top of it.
    scroll: ScrollHandle,
    layer: crate::console::Layer,
}

impl EventEmitter<PickerEvent> for ImagePicker {}

/// Default search text, like the Collection's image dialog.
pub fn default_query(title: &str, slot: Slot) -> String {
    let name = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let suffix = if slot == Slot::Square { "cover" } else { "wallpaper" };
    if name.is_empty() {
        suffix.to_string()
    } else {
        format!("\"{name}\" {suffix}")
    }
}

impl ImagePicker {
    pub fn new(ctx: &DialogCtx, title: &str, slot: Slot, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let service = hub(cx).read(cx).service.clone();
        let images = cx.new(|cx| DialogImages::new(ctx, cx));
        cx.observe(&images, |_, _, cx| cx.notify()).detach();
        let query = new_input(window, cx, &default_query(title, slot), "");
        cx.subscribe_in(&query, window, |this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        })
        .detach();
        let mut this = Self {
            service,
            images,
            slot,
            query,
            results: Vec::new(),
            visible: PAGE_SIZE,
            selected: None,
            loading: false,
            error: String::new(),
            request: 0,
            scroll: ScrollHandle::new(),
            layer: Default::default(),
        };
        // Captures of the empty state: no network.
        if !crate::env::flag("HERONITE_DEBUG_NO_SEARCH") {
            this.search(cx);
        }
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
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { service.search_web_images(&term, false) }).await;
            this.update(cx, |this, cx| {
                if this.request != id {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(list) if list.is_empty() => this.error = tr!("No images found. Try another search.").into(),
                    Ok(list) => this.results = list,
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn pick(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if let Some(image) = index.or(self.selected).and_then(|i| self.results.get(i)) {
            cx.emit(PickerEvent::Picked(self.slot, image.image_url.clone()));
            cx.emit(PickerEvent::Close);
        }
    }
}

impl Render for ImagePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.images.update(cx, |im, _| im.frame(window));
        if crate::console::active() {
            return self.render_console(cx);
        }
        let (tw, th) = if self.slot == Slot::Square { (120., 180.) } else { (240., 90.) };

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(action_button(
                "ag-pick-search",
                ButtonKind::Glass,
                tr!("Search"),
                !self.loading,
                cx.listener(|this, _, _, cx| this.search(cx)),
            ))
            .child(search_box(&self.query));

        let mut grid =
            div().id("ag-pick-grid").flex().flex_wrap().gap(px(9.6)).min_h(px(288.)).max_h(px(380.)).overflow_y_scroll().p(px(3.));
        if self.loading {
            grid = grid.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(9.6))
                    .text_size(px(16.))
                    .text_color(theme::text_bright())
                    .child(icon(Icon::FaSpinner, px(16.), theme::text_bright()).with_animation(
                        "ag-pick-spin",
                        Animation::new(Duration::from_secs(1)).repeat().with_max_fps(30.),
                        |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
                    ))
                    .child(tr!("Searching\u{2026}")),
            );
        } else {
            let shown = self.results.len().min(self.visible);
            for i in 0..shown {
                let image = self.results[i].clone();
                let selected = self.selected == Some(i);
                let chain = vec![ImageRef::Url(image.thumb_url.clone())];
                let src = self.images.update(cx, |im, _| im.get(&format!("ag:{}", image.thumb_url), chain, tw, th));
                let size = (image.width > 0 && image.height > 0).then(|| format!("{}\u{d7}{}px", image.width, image.height));
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("ag-pick-item-{i}")))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.8))
                        .p(px(6.4))
                        .rounded(px(8.))
                        .border_2()
                        .border_color(if selected { theme::accent().into() } else { gpui_kit::transparent_black() })
                        .bg(if selected { theme::ink_a(0.06) } else { theme::recess_a(0.28) })
                        .hover(|s| s.border_color(theme::accent()))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                            this.selected = Some(i);
                            if matches!(ev, ClickEvent::Mouse(m) if m.up.click_count >= 2) {
                                this.pick(Some(i), cx);
                            }
                            cx.notify();
                        }))
                        .child(picture(src).w(px(tw)).h(px(th)))
                        .children(size.map(|s| div().text_size(px(13.33)).text_color(theme::text_bright()).child(s))),
                );
            }
        }

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(16.)).text_color(theme::text_bright()).child(if self.slot == Slot::Square {
                tr!("Pick an image from the web for the cover (square art). It is used as a link; nothing is downloaded.")
            } else {
                tr!("Pick an image from the web for the background (cover/hero art). It is used as a link; nothing is downloaded.")
            }))
            .child(toolbar)
            .child(grid);
        if !self.loading && self.visible < self.results.len() {
            body = body.child(div().flex().justify_center().child(
                button("ag-pick-more", ButtonKind::Glass, tr!("Load more")).min_w(px(192.)).on_click(cx.listener(|this, _, _, cx| {
                    this.visible += PAGE_SIZE;
                    cx.notify();
                })),
            ));
        }
        if !self.error.is_empty() {
            body = body.child(div().text_size(px(16.)).text_color(theme::danger_text()).child(self.error.clone()));
        }

        let footer = div()
            .flex()
            .gap(px(10.))
            .child(action_button(
                "ag-pick-cancel",
                ButtonKind::Ghost,
                tr!("Cancel"),
                true,
                cx.listener(|_, _, _, cx| cx.emit(PickerEvent::Close)),
            ))
            .child(action_button(
                "ag-pick-select",
                ButtonKind::Primary,
                tr!("Select"),
                self.selected.is_some() && !self.loading,
                cx.listener(|this, _, _, cx| this.pick(None, cx)),
            ));
        let entity = cx.entity().downgrade();
        let vw = f32::from(window.viewport_size().width);
        dialog(
            DialogProps { id: "add-game-picker".into(), title: tr!("Search images").into(), width: 960f32.min(vw * 0.96), height: None },
            body.into_any_element(),
            Some(footer.into_any_element()),
            move |_, cx| {
                entity.update(cx, |_, cx| cx.emit(PickerEvent::Close)).ok();
            },
        )
        .into_any_element()
    }
}
