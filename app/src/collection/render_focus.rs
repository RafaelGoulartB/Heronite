//! The focus panel as the Collection view builds it: image resolution and the
//! callbacks into the view.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_images::{ImageRef, Variant};

use super::{focus, html, CollectionView};
use crate::theme;

impl CollectionView {
    pub(super) fn render_focus_panel(&mut self, vw: f32, list_w: f32, scale: f32, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let active_facets: Vec<(&'static str, SharedString)> = heronite_data::collection::FACET_KINDS
            .iter()
            .filter_map(|k| self.ui.facet_filters.get(*k).map(|v| (k.as_str(), SharedString::from(v.to_string()))))
            .collect();
        let Some(vm) = self.focus.clone() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(15.2))
                .text_color(theme::white_a(0.52))
                .child("No game selected")
                .into_any_element();
        };
        let covers = self.focus_covers(&vm, vw, list_w, scale);
        let html_images = self.focus_html_images(&vm, vw, list_w, scale);
        let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
        let (w5, w6) = (weak.clone(), weak.clone());
        let focus_key = vm.key.clone();
        let epic_focus = self.epic_focus(&vm.key, vm.installed, cx);
        let screenshots = self.screenshots_section(cx);
        let focus_extras = self.focus_extras(&focus_key, scale);
        focus::focus_panel(
            focus::FocusProps {
                vm,
                cover_for: Rc::new(move |k| covers.get(k).cloned()),
                html_images: Rc::new(move |u| html_images.get(u).cloned()),
                scroll: self.focus_scroll.clone(),
                viewport_w: vw,
                panel_w: vw - list_w,
                active_facets,
                screenshots,
                epic: epic_focus,
                extras: focus_extras,
            },
            focus::FocusActions {
                on_scroll: Box::new(cx.listener(|_, _: &(), _, cx| cx.notify()).into_fn()),
                on_play: Box::new(move |_, cx| {
                    w3.update(cx, |this, cx| {
                        this.primary_action(&focus_key, cx);
                        cx.notify();
                    })
                    .ok();
                }),
                on_favourite: Box::new(move |_, cx| {
                    w4.update(cx, |this, cx| this.toggle_favourite(cx)).ok();
                }),
                on_metadata: Box::new(move |window, cx| {
                    w5.update(cx, |this, cx| this.open_metadata(window, cx)).ok();
                }),
                on_game_settings: Box::new(move |window, cx| {
                    w6.update(cx, |this, cx| this.open_game_settings(window, cx)).ok();
                }),
                on_facet: Rc::new(move |f, v, _, cx| {
                    w1.update(cx, |this, cx| {
                        this.toggle_facet(f, &v);
                        cx.notify();
                    })
                    .ok();
                }),
                on_select: Rc::new(move |k, _, cx| {
                    w2.update(cx, |this, cx| {
                        this.select(k, true);
                        this.reveal_selected();
                        cx.notify();
                    })
                    .ok();
                }),
            },
        )
    }

    /// Every cover the panel shows: the game's own and the related games'.
    fn focus_covers(&mut self, vm: &super::vm::FocusVm, vw: f32, list_w: f32, scale: f32) -> HashMap<SharedString, ImageSource> {
        let mut covers: HashMap<SharedString, ImageSource> = HashMap::new();
        let narrow = vw - list_w <= 1100. - theme::LIST_COL_W_MD;
        let fw = if narrow { 72. } else { 86. };
        if let Some(src) = self.cover(&vm.cover_key, fw, fw * 1.5, scale, 200) {
            covers.insert(vm.cover_key.clone(), src);
        }
        for g in &vm.related {
            for c in &g.games {
                if let Some(src) = self.cover(&c.cover_key, 78., 117., scale, 50) {
                    covers.insert(c.cover_key.clone(), src);
                }
            }
        }
        covers
    }

    /// Images inside the overview HTML, by URL.
    fn focus_html_images(&mut self, vm: &super::vm::FocusVm, vw: f32, list_w: f32, scale: f32) -> HashMap<String, ImageSource> {
        let mut html_images: HashMap<String, ImageSource> = HashMap::new();
        let Some(blocks) = vm.overview_html.as_ref() else { return html_images };
        let column = focus::main_column_width(vw - list_w);
        for b in blocks.iter() {
            if let html::Block::Image { url, width, height } = b {
                let (w, h) = focus::html_image_size(*width, *height, column);
                let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
                let id: SharedString = format!("h:{url}:{pw}x{ph}").into();
                let chain = vec![ImageRef::Url(url.clone())];
                if let Some(img) = self.images.get_or_request(&id, || chain, pw, ph, Variant::Cover, 20) {
                    html_images.insert(url.clone(), ImageSource::Render(img));
                }
            }
        }
        html_images
    }
}

trait IntoFn {
    fn into_fn(self) -> Box<dyn Fn(&mut Window, &mut App)>;
}

impl<F: Fn(&(), &mut Window, &mut App) + 'static> IntoFn for F {
    fn into_fn(self) -> Box<dyn Fn(&mut Window, &mut App)> {
        Box::new(move |w, cx| self(&(), w, cx))
    }
}
