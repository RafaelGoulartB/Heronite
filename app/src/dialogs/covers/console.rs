//! "Covers" in console mode: full window, the slots (Tall, Square, Wide,
//! Background) as tabs on LB / RB, the picture the slot shows now on the
//! left and what the sources and the web found for it as a grid of
//! thumbnails, each with its resolution under it. A uses the focused
//! picture, Y types another search on the on-screen keyboard, X lists more
//! and Start opens the options (a file, one source alone, Reset).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_i18n::{tr, trn};
use heronite_images::ImageRef;

use super::slots::{Look, Slot};
use super::{found, Busy, CoversDialog};
use crate::console::browser::PickKind;
use crate::console::kit::{self, u};
use crate::console::layers::{open_browser, open_osk, open_sheet, HasLayer};
use crate::console::list::Row;
use crate::console::pointer::{self, Clickable};
use crate::console::{hint, Btn, Hint, Layer, Pad, PadTarget};
use crate::dialogs::art_images::{first_address, picture};
use crate::dialogs::metadata_update::words::source_name;
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::theme;

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// Width of the column of the current picture, design px.
const NOW_W: f32 = 290.;
/// Space around the screen's content, and between a thumbnail and the next.
const SIDE: f32 = 46.;
const GAP: f32 = 14.;
/// What a thumbnail's frame adds to its picture: padding and border, both sides.
const FRAME: f32 = 20.;

/// Width of a thumbnail's picture of each slot (its height follows the shape).
fn tile_w(slot: Slot) -> f32 {
    match slot {
        Slot::Cover(CoverFamily::Tall) => 140.,
        Slot::Cover(CoverFamily::Square) => 180.,
        Slot::Cover(CoverFamily::Wide) => 280.,
        Slot::Background => 340.,
    }
}

/// How many thumbnails of `tile` px fit a grid `room` px wide.
fn columns(room: f32, tile: f32) -> usize {
    (((room + GAP) / (tile + FRAME + GAP)).floor() as usize).max(1)
}

/// Where a press of `pad` takes the focus among `shown` thumbnails laid out
/// `cols` to a row, from `at`; and whether it asks for more first (Down on
/// the last row while the sources have more).
fn step(at: usize, shown: usize, cols: usize, pad: Pad, more: bool) -> (usize, bool) {
    let last = shown.saturating_sub(1);
    match pad {
        Pad::Left => (at.saturating_sub(1), false),
        Pad::Right => ((at + 1).min(last), false),
        Pad::Up => (at.checked_sub(cols).unwrap_or(at), false),
        Pad::Down if at + cols < shown => (at + cols, false),
        // The last row may stop short of this column: its last thumbnail.
        Pad::Down if at / cols < last / cols => (last, false),
        Pad::Down if more => (at, true),
        _ => (at, false),
    }
}

/// The presses a click on thumbnail `to` makes from the focus on `from`: A
/// when it has the focus already. Going up, the rows first, then along the
/// row; going down, along the row first, so the last row's gap is never
/// stepped into.
fn route(from: usize, to: usize, cols: usize) -> Vec<Pad> {
    let (r0, c0, r1, c1) = (from / cols, from % cols, to / cols, to % cols);
    let rows = pointer::along(r0, r1, Pad::Up, Pad::Down);
    let line = pointer::along(c0, c1, Pad::Left, Pad::Right);
    pointer::focus_or_activate(if r1 < r0 { [rows, line].concat() } else { [line, rows].concat() })
}

impl HasLayer for CoversDialog {
    fn layer_mut(&mut self) -> &mut Layer {
        &mut self.layer
    }
}

impl CoversDialog {
    /// The pictures listed (indices of [`Self::found`]): the source shown
    /// alone, the first `visible`; and whether there are more.
    fn console_listed(&self) -> (Vec<usize>, bool) {
        let shown = self.source_shown();
        let all: Vec<usize> =
            self.found().iter().enumerate().filter(|(_, f)| shown.is_none_or(|id| f.source == id)).map(|(i, _)| i).collect();
        let more = all.len() > self.visible;
        (all.into_iter().take(self.visible).collect(), more)
    }

