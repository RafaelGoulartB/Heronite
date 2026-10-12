//! Cover and background art: pickers, file choosers and the automatic search.

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
use crate::ui::fields::{choose_path, path_field};
use crate::ui::icon;

impl AddGameDialog {
    pub(super) fn open_picker(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        let title = text_of(&self.title, cx);
        let ctx = self.ctx.clone();
        let view = cx.new(|cx| ImagePicker::new(&ctx, &title, slot, window, cx));
        let sub = cx.subscribe_in(&view, window, |this, _, ev: &PickerEvent, window, cx| match ev {
            PickerEvent::Picked(slot, url) => {
                let input = if *slot == Slot::Square { this.square.clone() } else { this.hero.clone() };
                input.update(cx, |s, cx| s.set_value(url.clone(), window, cx));
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
            let input = if slot == Slot::Square { this.square.clone() } else { this.hero.clone() };
            input.update(cx, |s, cx| s.set_value(file_url(&path), window, cx));
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
                    if a || b { tr!("Art found. You can change it below.").into() } else { tr!("No art found for this title.").into() };
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn art_box(&self, id: &'static str, slot: Slot, url: &str, w: f32, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let chain: Vec<ImageRef> = chain_for_url(&self.ctx.data_paths, None, url.trim());
        let src = if chain.is_empty() { None } else { self.images.update(cx, |im, _| im.get(&format!("ag-{id}:{url}"), chain, w, h)) };
        let empty = src.is_none();
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
            .border_color(theme::input_border())
            .hover(|s| s.border_color(theme::accent()))
            .on_click(cx.listener(move |this, _, window, cx| this.open_picker(slot, window, cx)))
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
                        .child(icon(Icon::FaSearch, px(18.), theme::text_muted()))
                        .child(if slot == Slot::Square { tr!("Cover") } else { tr!("Background") }),
                )
            })
    }

    pub(super) fn art_field(&self, id: &'static str, label: &str, input: &Entity<InputState>, slot: Slot, cx: &mut Context<Self>) -> Div {
        let has = !text_of(input, cx).is_empty();
        path_field(
            SharedString::from(format!("{id}-pick")),
            Some(label),
            input,
            has,
            false,
            cx.listener(move |this, _, window, cx| {
                if has {
                    let input = if slot == Slot::Square { this.square.clone() } else { this.hero.clone() };
                    input.update(cx, |s, cx| s.set_value("", window, cx));
                } else {
                    this.choose_art_file(slot, window, cx);
                }
            }),
        )
    }
}
