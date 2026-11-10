//! "Cover shapes": a row per cover family (its outline, the aspects it
//! serves, who draws it, how many games have a picture of their own and how
//! many show one cropped, "In use" or a switch to keep it), the switch that
//! downloads a family as soon as a view starts using it, and, for a family
//! in use that games show cropped, "Download <family> covers" (the whole
//! Collection, through the metadata hub: the header pill follows it).
//!
//! The counts are read off the UI thread with the summary's (`summary::count`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::art::{self, ArtContext};
use heronite_data::cover_family::{CoverFamilies, CoverFamily};
use heronite_data::{DataPaths, ImageRef, Library};
use heronite_i18n::tr;
use heronite_metadata::service::Attention;
use heronite_metadata::MetadataService;

use super::super::SettingsPage;
use super::persist;
use super::sources::caption_hint;
use crate::dialogs::covers::words;
use crate::dialogs::metadata_wizard::{hub, HubGlobal};
use crate::pages::status_pill;
use crate::theme;
use crate::ui::controls::{action_button, card_box, row, toggle, toggle_row, ButtonKind};

/// The families and how many games have each, once counted.
#[derive(Clone, Debug)]
pub(super) struct Shapes {
    pub families: CoverFamilies,
    /// Per family (in [`CoverFamily::ALL`] order): the games with a picture
    /// of their own (for the tall one: any cover).
    pub real: [usize; 3],
    /// Per family: a source that gives it can be asked now.
    pub ready: [bool; 3],
    pub total: usize,
}

fn index(f: CoverFamily) -> usize {
    CoverFamily::ALL.iter().position(|x| *x == f).unwrap_or(0)
}

impl Shapes {
    /// Off the UI thread: it checks the games' local pictures.
    pub fn count(lib: &Library, paths: &DataPaths, service: &MetadataService) -> Self {
        let shots = lib.settings.screenshots_folder();
        let ctx = ArtContext::new(paths, shots);
        let mut real = [0; 3];
        for g in &lib.games {
            if art::card_cover(&ctx, g).iter().any(|r| *r != ImageRef::Fallback) {
                real[0] += 1;
            }
            for f in CoverFamily::EXTRA {
                if !art::family_art(&ctx, g, f).is_empty() {
                    real[index(f)] += 1;
                }
            }
        }
        let ready = CoverFamily::ALL.map(|f| f != CoverFamily::Tall && service.can_download_family(f));
        Self { families: service.cover_families(), real, ready, total: lib.games.len() }
    }
}

/// The games of the Collection that show `family` cropped.
fn cropped(attention: Option<&Attention>, family: CoverFamily) -> usize {
    attention.and_then(|a| a.cropped.get(&family)).map_or(0, |s| s.len())
}

/// The outline of a family's shape.
fn outline(f: CoverFamily) -> Div {
    let (w, h) = match f {
        CoverFamily::Tall => (20., 30.),
        CoverFamily::Square => (26., 26.),
        CoverFamily::Wide => (36., 20.),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(40.))
        .child(div().w(px(w)).h(px(h)).rounded(px(3.)).border_2().border_color(theme::ink_a(0.45)))
}

/// Keeps `family` or stops keeping it (`keepCoverFamilies`).
fn set_kept(d: &mut SettingsPage, family: CoverFamily, keep: bool, cx: &mut Context<SettingsPage>) {
    let res = heronite_data::settings::set_cover_family_kept(&d.ctx.data_paths, family, keep);
    if res.is_ok() {
        if let Some(s) = d.metadata.shapes.as_mut() {
            s.families.states.iter_mut().filter(|s| s.family == family).for_each(|s| s.kept_by_choice = keep);
        }
    }
    d.saved(res, cx);
}

/// "Download <family> covers" for the whole Collection.
fn download(family: CoverFamily, cx: &mut App) {
    if cx.has_global::<HubGlobal>() {
        hub(cx).update(cx, |h, cx| h.download_family(family, None, cx));
    }
}