    /// The place in `listed` of the focused picture.
    fn console_at(&self, listed: &[usize]) -> usize {
        self.selected.and_then(|s| listed.iter().position(|i| *i == s)).unwrap_or(0)
    }

    fn console_tabs(&self) -> (Vec<SharedString>, usize) {
        let names = Slot::ALL.iter().map(|s| SharedString::from(s.name())).collect();
        (names, Slot::ALL.iter().position(|s| *s == self.slot).unwrap_or(0))
    }

    /// The keyboard for another search.
    fn console_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.query.read(cx).value().to_string();
        open_osk(&mut self.layer, tr!("Search pictures"), &current, window, cx, |this, text, window, cx| {
            this.query.update(cx, |s, cx| s.set_value(text, window, cx));
            this.note = None;
            this.search(cx);
        });
    }

    fn console_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kinds = PickKind::File(IMAGE_EXTENSIONS.iter().map(|e| e.to_string()).collect());
        open_browser(&mut self.layer, tr!("Select an image"), kinds, None, window, cx, |this, path, _, cx| {
            this.use_file(path.to_string_lossy().into_owned(), cx);
        });
    }

    /// Start: a file, one source alone, Reset.
    fn console_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let idle = self.busy.is_none() && self.kept(self.slot);
        let custom = self.kind().of_art(&self.art).is_some_and(|u| !u.is_empty());
        let mut rows = vec![
            Row::action("file", tr!("Choose a file\u{2026}")).icon(Icon::FolderOpen).enabled(idle),
            Row::action("reset", tr!("Reset")).icon(Icon::Replay).enabled(idle && custom).desc(tr!("Back to what the sources gave")),
        ];
        let counts = self.source_counts();
        if counts.len() > 1 {
            let shown = self.source_shown();
            let total: usize = counts.iter().map(|(_, n)| n).sum();
            let pictures = |n: usize| trn!(n, "{n} picture", "{n} pictures");
            rows.push(Row::header(tr!("Show pictures from")));
            // A check marks the one listed.
            let mark = |row: Row, on: bool| if on { row.icon(Icon::Check) } else { row };
            rows.push(mark(Row::action("src-all", tr!("All sources")).desc(pictures(total)), shown.is_none()));
            for (id, n) in counts {
                let row = Row::action(SharedString::from(format!("src:{id}")), source_name(&self.service, id)).desc(pictures(n));
                rows.push(mark(row, shown == Some(id)));
            }
        }
        open_sheet(&mut self.layer, tr!("Options"), None, rows, window, cx, |this, id, window, cx| match id {
            "file" => this.console_file(window, cx),
            "reset" => this.reset(cx),
            "src-all" => this.show_source(None, cx),
            other => {
                let wanted = other.strip_prefix("src:");
                let source = this.source_counts().into_iter().map(|(id, _)| id).find(|id| Some(*id) == wanted);
                this.show_source(source, cx);
            }
        });
    }

    /// The current picture of the slot, in its shape, with where it comes
    /// from and what it measures.
    fn console_now(&mut self, cx: &mut Context<Self>) -> Div {
        let slot = self.slot;
        let (chain, line) = match self.info(slot) {
            Some(i) if i.look != Look::NotKept => (i.chain.clone(), self.slot_line(i)),
            Some(i) => (Vec::new(), self.slot_line(i)),
            None => (Vec::new(), String::new()),
        };
        let w = (NOW_W - 30.).min(if slot == Slot::Cover(CoverFamily::Tall) { 180. } else { NOW_W - 30. });
        let h = (w / slot.aspect()).round();
        let address = first_address(&chain);
        if self.fake.is_none() {
            self.sizes.update(cx, |s, cx| s.ask(address.clone(), cx));
        }
        let resolution = address.and_then(|url| self.sizes.read(cx).caption_of(&url));
        let pic = if chain.is_empty() {
            div().w(u(w)).h(u(h)).flex().items_center().justify_center().rounded(u(12.)).bg(kit::ink(0.06)).child(crate::ui::icon(
                Icon::Image,
                u(40.),
                theme::text_dim(),
            ))
        } else {
            let key = format!("cv-console-now:{:?}", chain[0]);
            let src = self.images.update(cx, |im, _| im.get(&key, chain, f32::from(u(w)), f32::from(u(h))));
            picture(src).w(u(w)).h(u(h)).rounded(u(12.)).shadow(kit::shadow(0.5, 8., 22.))
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(u(NOW_W))
            .gap(u(10.))
            .child(kit::kicker(tr!("Now")))
            .child(pic)
            .child(div().text_size(u(14.)).line_height(u(20.)).text_color(theme::text_muted()).child(line))
            .child(div().h(u(18.)).text_size(u(13.)).text_color(theme::text_dim()).children(resolution))
    }

    /// One found picture: the thumbnail, its source and region, its
    /// resolution (a line is kept for it while unknown).
    fn console_tile(&mut self, index: usize, focused: bool, route: Vec<Pad>, (w, h): (f32, f32), cx: &mut Context<Self>) -> Div {
        let Some((source, region, image)) = self.found().get(index).map(|f| (f.source, f.region.to_owned(), f.image.clone())) else {
            return div();
        };
        let chain = vec![ImageRef::Url(image.thumb_url.clone())];
        let src =
            self.images.update(cx, |im, _| im.get(&format!("cv-console:{}", image.thumb_url), chain, f32::from(u(w)), f32::from(u(h))));
        let name = source_name(&self.service, source);
        let place = if region.is_empty() { name.clone() } else { heronite_i18n::tr(&region).to_owned() };
        let resolution = self.sizes.read(cx).caption(&image);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(u(w + FRAME))
            .gap(u(6.))
            .p(u(8.))
            .rounded(u(14.))
            .border_2()
            .border_color(if focused { theme::console_focus().into() } else { transparent_black() })
            .bg(if focused { theme::white_a(0.1) } else { theme::black_a(0.28) })
            .on_pads(route)
            .child(picture(src).w(u(w)).h(u(h)).rounded(u(8.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(6.))
                    .min_w_0()
                    .child(labels::mark(source, &name, f32::from(u(16.))))
                    .child(div().min_w_0().truncate().text_size(u(13.)).text_color(theme::text_muted()).child(place)),
            )
            .child(div().h(u(16.)).text_size(u(12.)).text_color(theme::text_dim()).children(resolution))
    }

    /// "Tall pictures found" and how many, from how many sources.
    fn console_found_title(&self, listed: usize) -> String {
        let title = match self.slot {
            Slot::Cover(CoverFamily::Tall) => tr!("Tall pictures found"),
            Slot::Cover(CoverFamily::Square) => tr!("Square pictures found"),
            Slot::Cover(CoverFamily::Wide) => tr!("Wide pictures found"),
            Slot::Background => tr!("Backgrounds found"),
        };
        if self.loading || listed == 0 {
            return title.to_string();
        }
        let sources = trn!(self.sources_found(), "{n} source", "{n} sources");
        format!("{title} \u{b7} {}", tr!("{n} from {sources}", n = self.found().len(), sources = sources))
    }

    fn console_grid(&mut self, cols: usize, cx: &mut Context<Self>) -> AnyElement {
        let slot = self.slot;
        if let Slot::Cover(family) = slot {
            if !self.families.is_kept(family) {
                let (title, why) = match family {
                    CoverFamily::Wide => (
                        tr!("Wide covers are not kept"),
                        tr!("No view on this PC draws wide covers, so updates do not download them. Keep the shape to give games a wide picture anyway; a view that starts using it keeps it too."),
                    ),
                    _ => (
                        tr!("Square covers are not kept"),
                        tr!("No view on this PC draws square covers, so updates do not download them. Keep the shape to give games a square picture anyway; a view that starts using it keeps it too."),
                    ),
                };
                return kit::empty(Icon::Image, title, why).into_any_element();
            }
        }
        if self.loading {
            return kit::empty(Icon::FaSearch, tr!("Searching\u{2026}"), tr!("Looking for pictures on the web.")).into_any_element();
        }
        let (listed, more) = self.console_listed();
        if listed.is_empty() {
            let why = if self.error.is_empty() { tr!("Press Y to type another search.").to_string() } else { self.error.clone() };
            return kit::empty(Icon::FaSearch, tr!("No pictures found"), why).into_any_element();
        }
        let (tw, th) = (tile_w(slot), (tile_w(slot) / slot.aspect()).round());
        let at = self.console_at(&listed);
        let covered = self.layer.is_open();
        let mut grid = div().id("console-covers-grid").flex().flex_col().gap(u(GAP)).overflow_y_scroll().track_scroll(&self.console_scroll);
        for (row, chunk) in listed.chunks(cols).enumerate() {
            let mut line = div().flex().gap(u(GAP));
            for (j, index) in chunk.iter().enumerate() {
                let pos = row * cols + j;
                line = line.child(self.console_tile(*index, pos == at && !covered, route(at, pos, cols), (tw, th), cx));
            }
            grid = grid.child(line);
        }
        if more {
            grid = grid.child(kit::body_text(tr!("X shows more results.")));
        }
        self.console_scroll.scroll_to_item(at / cols);
        grid.flex_1().min_h_0().into_any_element()
    }

    pub(super) fn render_console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let slot = self.slot;
        let room = f32::from(crate::ui::content_size(window).width) / kit::scale() - 2. * SIDE - NOW_W - 30.;
        self.console_cols = columns(room, tile_w(slot));
        let (names, active) = self.console_tabs();
        let (listed, _) = self.console_listed();
        let status = match (self.busy, self.note) {
            (Some(Busy::Saving), _) => tr!("Downloading\u{2026}").to_string(),
            (Some(Busy::Keeping), _) => tr!("Saving\u{2026}").to_string(),
            (None, Some(note)) => note.to_string(),
            (None, None) => tr!("Search: {query}", query = self.query.read(cx).value().to_string()),
        };
        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .gap(u(30.))
            .px(u(SIDE))
            .pb(u(16.))
            .child(self.console_now(cx))
            .child(div().flex().flex_col().flex_1().min_w_0().min_h_0().gap(u(10.)).child(self.console_grid(self.console_cols, cx)));
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
                    .px(u(SIDE))
                    .child(kit::wordmark(tr!("COVERS")))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(u(10.))
                    .px(u(SIDE))
                    .pb(u(14.))
                    .child(kit::title(self.game.title.clone(), 28.))
                    .child(kit::tabs(&names, active)),
            )
            .children(self.kept(slot).then(|| {
                div()
                    .flex()
                    .flex_none()
                    .items_baseline()
                    .gap(u(16.))
                    .px(u(SIDE))
                    .pb(u(10.))
                    .child(div().text_size(u(15.)).text_color(theme::text_primary()).child(self.console_found_title(listed.len())))
                    .child(div().min_w_0().truncate().text_size(u(14.)).text_color(theme::text_dim()).child(status))
            }))
            .child(body)
            .children(self.layer.view())
            .into_any_element()
    }
}

