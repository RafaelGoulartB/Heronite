//! Cover and background art: the previews and their menu, pickers, file
//! choosers and the automatic search.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::GameInfo;
use heronite_i18n::tr;
use heronite_images::ImageRef;

use super::logic::*;
use super::picker::{ImagePicker, PickerEvent, Slot};
use super::{text_of, AddGameDialog};
use crate::dialogs::art_images::{chain_for_url, picture};
use crate::icons::Icon;
use crate::theme;
use crate::ui::dropdown::{menu_item, menu_panel, menu_separator};
use crate::ui::fields::{choose_path, input_box, labeled, meta_text};
use crate::ui::icon;

/// Cover/background preview box sizes (600x900 and 1600x650 ratios).
const SQUARE_W: f32 = 168.;
const SQUARE_H: f32 = 252.;
const HERO_W: f32 = 168.;
const HERO_H: f32 = 68.;

impl AddGameDialog {
    pub(super) fn open_picker(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        self.art_menu = None;
        let title = text_of(&self.title, cx);
        let ctx = self.ctx.clone();
        let view = cx.new(|cx| ImagePicker::new(&ctx, &title, slot, window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, ev: &PickerEvent, window, cx| match ev {
            PickerEvent::Picked(slot, url) => {
                this.art_input(*slot).clone().update(cx, |s, cx| s.set_value(url.clone(), window, cx));
            }
            PickerEvent::Close => {
                this.picker = None;
                cx.notify();
            }
        });
        self.picker = Some((view, sub));
        cx.notify();
    }

    fn choose_art_file(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        choose_path(window, cx, false, tr!("Select Image"), move |this: &mut Self, path, window, cx| {
            this.art_input(slot).clone().update(cx, |s, cx| s.set_value(file_url(&path), window, cx));
        });
    }

    /// Title field lost focus: like Heroic, look for a cover when there is none.
    pub(super) fn auto_find_art(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx).trim().to_string();
        let empty = text_of(&self.square, cx).trim().is_empty() && text_of(&self.hero, cx).trim().is_empty();
        if self.is_edit() || title.is_empty() || !empty || self.finding || self.auto_art_tried.as_deref() == Some(title.as_str()) {
            return;
        }
        if crate::env::flag("HERONITE_DEBUG_DIALOG") {
            return;
        }
        self.find_art(window, cx);
    }

