//! The pictures (each kept cover family and the background), above the
//! other fields: a row each, where every value a source has is a tile in
//! the picture's shape (one height for all, so the rows stay low) with the
//! sources' marks over it, and a click picks it (the tile under the pointer
//! shows its details). Beside the tiles,
//! the details of that value: who has it, its size (read from the
//! picture's first bytes, `MetadataService::picture_sizes`) and what to
//! know before picking it: whether it is of the field's shape or a view
//! crops it, a low resolution, and a picture set by hand in Covers that is
//! shown instead.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_i18n::tr;
use heronite_metadata::picture_size::family_of;
use heronite_metadata::ArtKind;

use super::fields::{Choice, LABEL_W};
use super::{field_label, is_picture, MetadataDialog};
use crate::dialogs::covers::Slot;
use crate::dialogs::metadata_update::words;
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::theme;
use crate::ui::icon;

/// Width of the details beside the tiles.
const DETAILS_W: f32 = 210.;
/// Below this many pixels on its short side a cover is low resolution (IGDB's
/// usual cover is 264 wide); a background below this width.
const LOW_COVER: u32 = 200;
const LOW_BACKGROUND: u32 = 960;

/// The height of every tile's picture.
const TILE_H: f32 = 60.;
/// The size of the arrows over a row of tiles that does not fit.
const ARROW: f32 = 28.;

/// The size of a field's tile picture: [`TILE_H`] high, in its shape.
fn tile_size(field: &str) -> (f32, f32) {
    let w = match field {
        "cover" => 40.,
        "coverSquare" => 60.,
        "coverWide" => 128.,
        _ => 107.,
    };
    (w, TILE_H)
}

/// The shape a field's picture is meant to have (a background is wide).
fn shape_of(field: &str) -> CoverFamily {
    CoverFamily::from_field(field).unwrap_or(CoverFamily::Wide)
}

/// What a size says of a picture for `field`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fit {
    Own,
    /// Of another shape: views crop it.
    Cropped(CoverFamily),
}

fn fit(field: &str, (w, h): (u32, u32)) -> Fit {
    match family_of(w, h) {
        f if f == shape_of(field) => Fit::Own,
        f => Fit::Cropped(f),
    }
}

fn low_resolution(field: &str, (w, h): (u32, u32)) -> bool {
    if field == "hero" {
        w < LOW_BACKGROUND
    } else {
        w.min(h) < LOW_COVER
    }
}

/// A badge under the details, in the colour of its `tone`.
fn badge(text: String, tone: Tone) -> Div {
    let (fg, bg): (Hsla, Hsla) = match tone {
        Tone::Good => (theme::success().into(), Hsla { a: 0.13, ..theme::success().into() }),
        Tone::Warn => (theme::warning().into(), Hsla { a: 0.14, ..theme::warning().into() }),
        Tone::Info => (theme::text_muted().into(), theme::ink_a(0.07)),
    };
    div().flex_none().px(px(7.)).py(px(2.)).rounded_full().bg(bg).text_size(px(11.)).text_color(fg).whitespace_nowrap().child(text)
}

#[derive(Clone, Copy)]
enum Tone {
    Good,
    Warn,
    Info,
}

impl MetadataDialog {
    /// The size read of `url`: `None` while unknown (not read, or not a
    /// picture whose size can be read).
    fn size_of(&self, url: &str) -> Option<(u32, u32)> {
        self.sizes.get(url).copied().flatten()
    }

