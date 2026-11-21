//! What Spotlight opens over the whole window: a store screenshot full
//! screen (← → browse, A / B close; the user's own captures use the game
//! page's viewer) and About this game's whole text in a sheet that scrolls
//! (↑ ↓ or the right stick, B closes).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_images::{ImageRef, Variant};

use super::super::super::super::CollectionView;
use crate::console::kit::{self, u};
use crate::console::motion::{self, Glide};
use crate::console::pointer::Clickable;
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

/// The sheet's width, design px.
const SHEET_W: f32 = 820.;
/// How far one press scrolls the sheet, design px.
const SCROLL_STEP: f32 = 110.;

/// A store screenshot at `w`×`h` px (the picture is cropped to that shape).
pub(super) fn store_image(view: &mut CollectionView, url: &str, w: u32, h: u32, priority: i32) -> Option<ImageSource> {
    if url.is_empty() {
        return None;
    }
    let id: SharedString = format!("sm:{w}x{h}:{url}").into();
    let chain = || vec![ImageRef::Url(url.to_string())];
    view.images.get_or_request(&id, chain, w, h, Variant::Cover, priority).map(ImageSource::Render)
}

/// A spinner while a picture loads.
fn spinner(id: &'static str) -> AnyElement {
    crate::ui::icon(Icon::FaSpinner, u(28.), theme::white_a(0.7))
        .with_animation(id, Animation::new(std::time::Duration::from_secs(1)).repeat().with_max_fps(30.), |s, t| {
            s.with_transformation(Transformation::rotate(percentage(t)))
        })
        .into_any_element()
}

/// The hint bar of a layer over the window (the shell's sits under it).
fn layer_hints(hints: &[Hint]) -> Div {
    div().absolute().left_0().right_0().bottom_0().child(kit::hint_bar(hints).bg(transparent_black()).border_color(transparent_black()))
}

impl CollectionView {
    /// Store screenshot `i` of `urls` over the whole window.
    pub(super) fn spotlight_viewer(&mut self, urls: &[String], i: usize, window: &Window) -> AnyElement {
        let size = crate::ui::content_size(window);
        let dpr = window.scale_factor();
        // One size for every screenshot: they are 16:9 on the stores.
        let w = (f32::from(size.width) * dpr).min(2560.);
        let (pw, ph) = (w.round() as u32, (w * 9. / 16.).round() as u32);
        let picture = match urls.get(i).and_then(|u| store_image(self, u, pw, ph, 400)) {
            Some(src) => img(src).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            None => spinner("spot-viewer-loading"),
        };
        let hints = [hint(Btn::LeftRight, tr!("Browse")), hint(Btn::B, tr!("Close"))];
        let layer = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::black_a(0.94))
            .on_pads([Pad::B])
            .child(div().size_full().px(u(40.)).pt(u(36.)).pb(u(80.)).flex().items_center().justify_center().child(picture))
            .child(
                div()
                    .absolute()
                    .bottom(u(26.))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .text_size(u(14.))
                    .text_color(theme::white_a(0.78))
                    .child(format!("{}/{}", i + 1, urls.len())),
            )
            .child(layer_hints(&hints));
        kit::window_layer(window, motion::fade_in("spot-viewer", layer, 180))
    }

    /// About this game's whole text in a sheet over the whole window.
    pub(super) fn spotlight_about_sheet(&self, title: &str, text: &str, scroll: &Glide, window: &Window) -> AnyElement {
        let paragraphs = text
            .split('\n')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| div().text_size(u(15.)).line_height(u(24.)).text_color(theme::text_primary()).child(p.to_string()));
        let body = div()
            .id("spot-about-text")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(scroll.handle())
            .px(u(34.))
            .pb(u(30.))
            .child(div().flex().flex_col().gap(u(14.)).children(paragraphs))
            .child(scroll.driver());
        let panel = div()
            .flex()
            .flex_col()
            .w(u(SHEET_W))
            .max_h(relative(0.8))
            .rounded(u(22.))
            .overflow_hidden()
            .bg(theme::console_sidebar())
            .border_1()
            .border_color(kit::ink(0.1))
            .shadow(kit::shadow(0.55, 18., 56.))
            .eats_clicks()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .px(u(34.))
                    .pt(u(28.))
                    .pb(u(16.))
                    .child(kit::kicker(tr!("About this game")))
                    .child(kit::title(title.to_string(), 26.)),
            )
            .child(body);
        let hints = [hint(Btn::B, tr!("Close"))];
        let layer = div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_pads([Pad::B])
            .child(motion::fade_in("spot-about-dim", div().absolute().inset_0().bg(kit::dim(0.6)), 180))
            .child(motion::enter("spot-about-sheet", panel, 260, 0., 24.))
            .child(layer_hints(&hints));
        kit::window_layer(window, layer)
    }

    /// A press while the sheet is open: ↑ ↓ scroll it, A / B close it.
    pub(super) fn spotlight_sheet_pad(scroll: &Glide, pad: Pad) -> bool {
        match pad {
            Pad::Up | Pad::ScrollUp => scroll.by(-SCROLL_STEP),
            Pad::Down | Pad::ScrollDown => scroll.by(SCROLL_STEP),
            Pad::A | Pad::B => return false,
            _ => {}
        }
        true
    }
}