impl PadTarget for CoversDialog {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        let kept = self.kept(self.slot);
        let (listed, more) = self.console_listed();
        let at = self.console_at(&listed);
        match pad {
            Pad::LB | Pad::RB => {
                let slots = Slot::ALL;
                let i = slots.iter().position(|s| *s == self.slot).unwrap_or(0) as isize;
                let next = (i + pad.dir()).clamp(0, slots.len() as isize - 1) as usize;
                self.note = None;
                self.pick_slot(slots[next], cx);
            }
            Pad::Left | Pad::Right | Pad::Up | Pad::Down if kept && !self.loading && !listed.is_empty() => {
                let (to, ask_more) = step(at, listed.len(), self.console_cols, pad, more);
                if ask_more {
                    self.visible += found::PAGE;
                    let (now, _) = self.console_listed();
                    let (to, _) = step(at, now.len(), self.console_cols, pad, false);
                    self.selected = now.get(to).copied();
                } else {
                    self.selected = listed.get(to).copied();
                }
                self.note = None;
            }
            Pad::A if self.busy.is_some() => {}
            Pad::A if !kept => {
                if let Slot::Cover(family) = self.slot {
                    self.keep_family(family, cx);
                }
            }
            Pad::A if !self.loading && !listed.is_empty() => {
                self.selected = listed.get(at).copied();
                self.use_selected(None, cx);
            }
            Pad::X if more => self.visible += found::PAGE,
            Pad::Y if kept && self.busy.is_none() => self.console_search(window, cx),
            Pad::Start if self.busy.is_none() => self.console_options(window, cx),
            Pad::B => {
                cx.emit(DialogEvent::Close);
            }
            // The presses the dialog uses, and A / X / Y it has no use for, stay inside.
            _ => return matches!(pad, Pad::Left | Pad::Right | Pad::Up | Pad::Down | Pad::A | Pad::X | Pad::Y | Pad::Start),
        }
        cx.notify();
        true
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        let kept = self.kept(self.slot);
        let (listed, more) = self.console_listed();
        let mut h = Vec::new();
        match (kept, self.busy) {
            (_, Some(_)) => {}
            (false, None) => h.push(hint(Btn::A, tr!("Keep this shape"))),
            (true, None) if !listed.is_empty() => h.push(hint(Btn::A, tr!("Use picture"))),
            _ => {}
        }
        if kept {
            h.push(hint(Btn::Y, tr!("Search")));
        }
        if more {
            h.push(hint(Btn::X, tr!("More")));
        }
        h.extend([hint(Btn::Bumpers, tr!("Tabs")), hint(Btn::Start, tr!("Options")), hint(Btn::B, tr!("Done"))]);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn the_grid_fits_whole_thumbnails() {
        // 1500 px of room, 140 px pictures in 20 px frames and 14 px gaps.
        assert_eq!(columns(1500., 140.), 8);
        assert_eq!(columns(100., 340.), 1);
    }