fn family_row(d: &SettingsPage, s: &Shapes, f: CoverFamily, cx: &mut Context<SettingsPage>) -> Div {
    let state = s.families.state(f);
    let (in_use, kept) = (state.in_use(), state.kept());
    let real = s.real[index(f)];
    let crop = if in_use { cropped(d.metadata.attention.as_ref(), f) } else { 0 };
    let count = if in_use && f != CoverFamily::Tall {
        tr!("{real} with their own \u{b7} {cropped} cropped", real, cropped = crop)
    } else {
        tr!("{real} with their own", real)
    };
    let total = s.total.max(1) as f32;
    let bar = div()
        .flex()
        .w(px(150.))
        .h(px(4.))
        .rounded_full()
        .overflow_hidden()
        .bg(theme::ink_a(0.08))
        .child(div().h_full().w(relative(real as f32 / total)).bg(theme::accent()))
        .child(div().h_full().w(relative(crop as f32 / total)).bg(theme::warning()));
    let switch = toggle(SharedString::from(format!("md-shape-{}", f.id())), kept, in_use);
    let switch = if in_use { switch } else { switch.on_click(cx.listener(move |d, _, _, cx| set_kept(d, f, !kept, cx))) };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(12.))
        .child(outline(f))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(words::family_name(f)))
                        .child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(words::aspects(f))),
                )
                .child(div().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(words::users(state))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(170.))
                .gap(px(5.))
                .when(kept, |d| d.child(div().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(count)).child(bar))
                .when(!kept, |d| d.child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Not kept")))),
        )
        .child(div().w(px(62.)).flex_none().flex().justify_end().when(in_use, |d| d.child(status_pill(tr!("In use"), theme::accent()))))
        .child(switch)
}

pub(super) fn card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let s = d.metadata.shapes.clone()?;
    let fill = d.settings.metadata.fill_cover_families;
    let mut card = card_box();
    for (i, f) in CoverFamily::ALL.into_iter().enumerate() {
        if i > 0 {
            card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
        }
        card = card.child(family_row(d, &s, f, cx));
    }
    card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(
        toggle_row(
            "md-fill-shapes",
            tr!("Download a shape as soon as a view starts using it"),
            Some(tr!("Picking a square aspect, or a Console mode theme with square or wide tiles, fetches those covers by itself.")),
            fill,
        )
        .on_click(cx.listener(move |d, _, _, cx| {
            let res = heronite_data::settings::set_fill_cover_families(&d.ctx.data_paths, !fill);
            d.saved(res, cx);
        })),
    );
    for f in CoverFamily::EXTRA.into_iter().filter(|f| s.families.is_in_use(*f)) {
        let n = cropped(d.metadata.attention.as_ref(), f);
        if n == 0 {
            continue;
        }
        let ready = s.ready[index(f)];
        card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(row(
            words::cropped(f, n),
            (!ready).then(|| words::no_source(f)),
            action_button(
                SharedString::from(format!("md-shape-dl-{}", f.id())),
                ButtonKind::Primary,
                words::download(f),
                ready,
                move |_, _, cx| download(f, cx),
            ),
        ));
    }
    Some(div().flex().flex_col().child(caption_hint(tr!("Cover shapes"), Some(tr!("A shape in use is always kept")))).child(card))
}

/// Console mode's rows: a switch per family (locked while in use), the
/// automatic download and the downloads of the families shown cropped.
pub(super) fn console_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let Some(s) = &d.metadata.shapes else { return Vec::new() };
    let mut rows = vec![Row::header(tr!("Cover shapes"))];
    for f in CoverFamily::ALL {
        let state = s.families.state(f);
        let desc = format!("{} \u{b7} {}", words::aspects(f), words::users(state));
        rows.push(Row::toggle(format!("shape:{}", f.id()), words::family_name(f), state.kept()).desc(desc).enabled(!state.in_use()));
    }
    rows.push(Row::toggle(
        "fill-shapes",
        tr!("Download a shape as soon as a view starts using it"),
        d.settings.metadata.fill_cover_families,
    ));
    for f in CoverFamily::EXTRA.into_iter().filter(|f| s.families.is_in_use(*f)) {
        let n = cropped(d.metadata.attention.as_ref(), f);
        if n > 0 {
            rows.push(
                Row::action(format!("shape-dl:{}", f.id()), words::download(f)).desc(words::cropped(f, n)).enabled(s.ready[index(f)]),
            );
        }
    }
    rows
}

/// A console row of [`console_rows`] changed; false when it is not one of them.
pub(super) fn console_apply(d: &mut SettingsPage, id: &str, on: Option<bool>, cx: &mut Context<SettingsPage>) -> bool {
    if id == "fill-shapes" {
        if let Some(on) = on {
            persist(d, cx, |m| m.fill_cover_families = on);
        }
        return true;
    }
    if let Some(f) = id.strip_prefix("shape:").and_then(CoverFamily::parse) {
        if let Some(on) = on {
            set_kept(d, f, on, cx);
        }
        return true;
    }
    if let Some(f) = id.strip_prefix("shape-dl:").and_then(CoverFamily::parse) {
        download(f, cx);
        return true;
    }
    false
}