    /// Reads the sizes of the pictures the preview has that are not read yet.
    pub(super) fn read_sizes(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &self.preview else { return };
        let mut urls: Vec<String> = Vec::new();
        for field in ["cover", "coverSquare", "coverWide", "hero"] {
            let values = p.candidates.iter().map(|c| super::format_value(Some(&c.metadata), field));
            for url in values.chain(std::iter::once(super::format_value(p.current.as_ref(), field))) {
                if !url.is_empty() && !self.sizes.contains_key(&url) && !urls.contains(&url) {
                    urls.push(url);
                }
            }
        }
        if urls.is_empty() {
            return;
        }
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let sizes = cx.background_spawn(async move { service.picture_sizes(&urls) }).await;
            this.update(cx, |this, cx| {
                this.sizes.extend(sizes);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A picture set by hand in Covers for `field`, which views show
    /// whatever is saved here.
    fn custom_art(&self, field: &str) -> bool {
        let Some(kind) = ArtKind::from_field(field) else { return false };
        let art = self.covers_art.as_ref().or(self.game.art.as_deref());
        art.and_then(|a| kind.of_art(a)).is_some_and(|u| !u.is_empty())
    }

    /// Who has a value: the sources' names, "Now" for what the game has alone.
    fn choice_name(&self, choice: &Choice) -> String {
        let names = choice.sources().map(|id| words::source_name(&self.service, id)).collect::<Vec<_>>().join(" + ");
        match (names.is_empty(), choice.keeps()) {
            (true, _) => tr!("Now").to_string(),
            (false, true) => tr!("{from} \u{b7} now", from = names),
            (false, false) => names,
        }
    }

    fn marks(&self, choice: &Choice, size: f32) -> Div {
        let mut row = div().flex().flex_none().items_center().gap(px(2.));
        for id in choice.sources() {
            row = row.child(labels::mark(id, &words::source_name(&self.service, id), size));
        }
        row
    }

    /// A value of `field` as a tile: its picture in the field's shape with
    /// the sources' marks over it ("Now" for what only the game has), a
    /// check when picked and a warning when its size says something is off.
    fn picture_tile(&mut self, field: &'static str, choice: &Choice, picked: bool, frozen: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let (w, h) = tile_size(field);
        let url = self.value_of(field, &choice.pick);
        let pic = self.thumb(&url, w, h, cx);
        let off = self.size_of(&url).is_some_and(|s| fit(field, s) != Fit::Own || low_resolution(field, s));
        let source = choice.pick.clone();
        let (hover_field, hover_source) = (field, source.clone());
        let accent: Hsla = theme::accent().into();
        let corner = |d: Div| d.absolute().top(px(5.)).size(px(14.)).rounded_full().flex().items_center().justify_center();
        let who = if choice.sources().next().is_some() {
            self.marks(choice, 13.)
        } else {
            div().px(px(5.)).rounded(px(4.)).text_size(px(10.)).text_color(theme::text_bright()).child(tr!("Now"))
        };
        div()
            .id(SharedString::from(format!("md-pic-{field}-{source}")))
            .relative()
            .flex_none()
            .p(px(3.))
            .rounded(px(8.))
            .border_1()
            .border_color(if picked { Hsla { a: 0.85, ..accent } } else { theme::ink_a(0.07) })
            .bg(if picked { theme::accent_alpha(0.12).into() } else { theme::ink_a(0.025) })
            .child(pic)
            .child(div().absolute().left(px(6.)).bottom(px(6.)).p(px(1.)).rounded(px(5.)).bg(theme::recess_a(0.7)).child(who))
            .when(picked, |d| d.child(corner(div()).right(px(5.)).bg(accent).child(icon(Icon::Check, px(10.), theme::on_accent()))))
            .when(off, |d| d.child(corner(div()).left(px(5.)).bg(theme::warning()).child(icon(Icon::Warning, px(9.), theme::on_accent()))))
            .when(frozen, |d| d.opacity(0.4))
            .when(!frozen, |d| {
                d.cursor_pointer()
                    .when(!picked, |d| d.hover(|s| s.border_color(theme::ink_a(0.3))))
                    .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                        let me = (hover_field, hover_source.clone());
                        if *over {
                            this.hover = Some(me);
                        } else if this.hover.as_ref() == Some(&me) {
                            this.hover = None;
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_pick(field, &source);
                        cx.notify();
                    }))
            })
    }

    /// The details of the value shown (the tile under the pointer, else the
    /// pick): who has it, its size, and the badges.
    fn picture_details(&self, field: &'static str, choices: &[Choice]) -> Div {
        let col = div().flex().flex_col().gap(px(4.)).w(px(DETAILS_W)).flex_none().min_w_0();
        let shown = match &self.hover {
            Some((f, s)) if *f == field => s.clone(),
            _ => self.pick(field),
        };
        let Some(choice) = choices.iter().find(|c| c.ids.contains(&shown) || c.pick == shown) else {
            // No source, or keeping what the game has, which is nothing (a
            // locked field, or one the merge leaves as it is).
            let text = if choices.is_empty() { tr!("No source has it") } else { tr!("Empty") };
            return col.child(div().text_size(px(12.)).text_color(theme::text_dim()).child(text));
        };
        let url = self.value_of(field, &choice.pick);
        let size = self.size_of(&url);
        let mut badges = div().flex().flex_wrap().gap(px(4.));
        if let Some(s) = size {
            badges = badges.child(match fit(field, s) {
                Fit::Own => badge(tr!("Own shape").to_string(), Tone::Good),
                Fit::Cropped(CoverFamily::Square) => badge(tr!("Cropped: the picture is square").to_string(), Tone::Warn),
                Fit::Cropped(CoverFamily::Wide) => badge(tr!("Cropped: the picture is wide").to_string(), Tone::Warn),
                Fit::Cropped(CoverFamily::Tall) => badge(tr!("Cropped: the picture is tall").to_string(), Tone::Warn),
            });
            if low_resolution(field, s) {
                badges = badges.child(badge(tr!("Low resolution").to_string(), Tone::Warn));
            }
        }
        if self.custom_art(field) {
            badges = badges.child(
                badge(tr!("Set in Covers").to_string(), Tone::Info)
                    .id(SharedString::from(format!("md-custom-{field}")))
                    .tooltip(crate::ui::tooltip(tr!("Your picture from Covers is shown instead"))),
            );
        }
        let size_text = match (size, self.sizes.contains_key(&url)) {
            (Some((w, h)), _) => Some(format!("{w}\u{d7}{h}")),
            (None, false) => Some("\u{2026}".to_string()),
            (None, true) => None,
        };
        col.child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .min_w_0()
                .child(self.marks(choice, 15.))
                .child(div().min_w_0().truncate().text_size(px(12.5)).text_color(theme::text_bright()).child(self.choice_name(choice)))
                .children(size_text.map(|s| div().flex_none().text_size(px(12.)).text_color(theme::text_muted()).child(s))),
        )
        .child(badges)
    }

