//! "Order for one field (advanced)", closed until clicked: a line per field
//! with its sources as chips, first to last. Dragging a chip onto another
//! puts it in that place for this field only; "Reset" gives the field back
//! to the order of the rows.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::SettingsPage;
use super::sources::DragGhost;
use super::{apply, labels, FieldLine};
use crate::dialogs::metadata::field_label;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, card_box, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::icon;

/// Width of the fields' names, so the chips start in one column.
const LABEL_W: f32 = 120.;

/// A source chip being dragged within its field's line.
struct ChipDrag {
    field: &'static str,
    id: String,
    name: SharedString,
}

/// The order `list` takes once `id` is put at place `to`.
fn moved(list: &[String], id: &str, to: usize) -> Vec<String> {
    let mut out: Vec<String> = list.iter().filter(|s| *s != id).cloned().collect();
    out.insert(to.min(out.len()), id.to_string());
    out
}

pub(super) fn block(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let open = d.metadata.advanced_open;
    let title = tr!("Order for one field (advanced)");
    let head = div()
        .id("md-advanced")
        .relative()
        .children(crate::ui::find::flash(title, 8.))
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(px(14.))
        .mx(px(6.))
        .mb(px(6.))
        .cursor_pointer()
        .on_click(cx.listener(|d, _, _, cx| {
            d.metadata.advanced_open = !d.metadata.advanced_open;
            d.metadata.open_select = None;
            cx.notify();
        }))
        .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::text_muted()))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(title));
    let col = div().flex().flex_col().child(head);
    if !open {
        return col;
    }
    let mut card = card_box().py(px(6.));
    for (i, line) in d.metadata.fields.iter().enumerate() {
        if i > 0 {
            card = card.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
        }
        card = card.child(field_line(d, line, cx));
    }
    col.child(
        div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "Each field asks its sources from left to right. Drag a source to change the order for that field only."
        ))),
    )
    .child(card)
}

fn source_name(d: &SettingsPage, id: &str) -> String {
    d.metadata.rows.iter().find(|r| r.descriptor.id == id).map_or_else(|| id.to_string(), |r| labels::name(&r.descriptor.name).to_string())
}

fn field_line(d: &SettingsPage, line: &FieldLine, cx: &mut Context<SettingsPage>) -> Div {
    let field = line.field;
    let movable = line.sources.len() > 1;
    let mut chips = div().flex().flex_1().min_w_0().flex_wrap().items_center().gap(px(4.));
    if line.sources.is_empty() {
        chips = chips.child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("No source is on for it")));
    }
    for (place, id) in line.sources.iter().enumerate() {
        if place > 0 {
            chips = chips.child(icon(Icon::ChevronRight, px(14.), theme::text_dim()));
        }
        let name = source_name(d, id);
        let chip = div()
            .id(SharedString::from(format!("md-order-{field}-{id}")))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(9.))
            .py(px(3.))
            .rounded_full()
            .bg(if place == 0 { theme::accent_alpha(0.12).into() } else { theme::ink_a(0.06) })
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(if place == 0 { theme::text_primary() } else { theme::text_muted() })
            .when(movable, |c| c.cursor_grab().child(icon(Icon::DragIndicator, px(12.), theme::ink_a(0.35))))
            .child(name.clone());
        let chip = if movable {
            let (list, target) = (line.sources.clone(), id.clone());
            chip.on_drag(ChipDrag { field, id: id.clone(), name: name.into() }, |c, _, _, cx| cx.new(|_| DragGhost(c.name.clone())))
                .drag_over::<ChipDrag>(move |s, c, _, _| if c.field == field { s.bg(theme::accent_alpha(0.22)) } else { s })
                .on_drop(cx.listener(move |d, c: &ChipDrag, _, cx| {
                    if c.field == field && c.id != target {
                        let order = moved(&list, &c.id, place);
                        apply(d, cx, |s| s.set_field_order(field, &order));
                    }
                }))
        } else {
            chip
        };
        chips = chips.child(chip);
    }
    let end = if line.customised {
        button_sm(SharedString::from(format!("md-order-reset-{field}")), ButtonKind::Ghost, tr!("Reset"))
            .on_click(cx.listener(move |d, _, _, cx| apply(d, cx, |s| s.reset_field_order(field))))
            .into_any_element()
    } else {
        div().px(px(12.)).text_size(px(12.)).text_color(theme::text_dim()).child(tr!("List order")).into_any_element()
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(9.))
        .child(div().flex_none().w(px(LABEL_W)).text_size(px(13.)).text_color(theme::text_primary()).truncate().child(field_label(field)))
        .child(chips)
        .child(div().flex_none().child(end))
}

#[cfg(test)]
mod tests {
    use super::moved;

    #[test]
    fn a_chip_dropped_on_another_takes_its_place() {
        let list: Vec<String> = ["igdb", "steam", "store", "lutris"].iter().map(|s| s.to_string()).collect();
        assert_eq!(moved(&list, "lutris", 0), ["lutris", "igdb", "steam", "store"]);
        assert_eq!(moved(&list, "igdb", 2), ["steam", "store", "igdb", "lutris"]);
        assert_eq!(moved(&list, "steam", 9), ["igdb", "store", "lutris", "steam"]);
    }
}
