//! The left column: a slot per cover family and the background, each with
//! its picture in its true shape, where it comes from, who draws it and a
//! dot. Built again only when the pictures or the families change, never
//! per frame (finding the picture checks local files).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::art::CoverOrigin;
use heronite_data::cover_family::CoverFamily;
use heronite_i18n::tr;
use heronite_images::ImageRef;
use heronite_metadata::ArtKind;

use super::{words, Busy, CoversDialog};
use crate::dialogs::art_images::{picture, GameChains};
use crate::dialogs::metadata_update::words::source_name;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// A picture of the game the dialog changes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Slot {
    Cover(CoverFamily),
    Background,
}

impl Slot {
    pub const ALL: [Slot; 4] =
        [Slot::Cover(CoverFamily::Tall), Slot::Cover(CoverFamily::Square), Slot::Cover(CoverFamily::Wide), Slot::Background];

    pub fn kind(self) -> ArtKind {
        match self {
            Slot::Cover(f) => ArtKind::cover(f),
            Slot::Background => ArtKind::Hero,
        }
    }

    pub fn parse(id: &str) -> Option<Slot> {
        match id {
            "background" | "hero" => Some(Slot::Background),
            other => CoverFamily::parse(other).map(Slot::Cover),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Slot::Cover(f) => words::family_name(f),
            Slot::Background => tr!("Background"),
        }
    }

    /// Width / height of its pictures (a background as the stage shows it).
    pub fn aspect(self) -> f32 {
        match self {
            Slot::Cover(f) => f.aspect(),
            Slot::Background => 840. / 320.,
        }
    }
}

/// How a slot's picture stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Look {
    /// A picture of its own.
    Own,
    /// Cropped from another picture.
    Cropped(CoverOrigin),
    /// No picture at all.
    Missing,
    /// The family is not kept here.
    NotKept,
}

pub(super) struct SlotInfo {
    pub slot: Slot,
    /// What the slot shows now (empty: nothing).
    pub chain: Vec<ImageRef>,
    pub look: Look,
    /// Where the picture comes from ("IGDB", "Your own picture"), when it
    /// is the slot's own.
    pub from: Option<String>,
}

impl CoversDialog {
    /// The slots as the pictures and families are now.
    pub(super) fn compute_slots(&mut self) {
        let chains = GameChains::of(&self.ctx, &self.game, &self.art);
        let md = self.game.metadata.clone();
        let source = |field: &str| md.as_deref().and_then(|m| m.field_sources.get(field)).map(String::as_str);
        self.slots = Slot::ALL
            .into_iter()
            .map(|slot| {
                let custom = slot.kind().of_art(&self.art).is_some_and(|u| !u.is_empty());
                let fetched = md.as_deref().and_then(|m| slot.kind().of_record(m)).is_some_and(|u| !u.is_empty());
                let (chain, origin) = match slot {
                    Slot::Cover(f) => chains.family(f),
                    Slot::Background => (chains.stage.clone(), CoverOrigin::Own),
                };
                let look = match slot {
                    Slot::Cover(f) if !self.families.is_kept(f) => Look::NotKept,
                    _ if chain.is_empty() => Look::Missing,
                    _ if origin.is_cropped() => Look::Cropped(origin),
                    _ => Look::Own,
                };
                let from = match () {
                    _ if look != Look::Own => None,
                    _ if custom => Some(tr!("Your own picture").to_string()),
                    _ => match source(slot.kind().as_str()).filter(|_| fetched) {
                        Some("manual") => Some(tr!("Your own picture").to_string()),
                        Some(id) => Some(source_name(&self.service, id)),
                        None => Some(tr!("From the store").to_string()),
                    },
                };
                SlotInfo { slot, chain, look, from }
            })
            .collect();
    }

    pub(super) fn info(&self, slot: Slot) -> Option<&SlotInfo> {
        self.slots.iter().find(|s| s.slot == slot)
    }

    /// The muted line of a slot: where its picture comes from, then who
    /// draws it.
    fn slot_line(&self, info: &SlotInfo) -> String {
        let what = match info.look {
            Look::NotKept => return tr!("Not kept on this PC").to_string(),
            Look::Missing => tr!("No picture").to_string(),
            Look::Cropped(CoverOrigin::Background) => tr!("Cropped from the background").to_string(),
            Look::Cropped(_) => tr!("Cropped from the tall cover").to_string(),
            Look::Own => info.from.clone().unwrap_or_default(),
        };
        let used = match info.slot {
            Slot::Cover(f) => {
                let state = self.families.state(f);
                Some(words::used_where(state).unwrap_or(tr!("kept, not shown now")))
            }
            Slot::Background => None,
        };
        [Some(what), used.map(str::to_string)].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ")
    }

    /// A picture of `chain` in a `w`×`h` box, or a placeholder.
    pub(super) fn picture_box(&mut self, id: &str, chain: &[ImageRef], w: f32, h: f32, cx: &mut Context<Self>) -> Div {
        if chain.is_empty() {
            return div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(w))
                .h(px(h))
                .rounded(px(6.))
                .border_1()
                .border_color(theme::ink_a(0.12))
                .bg(theme::ink_a(0.03))
                .child(icon(Icon::Image, px(16.), theme::text_dim()));
        }
        let key = format!("cv-{id}:{:?}", chain[0]);
        let src = self.images.update(cx, |im, _| im.get(&key, chain.to_vec(), w, h));
        picture(src).flex_none().w(px(w)).h(px(h)).rounded(px(6.))
    }

    fn slot_row(&mut self, info_index: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let (slot, chain, look) = {
            let info = &self.slots[info_index];
            (info.slot, info.chain.clone(), info.look)
        };
        let line = self.slot_line(&self.slots[info_index]);
        let on = self.slot == slot;
        let h = 46.;
        let w = (h * slot.aspect()).min(84.);
        let thumb =
            if look == Look::NotKept { self.picture_box("none", &[], w, h, cx) } else { self.picture_box("slot", &chain, w, h, cx) };
        let dot: Hsla = match look {
            Look::Own => theme::success().into(),
            Look::Cropped(_) => theme::warning().into(),
            Look::Missing | Look::NotKept => theme::ink_a(0.25),
        };
        let keep = (look == Look::NotKept).then(|| {
            let Slot::Cover(family) = slot else { unreachable!("only a family can be not kept") };
            div()
                .id(SharedString::from(format!("cv-keep-{}", family.id())))
                .text_size(px(12.))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|s| s.text_color(theme::text_bright()))
                .child(if self.busy == Some(Busy::Keeping) { tr!("Saving\u{2026}") } else { tr!("Keep this shape") })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.keep_family(family, cx);
                }))
        });
        div()
            .id(SharedString::from(format!("cv-slot-{info_index}")))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(theme::R_TILE))
            .border_1()
            .cursor_pointer()
            .when(on, |d| d.bg(theme::accent_tint()).border_color(theme::accent_tint_border()))
            .when(!on, |d| d.bg(theme::card_fill()).border_color(theme::card_border()).hover(|s| s.bg(theme::ink_a(0.06))))
            .child(div().w(px(84.)).flex_none().flex().justify_center().child(thumb))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(slot.name()))
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(line))
                    .children(keep),
            )
            .child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
            .on_click(cx.listener(move |this, _, _, cx| this.pick_slot(slot, cx)))
    }

    pub(super) fn render_slots(&mut self, cx: &mut Context<Self>) -> Div {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("This game\u{2019}s pictures")));
        for i in 0..self.slots.len() {
            col = col.child(self.slot_row(i, cx));
        }
        col
    }
}