    /// The scroll of `field`'s tiles. A new one knows how far it can move
    /// only once drawn: the next frame draws its arrows.
    fn tile_scroll(&mut self, field: &'static str, window: &mut Window, cx: &mut Context<Self>) -> ScrollHandle {
        if let Some(s) = self.tile_scrolls.get(field) {
            return s.clone();
        }
        let s = ScrollHandle::new();
        self.tile_scrolls.insert(field, s.clone());
        cx.on_next_frame(window, |_, _, cx| cx.notify());
        s
    }

    /// A picture's row: its name and "More options…" (the Covers dialog on
    /// its slot: every region's box, the image search, a file), the tiles,
    /// the details and the lock.
    pub(super) fn picture_row(&mut self, field: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let choices = self.choices(field);
        let picked = self.pick(field);
        let frozen = self.frozen(field);
        // One row: the tiles that do not fit scroll sideways, with an arrow
        // at each side that has more.
        let scroll = self.tile_scroll(field, window, cx);
        let mut tiles =
            div().id(SharedString::from(format!("md-tiles-{field}"))).flex().w_full().gap(px(5.)).overflow_x_scroll().track_scroll(&scroll);
        for choice in &choices {
            let on = choice.ids.contains(&picked);
            tiles = tiles.child(self.picture_tile(field, choice, on, frozen, cx));
        }
        if choices.is_empty() {
            tiles = tiles.child(div().py(px(6.)).text_size(px(12.5)).text_color(theme::text_dim()).child(tr!("Empty")));
        }
        let slot = Slot::of_field(field);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .py(px(6.))
            .px(px(14.))
            .border_b_1()
            .border_color(theme::row_separator())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(LABEL_W))
                    .flex_none()
                    .gap(px(2.))
                    .child(div().truncate().text_size(px(12.5)).text_color(theme::text_muted()).child(field_label(field)))
                    .when_some(slot, |d, slot| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("md-choose-covers-{field}")))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(theme::accent())
                                .cursor_pointer()
                                .hover(|s| s.text_color(theme::text_bright()))
                                .child(tr!("More options\u{2026}"))
                                .on_click(cx.listener(move |this, _, window, cx| this.open_covers(slot, window, cx))),
                        )
                    }),
            )
            .child(div().relative().flex_1().min_w_0().child(tiles).children(scroll_arrows(field, &scroll)))
            .child(self.picture_details(field, &choices))
            .child(self.lock_button(field, cx))
    }

    /// The fields drawn as pictures, in the order shown.
    pub(super) fn picture_fields(fields: &[&'static str]) -> Vec<&'static str> {
        fields.iter().copied().filter(|f| is_picture(f)).collect()
    }
}

/// The ‹ › over the ends of a row of tiles wider than its room: each
/// moves it by most of what is in view, and shows only while there is more
/// that way (known from the last frame).
fn scroll_arrows(field: &'static str, scroll: &ScrollHandle) -> Vec<Stateful<Div>> {
    let max = f32::from(scroll.max_offset().x);
    if max <= 0.5 {
        return Vec::new();
    }
    let at = -f32::from(scroll.offset().x);
    let step = (f32::from(scroll.bounds().size.width) * 0.8).max(60.);
    let arrow = |side: &str, ic: Icon, delta: f32| {
        let scroll = scroll.clone();
        // Over the pictures: the dark disc of the round buttons, nearly
        // opaque, with a light ring and a shadow so it holds on any cover.
        div()
            .id(SharedString::from(format!("md-tiles-{field}-{side}")))
            .absolute()
            .top(px((TILE_H + 8. - ARROW) / 2.))
            .size(px(ARROW))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .bg(theme::round_fill(0.95))
            .border_1()
            .border_color(theme::ink_a(0.28))
            .shadow(vec![BoxShadow {
                color: theme::shadow_a(0.5),
                offset: point(px(0.), px(1.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
                inset: false,
            }])
            .hover(|s| s.bg(theme::round_fill_hover()).border_color(theme::ink_a(0.45)))
            .child(icon(ic, px(16.), theme::round_icon()))
            .on_click(move |_, window, _| {
                let offset = scroll.offset();
                let x = (f32::from(offset.x) - delta).clamp(-max, 0.);
                scroll.set_offset(point(px(x), offset.y));
                window.refresh();
            })
    };
    let mut arrows = Vec::new();
    if at > 0.5 {
        arrows.push(arrow("prev", Icon::ChevronLeft, -step).left(px(4.)));
    }
    if at < max - 0.5 {
        arrows.push(arrow("next", Icon::ChevronRight, step).right(px(4.)));
    }
    arrows
}