    #[test]
    fn the_focus_moves_along_the_grid_and_stays_in_it() {
        assert_eq!(step(0, 7, 3, Pad::Right, false), (1, false));
        assert_eq!(step(6, 7, 3, Pad::Right, false), (6, false));
        assert_eq!(step(0, 7, 3, Pad::Left, false), (0, false));
        assert_eq!(step(4, 7, 3, Pad::Up, false), (1, false));
        assert_eq!(step(1, 7, 3, Pad::Up, false), (1, false));
        assert_eq!(step(1, 7, 3, Pad::Down, false), (4, false));
        // The last row has no thumbnail under the second one: it goes to its last.
        assert_eq!(step(4, 7, 3, Pad::Down, false), (6, false));
        assert_eq!(step(6, 7, 3, Pad::Down, false), (6, false));
    }

    #[test]
    fn down_on_the_last_row_asks_for_more_when_there_is() {
        assert_eq!(step(6, 7, 3, Pad::Down, true), (6, true));
        assert_eq!(step(4, 7, 3, Pad::Down, true), (6, false));
        assert_eq!(step(2, 7, 3, Pad::Down, true), (5, false));
    }

    #[test]
    fn a_click_presses_the_way_to_the_thumbnail() {
        // Two columns of three rows; the focus is on the middle of the left column.
        assert_eq!(route(2, 2, 2), [Pad::A]);
        assert_eq!(route(2, 3, 2), [Pad::Right]);
        assert_eq!(route(2, 0, 2), [Pad::Up]);
        assert_eq!(route(3, 0, 2), [Pad::Up, Pad::Left]);
        assert_eq!(route(0, 5, 2), [Pad::Right, Pad::Down, Pad::Down]);
    }
}
