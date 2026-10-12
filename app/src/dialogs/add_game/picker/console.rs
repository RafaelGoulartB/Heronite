//! The web image search in console mode: a full-window grid of thumbnails
//! moved with the d-pad, A picks, Y edits the search text with the
//! on-screen keyboard, X shows more results.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_images::ImageRef;

use super::{ImagePicker, PickerEvent, Slot, PAGE_SIZE};
use crate::console::kit::{self, u};
use crate::console::osk::{Osk, OskEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::dialogs::art_images::picture;
use crate::icons::Icon;
use crate::theme;

impl ImagePicker {
    /// Thumbnails per row and their size (design px).
    fn console_grid(&self) -> (usize, f32, f32) {
        if self.slot == Slot::Square {
            (6, 150., 225.)
        } else {
            (3, 330., 124.)
        }
    }

    fn console_cursor(&self) -> usize {
        let shown = self.results.len().min(self.visible);
        self.selected.unwrap_or(0).min(shown.saturating_sub(1))
    }

    fn open_search_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.query.read(cx).value().to_string();
        let osk = cx.new(|_| Osk::new(tr!("Search images"), &current));
        let sub = cx.subscribe_in(&osk, window, |this, _, ev: &OskEvent, window, cx| {
            if let OskEvent::Done(text) = ev {
                let text = text.clone();
                this.query.update(cx, |s, cx| s.set_value(text, window, cx));
                this.search(cx);
            }
            this.layer.close();
            cx.notify();
        });
        self.layer.open(&osk, sub);
    }

    pub(super) fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (cols, tw, th) = self.console_grid();
        let cursor = self.console_cursor();
        let shown = self.results.len().min(self.visible);
        let heading = if self.slot == Slot::Square { tr!("Pick the cover art") } else { tr!("Pick the background art") };

        let mut grid = div().id("console-ag-grid").flex().flex_col().gap(u(14.)).overflow_y_scroll().track_scroll(&self.scroll);
        if self.loading {
            grid = grid.child(kit::empty(Icon::FaSearch, tr!("Searching\u{2026}"), tr!("Looking for images on the web.")));
        } else if shown == 0 {
            let msg = if self.error.is_empty() { tr!("Press Y to type a search.") } else { self.error.as_str() };
            grid = grid.child(kit::empty(Icon::FaSearch, tr!("No images"), msg.to_string()));
        } else {
            let mut i = 0;
            while i < shown {
                let mut line = div().flex().gap(u(14.));
                for j in i..(i + cols).min(shown) {
                    let image = self.results[j].clone();
                    let focused = j == cursor && !self.layer.is_open();
                    let chain = vec![ImageRef::Url(image.thumb_url.clone())];
                    let src = self
                        .images
                        .update(cx, |im, _| im.get(&format!("ag:{}", image.thumb_url), chain, f32::from(u(tw)), f32::from(u(th))));
                    let size = (image.width > 0 && image.height > 0).then(|| format!("{}\u{d7}{}", image.width, image.height));
                    line = line.child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(u(6.))
                            .p(u(8.))
                            .rounded(u(14.))
                            .border_2()
                            .border_color(if focused { theme::console_focus().into() } else { transparent_black() })
                            .bg(if focused { theme::white_a(0.1) } else { theme::black_a(0.28) })
                            .child(picture(src).w(u(tw)).h(u(th)).rounded(u(8.)))
                            .child(div().h(u(16.)).text_size(u(12.)).text_color(theme::text_muted()).children(size)),
                    );
                }
                grid = grid.child(line);
                i += cols;
            }
            if self.visible < self.results.len() {
                grid = grid.child(kit::body_text(tr!("X shows more results.")));
            }
            // Keep the cursor's row in view: the grid is a column of rows.
            self.scroll.scroll_to_item(cursor / cols);
        }

        let query = self.query.read(cx).value().to_string();
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .pb(u(60.))
            .bg(theme::console_bg())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(u(36.))
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark(tr!("SEARCH IMAGES")))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .px(u(46.))
                    .pb(u(14.))
                    .child(kit::title(heading, 28.))
                    .child(kit::body_text(tr!("Search: {query}. The image is used as a link; nothing is downloaded.", query))),
            )
            .child(div().flex().flex_col().flex_1().min_h_0().px(u(46.)).pb(u(16.)).child(grid.flex_1().min_h_0()))
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for ImagePicker {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let (cols, _, _) = self.console_grid();
        let shown = self.results.len().min(self.visible);
        let cur = self.console_cursor();
        match pad {
            Pad::Left | Pad::Right | Pad::Up | Pad::Down if shown > 0 => {
                let next = match pad {
                    Pad::Left => cur.saturating_sub(1),
                    Pad::Right => (cur + 1).min(shown - 1),
                    Pad::Up => cur.checked_sub(cols).unwrap_or(cur),
                    _ if cur + cols < shown => cur + cols,
                    _ => {
                        // Last row: more results load when there are some.
                        if self.visible < self.results.len() {
                            self.visible += PAGE_SIZE;
                        }
                        cur
                    }
                };
                self.selected = Some(next);
            }
            Pad::A if shown > 0 && !self.loading => {
                self.selected = Some(cur);
                self.pick(None, cx);
            }
            Pad::X if !self.loading && self.visible < self.results.len() => self.visible += PAGE_SIZE,
            Pad::Y => self.open_search_keyboard(window, cx),
            Pad::B => {
                cx.emit(PickerEvent::Close);
                return true;
            }
            _ => return matches!(pad, Pad::Left | Pad::Right | Pad::Up | Pad::Down | Pad::A | Pad::X),
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        let mut h = vec![hint(Btn::A, tr!("Use image")), hint(Btn::Y, tr!("Search"))];
        if self.visible < self.results.len() {
            h.push(hint(Btn::X, tr!("More")));
        }
        h.push(hint(Btn::B, tr!("Back")));
        h
    }
}