    /// Looks the title up (Steam CDN, SteamGridDB with a key, Bottles' mirror)
    /// and fills the art fields that are empty.
    pub(super) fn find_art(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx).trim().to_string();
        if title.is_empty() || self.finding {
            return;
        }
        self.auto_art_tried = Some(title.clone());
        self.finding = true;
        self.notice.clear();
        let service = crate::dialogs::metadata_wizard::hub(cx).read(cx).service.clone();
        let app_name = self.app_name.clone();
        cx.spawn_in(window, async move |this, cx| {
            let t = title.clone();
            let found = cx
                .background_spawn(async move {
                    let key = heronite_metadata::covers::steamgrid_api_key(&service.paths);
                    let meta = heronite_data::models::LocalGameMeta {
                        app_name: app_name.clone(),
                        runner: "sideload".into(),
                        title: t.clone(),
                        ..Default::default()
                    };
                    let game = GameInfo { runner: "sideload".into(), app_name, title: t, ..Default::default() };
                    heronite_metadata::covers::fetch_covers_for_game(&service.http, key.as_deref(), &meta, &game)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.finding = false;
                let fill = |input: &Entity<InputState>, value: Option<String>, window: &mut Window, cx: &mut Context<Self>| -> bool {
                    match value.filter(|v| !v.is_empty()) {
                        Some(v) if text_of(input, cx).trim().is_empty() => {
                            input.update(cx, |s, cx| s.set_value(v, window, cx));
                            true
                        }
                        _ => false,
                    }
                };
                let a = fill(&this.square, found.art_square.clone(), window, cx);
                let b = fill(&this.hero, found.art_cover.clone(), window, cx);
                this.notice =
                    if a || b { tr!("Art found. Click an image to change it.").into() } else { tr!("No art found for this title.").into() };
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn art_input(&self, slot: Slot) -> &Entity<InputState> {
        if slot == Slot::Square {
            &self.square
        } else {
            &self.hero
        }
    }

    /// The previews (a click opens what can change them), and the address
    /// field of one when "Type an address" was picked.
    pub(super) fn art_column(&mut self, square_url: &str, hero_url: &str, cx: &mut Context<Self>) -> Div {
        let mut col = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(12.))
            .w(px(SQUARE_W))
            .child(self.art_slot("ag-art-square", Slot::Square, square_url, SQUARE_W, SQUARE_H, cx))
            .child(self.art_slot("ag-art-hero", Slot::Cover, hero_url, HERO_W, HERO_H, cx))
            .child(meta_text(tr!("Click an image to change it.")).text_center());
        if let Some(slot) = self.art_address {
            let label = if slot == Slot::Square { tr!("Cover art address") } else { tr!("Background art address") };
            col = col.child(labeled(label, input_box(self.art_input(slot), false)));
        }
        col
    }

    /// A preview and, when open, its menu below it.
    fn art_slot(&self, id: &'static str, slot: Slot, url: &str, w: f32, h: f32, cx: &mut Context<Self>) -> Div {
        div()
            .relative()
            .child(self.art_box(id, slot, url, w, h, cx))
            .when(self.art_menu == Some(slot), |d| d.child(self.art_menu_panel(slot, h + 6., cx)))
    }

    fn art_box(&self, id: &'static str, slot: Slot, url: &str, w: f32, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let chain: Vec<ImageRef> = chain_for_url(&self.ctx.data_paths, None, url.trim());
        let src = if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("ag-{id}:{url}"), chain, w, h)) };
        let empty = src.is_none();
        let open = self.art_menu == Some(slot);
        div()
            .id(id)
            .relative()
            .w(px(w))
            .h(px(h))
            .rounded(px(theme::R_INPUT))
            .overflow_hidden()
            .cursor_pointer()
            .bg(theme::input_fill())
            .border_1()
            .border_color(if open { theme::accent().into() } else { theme::input_border() })
            .hover(|s| s.border_color(theme::accent()))
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                if *hovered {
                    this.art_hover = Some(slot);
                } else if this.art_hover == Some(slot) {
                    this.art_hover = None;
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.art_menu = if this.art_menu == Some(slot) { None } else { Some(slot) };
                this.open_select = None;
                cx.notify();
            }))
            .child(picture(src).size_full())
            .when(empty, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .text_size(px(13.))
                        .text_color(theme::text_muted())
                        .child(icon(Icon::Image, px(20.), theme::text_muted()))
                        .child(if slot == Slot::Square { tr!("Cover") } else { tr!("Background") }),
                )
            })
    }

    /// Find art (while the slot is empty), search the web, a file, an
    /// address, remove. Each closes the menu.
    fn art_menu_panel(&self, slot: Slot, top: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let has = !text_of(self.art_input(slot), cx).trim().is_empty();
        let can_find = !has && !self.finding && !text_of(&self.title, cx).trim().is_empty();
        let item = |key: &str, ic: Icon, label: &'static str, color: Rgba| {
            menu_item(SharedString::from(format!("ag-art-menu-{key}")), false)
                .child(icon(ic, px(16.), color))
                .child(div().text_color(color).child(label))
        };
        let close = |this: &mut Self| this.art_menu = None;
        let mut menu = menu_panel(232.).on_mouse_down_out(cx.listener(move |this, _, _, cx| {
            // A click on the preview itself toggles the menu.
            if this.art_menu == Some(slot) && this.art_hover != Some(slot) {
                this.art_menu = None;
                cx.notify();
            }
        }));
        if !has {
            let find = item("find", Icon::AutoAwesome, tr!("Find art for this title"), theme::text_primary());
            menu = menu.child(if can_find {
                find.on_click(cx.listener(move |this, _, window, cx| {
                    close(this);
                    this.find_art(window, cx);
                }))
            } else {
                find.opacity(0.45).cursor_not_allowed()
            });
        }
        menu = menu
            .child(
                item("web", Icon::TravelExplore, tr!("Search the web"), theme::text_primary())
                    .on_click(cx.listener(move |this, _, window, cx| this.open_picker(slot, window, cx))),
            )
            .child(item("file", Icon::Folder, tr!("Choose an image file"), theme::text_primary()).on_click(cx.listener(
                move |this, _, window, cx| {
                    close(this);
                    this.choose_art_file(slot, window, cx);
                    cx.notify();
                },
            )))
            .child(item("url", Icon::Link, tr!("Type an address"), theme::text_primary()).on_click(cx.listener(
                move |this, _, window, cx| {
                    close(this);
                    this.art_address = Some(slot);
                    this.art_input(slot).clone().update(cx, |s, cx| s.focus(window, cx));
                    cx.notify();
                },
            )));
        if has {
            menu = menu.child(menu_separator()).child(
                item("remove", Icon::DeleteOutline, tr!("Remove the image"), theme::danger_text()).on_click(cx.listener(
                    move |this, _, window, cx| {
                        close(this);
                        this.art_input(slot).clone().update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    },
                )),
            );
        }
        deferred(div().absolute().top(px(top)).left_0().w(px(232.)).occlude().child(menu)).with_priority(40)
    }
}
