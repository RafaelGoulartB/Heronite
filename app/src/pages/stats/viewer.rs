//! The screenshot viewer of the Statistics page: one capture as large as
//! the window allows, its game and moment, arrows (and ← →) through the list
//! it came from, Esc or a click outside closes it. It opens from the Album,
//! the game sheet and the month cards.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDateTime;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{day_label, StatsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// A capture of the album: game, when, file.
#[derive(Clone, Debug)]
pub(super) struct Photo {
    pub game: usize,
    pub at: NaiveDateTime,
    pub path: PathBuf,
}

/// What the viewer shows: a list of captures and the one open.
pub(super) struct Lightbox {
    pub photos: Arc<Vec<Photo>>,
    pub index: usize,
}

impl StatsPage {
    /// Opens the viewer on `photos[index]`.
    pub(super) fn open_photo(&mut self, photos: Arc<Vec<Photo>>, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if photos.is_empty() {
            return;
        }
        self.lightbox = Some(Lightbox { index: index.min(photos.len() - 1), photos });
        self.hover = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn step_photo(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(lb) = self.lightbox.as_mut() {
            let n = lb.photos.len() as isize;
            lb.index = ((lb.index as isize + delta).rem_euclid(n)) as usize;
            cx.notify();
        }
    }

    /// Opens the viewer on every capture of `game`, at the one at `path`. The
    /// list is made on the click: built while rendering it cost a copy of
    /// every capture of the game on each frame.
    pub(super) fn open_game_photo(&mut self, game: usize, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let photos = self.game_photos(game);
        let at = photos.iter().position(|p| p.path == path).unwrap_or(0);
        self.open_photo(Arc::new(photos), at, window, cx);
    }

    /// The captures of `game` (all of them, oldest first), for the viewer.
    fn game_photos(&self, game: usize) -> Vec<Photo> {
        self.shots
            .as_ref()
            .and_then(|all| all.get(&game))
            .map(|l| l.iter().map(|s| Photo { game, at: s.at, path: s.path.clone() }).collect())
            .unwrap_or_default()
    }

    /// The viewer over the page: the capture as large as it fits, its game
    /// and day, arrows to the others of the list (also ← →), Esc closes.
    pub(super) fn render_lightbox(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let lb = self.lightbox.as_ref()?;
        let (photos_len, index) = (lb.photos.len(), lb.index);
        let p = lb.photos[index].clone();
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width) - 220., f32::from(vp.height) - 220.);
        let (w, h) = if vw / vh > 16. / 9. { (vh * 16. / 9., vh) } else { (vw, vw * 9. / 16.) };
        let src = self.covers.get_file(&p.path, w, h, window.scale_factor());
        // The small preview while the full image loads.
        let src = src.or_else(|| self.shot(&p.path, w, h, window, cx));
        let arrow = |id: &'static str, ic: Icon, delta: isize| {
            div()
                .id(id)
                .flex_none()
                .size(px(48.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::white_a(0.08))
                .cursor_pointer()
                .hover(|s| s.bg(theme::white_a(0.16)))
                .on_click(cx.listener(move |this, _, _, cx| this.step_photo(delta, cx)))
                .child(icon(ic, px(24.), theme::text_primary()))
        };
        let game = p.game;
        let caption = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(16.)).child(self.game_title(game)))
            .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(format!(
                "{} \u{b7} {}",
                day_label(p.at.date()),
                p.at.format("%H:%M")
            )))
            .child(div().text_size(px(13.)).text_color(theme::text_dim()).child(tr!("{i} of {n}", i = index + 1, n = photos_len)))
            .child(div().flex_1())
            .child(
                crate::ui::controls::button_sm("lightbox-sheet", crate::ui::controls::ButtonKind::Glass, tr!("Open game sheet")).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.lightbox = None;
                        this.open_sheet(game, window, cx);
                    }),
                ),
            );
        let close = crate::ui::controls::close_button("lightbox-close").on_click(cx.listener(|this, _, _, cx| {
            this.lightbox = None;
            cx.notify();
        }));
        Some(
            div()
                .id("stats-lightbox")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .bg(theme::black_a(0.9))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.lightbox = None;
                    cx.notify();
                }))
                .child(div().absolute().top(px(18.)).right(px(18.)).child(close))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(20.))
                        .child(arrow("lightbox-prev", Icon::ChevronLeft, -1))
                        .child(
                            div()
                                .id("lightbox-image")
                                .w(px(w))
                                .h(px(h))
                                .rounded(px(12.))
                                .overflow_hidden()
                                .bg(theme::white_a(0.04))
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .children(src.map(|s| img(s).size_full().object_fit(ObjectFit::Contain))),
                        )
                        .child(arrow("lightbox-next", Icon::ChevronRight, 1)),
                )
                .child(div().id("lightbox-caption").w(px(w)).on_click(|_, _, cx| cx.stop_propagation()).child(caption))
                .into_any_element(),
        )
    }
}
