//! The sources as rows, in three captioned cards by kind (Game details,
//! Pictures, Extras). A closed row: drag handle (details only), tile, name and
//! what it gives, the games it is for, its status, its switch and a chevron.
//! A click on the row opens it in place (`source_body.rs`); several can be
//! open at once. Dragging a details row moves it among them.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::sources::{SourceKind, SourceRow, SourceStatus};

use super::super::SettingsPage;
use super::{labels, move_source, set_enabled, source_body, toggle_open};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{caption, card_box, toggle, toggle_inactive, CAPTION_H};
use crate::ui::icon;

/// The width of the games a source is for, and of its status, so the
/// columns line up from row to row.
const AUDIENCE_W: f32 = 160.;
const STATUS_W: f32 = 132.;

/// A details row being dragged to another place.
pub(super) struct SourceDrag {
    id: String,
    name: SharedString,
}

/// What follows the pointer while a row or a chip is dragged.
pub(super) struct DragGhost(pub SharedString);

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::glass::glass_menu_surface(theme::R_ITEM)
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .text_size(px(13.))
            .text_color(theme::text_primary())
            .child(icon(Icon::DragIndicator, px(16.), theme::text_dim()))
            .child(self.0.clone())
    }
}

/// The caption of a card, with a dim hint beside it.
pub(super) fn caption_hint(text: &str, hint: Option<&str>) -> Div {
    div().flex().items_end().gap(px(4.)).child(caption(text.to_string())).children(
        hint.map(|h| div().mb(px(6.)).text_size(px(12.)).line_height(px(CAPTION_H)).text_color(theme::text_dim()).child(h.to_string())),
    )
}

/// The three cards, each with its caption; a kind with no source is left out.
pub(super) fn cards(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Vec<AnyElement> {
    let kinds = [(SourceKind::Details, tr!("Game details")), (SourceKind::Pictures, tr!("Pictures")), (SourceKind::Extras, tr!("Extras"))];
    let mut out = Vec::new();
    for (kind, title) in kinds {
        let rows: Vec<&SourceRow> = d.metadata.rows.iter().filter(|r| r.descriptor.kind == kind).collect();
        if rows.is_empty() {
            continue;
        }
        let movable = rows.iter().filter(|r| r.reorderable).count() > 1;
        let hint = movable.then(|| tr!("Drag to change the order"));
        let mut card = card_box();
        for (i, r) in rows.iter().enumerate() {
            if i > 0 {
                card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
            }
            let open = d.metadata.open.contains(&r.descriptor.id);
            card = card.child(row(r, i, movable && r.reorderable, open, cx));
            if open {
                let indent = if movable { 18. + 8. + labels::TILE + 12. } else { labels::TILE + 12. };
                card = card.child(source_body::body(d, r, indent, cx));
            }
        }
        out.push(div().flex().flex_col().child(caption_hint(title, hint)).child(card).into_any_element());
    }
    out
}

/// A closed row; `place` is its index among the rows of its kind.
fn row(r: &SourceRow, place: usize, movable: bool, open: bool, cx: &mut Context<SettingsPage>) -> Stateful<Div> {
    let id = r.descriptor.id.clone();
    let name = labels::name(&r.descriptor.name).to_string();
    let dim = |el: Div| if r.enabled { el } else { el.opacity(0.5) };

    let what = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .min_w_0()
                .child(
                    div()
                        .relative()
                        .min_w_0()
                        .children(crate::ui::find::flash(&name, 8.))
                        .text_size(px(14.))
                        .text_color(theme::text_primary())
                        .truncate()
                        .child(name.clone()),
                )
                .children(super::extension::pill(r)),
        )
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(labels::gives(&r.descriptor.provides)));
    let audience =
        div().flex_none().w(px(AUDIENCE_W)).text_size(px(12.)).text_color(theme::text_dim()).truncate().child(labels::row_audience(r));
    let status = div().flex_none().w(px(STATUS_W)).child(labels::status(r.status));

    let sw_id = SharedString::from(format!("md-src-on-{id}"));
    let switch = match r.status {
        SourceStatus::Unavailable => toggle(sw_id, r.enabled, true),
        s if r.enabled && labels::waiting(s) => toggle_inactive(sw_id, true),
        _ => toggle(sw_id, r.enabled, false),
    };
    let switch = if r.status == SourceStatus::Unavailable {
        switch
    } else {
        let (sid, on) = (id.clone(), r.enabled);
        switch.on_click(cx.listener(move |d, _, _, cx| {
            // The switch, not the row: the row stays as it is.
            cx.stop_propagation();
            set_enabled(d, &sid, !on, cx);
        }))
    };
    let chevron =
        icon(Icon::ExpandMore, px(20.), theme::text_muted()).when(open, |i| i.with_transformation(Transformation::rotate(percentage(0.5))));

    let open_id = id.clone();
    let line = div()
        .id(SharedString::from(format!("md-src-{id}")))
        .flex()
        .items_center()
        .gap(px(12.))
        .mx(px(-8.))
        .px(px(8.))
        .py(px(10.))
        .rounded(px(theme::R_ITEM))
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink_a(0.04)))
        .on_click(cx.listener(move |d, _, _, cx| toggle_open(d, &open_id, cx)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .when(movable, |d| d.child(icon(Icon::DragIndicator, px(18.), theme::ink_a(0.3))))
                .child(dim(labels::tile(&id, &name))),
        )
        .child(dim(what))
        .child(dim(audience))
        .child(dim(status))
        .child(switch)
        .child(chevron);
    if !movable {
        return line;
    }
    let drop_id = id.clone();
    line.on_drag(SourceDrag { id, name: name.into() }, |d, _, _, cx| cx.new(|_| DragGhost(d.name.clone())))
        .drag_over::<SourceDrag>(|s, _, _, _| s.bg(theme::accent_alpha(0.1)))
        .on_drop(cx.listener(move |d, drag: &SourceDrag, _, cx| {
            if drag.id != drop_id {
                move_source(d, &drag.id, place, cx);
            }
        }))
}
