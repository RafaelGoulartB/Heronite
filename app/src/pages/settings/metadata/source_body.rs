//! An open source row: a line about it, its database when it downloads one
//! (`download.rs`), its options as the descriptor lists them (text and
//! secret inputs, selects, switches, with their help and link), "Test"
//! where it has one, and "Fills": a chip per field, which a click switches
//! off for this source where the merge honours it.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::sources::{OptionKind, OptionRow, OptionValue, SourceRow};

use super::super::SettingsPage;
use super::{download, extension, labels, set_option, test};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, toggle_row, ButtonKind};
use crate::ui::fields::{field_label, input_box, labeled, meta_text, select_field};
use crate::ui::icon;

/// The narrowest a text field or a select gets before the next one wraps.
const FIELD_MIN_W: f32 = 220.;
/// The widest a text field gets, so a lone key does not span the card.
const FIELD_MAX_W: f32 = 420.;

pub(super) fn body(d: &SettingsPage, r: &SourceRow, indent: f32, cx: &mut Context<SettingsPage>) -> Div {
    let id = r.descriptor.id.as_str();
    let mut col = div().flex().flex_col().gap(px(14.)).pl(px(indent)).pt(px(2.)).pb(px(16.));
    let about =
        [labels::about(&r.descriptor).map(str::to_owned), labels::when_asked(&r.descriptor)].into_iter().flatten().collect::<Vec<_>>();
    if !about.is_empty() {
        col = col.child(div().flex().flex_col().gap(px(4.)).children(about.into_iter().map(meta_text)));
    }
    col = col.children(extension::problem_block(r, cx));
    if let Some(row) = &r.download {
        col = col.child(download::block(id, row, cx));
    }

    let mut fields = div().flex().flex_wrap().gap_x(px(12.)).gap_y(px(12.));
    let mut switches = Vec::new();
    for o in &r.options {
        match o.option.kind {
            OptionKind::Text | OptionKind::Secret => fields = fields.child(text_option(d, id, o)),
            OptionKind::Select => fields = fields.child(select_option(d, id, o, cx)),
            OptionKind::Switch => switches.push(switch_option(id, o, cx)),
        }
    }
    if r.options.iter().any(|o| o.option.kind != OptionKind::Switch) {
        col = col.child(fields);
    }
    col = col.children(switches);
    if r.can_test {
        col = col.child(test_line(d, id, cx));
    }
    let extension = extension::block(d, r, cx);
    // An extension that could not be read has no field to list.
    col.when(!r.fields.is_empty(), |c| c.child(fills(r, cx))).children(extension)
}

/// A label, a link under it when the descriptor gives one (its help as the
/// link's text), else the help as a dim line.
fn help(o: &OptionRow) -> Option<AnyElement> {
    let text = o.option.help.as_deref().map(|h| tr(h).to_string());
    match (o.option.link.clone(), text) {
        (Some(link), text) => {
            let text = text.unwrap_or_else(|| link.clone());
            Some(
                div()
                    .id(SharedString::from(format!("md-link-{}", o.option.key)))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .mt(px(6.))
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(theme::readable(theme::accent()))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.8))
                    .on_click(move |_, _, _| crate::ui::open_external(&link))
                    .child(div().min_w_0().child(text))
                    .child(icon(Icon::OpenInNew, px(13.), theme::readable(theme::accent())))
                    .into_any_element(),
            )
        }
        (None, Some(text)) => Some(meta_text(text).mt(px(6.)).into_any_element()),
        (None, None) => None,
    }
}

fn text_option(d: &SettingsPage, id: &str, o: &OptionRow) -> Div {
    let input = d.metadata.inputs.get(&(id.to_string(), o.option.key.clone()));
    let field = div().flex_1().min_w(px(FIELD_MIN_W)).max_w(px(FIELD_MAX_W)).flex().flex_col();
    match input {
        Some(input) => field.child(labeled(tr(&o.option.label).to_string(), input_box(input, false))).children(help(o)),
        None => field,
    }
}

fn select_option(d: &SettingsPage, id: &str, o: &OptionRow, cx: &mut Context<SettingsPage>) -> Div {
    let key = SharedString::from(format!("opt:{id}:{}", o.option.key));
    let current = o.value.as_text();
    let shown = o.option.choices.iter().find(|c| c.value == current).map_or(current.to_string(), |c| tr(&c.label).to_string());
    let options = o.option.choices.iter().map(|c| (SharedString::from(tr(&c.label).to_string()), c.value == current)).collect();
    let (toggle_key, values) = (key.clone(), o.option.choices.iter().map(|c| c.value.clone()).collect::<Vec<_>>());
    let (sid, opt) = (id.to_string(), o.option.key.clone());
    let select = select_field(
        &format!("md-{key}"),
        Some(tr(&o.option.label)),
        shown,
        false,
        d.metadata.open_select.as_ref() == Some(&key),
        options,
        cx,
        move |d: &mut SettingsPage, _, cx| {
            let open = d.metadata.open_select.as_ref() == Some(&toggle_key);
            d.metadata.open_select = (!open).then(|| toggle_key.clone());
            cx.notify();
        },
        move |d: &mut SettingsPage, i, _, cx| {
            d.metadata.open_select = None;
            if let Some(v) = values.get(i) {
                set_option(d, &sid, &opt, OptionValue::Text(v.clone()), cx);
            }
        },
    );
    div().w(px(FIELD_MIN_W)).flex().flex_col().child(select).children(help(o))
}

fn switch_option(id: &str, o: &OptionRow, cx: &mut Context<SettingsPage>) -> AnyElement {
    let on = o.value.as_bool();
    let help = o.option.help.as_deref().map(tr);
    let (sid, opt) = (id.to_string(), o.option.key.clone());
    toggle_row(SharedString::from(format!("md-opt-{id}-{}", o.option.key)), tr(&o.option.label).to_string(), help, on)
        .py(px(4.))
        .on_click(cx.listener(move |d, _, _, cx| set_option(d, &sid, &opt, OptionValue::Switch(!on), cx)))
        .into_any_element()
}

/// "Test" and what the last test said.
fn test_line(d: &SettingsPage, id: &str, cx: &mut Context<SettingsPage>) -> Div {
    let testing = d.metadata.testing.as_deref() == Some(id);
    let label = if testing { tr!("Testing\u{2026}") } else { tr!("Test") };
    let sid = id.to_string();
    let button = action_button(
        SharedString::from(format!("md-test-{id}")),
        ButtonKind::Glass,
        label,
        d.metadata.testing.is_none(),
        cx.listener(move |d, _, window, cx| test(d, &sid, window, cx)),
    );
    let result = d.metadata.tested.get(id).filter(|_| !testing).map(|t| {
        let c: Hsla = if t.ok { theme::success().into() } else { theme::danger().into() };
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .text_size(px(13.))
            .text_color(theme::readable(c))
            .child(icon(if t.ok { Icon::Check } else { Icon::Warning }, px(15.), theme::readable(c)))
            .child(div().min_w_0().child(t.message.clone()))
    });
    div().flex().items_center().gap(px(12.)).child(button).children(result)
}

/// "Fills" and a chip per field.
fn fills(r: &SourceRow, cx: &mut Context<SettingsPage>) -> Div {
    let switchable = r.fields.iter().any(|f| f.switchable);
    let head = div().flex().items_end().gap(px(8.)).child(field_label(tr!("Fills"))).when(switchable, |d| {
        d.child(
            div()
                .mb(px(6.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme::text_dim())
                .child(tr!("Click one to stop taking it from here")),
        )
    });
    let mut chips = div().flex().flex_wrap().gap(px(6.));
    for f in &r.fields {
        let label = labels::field_name(&f.field).to_string();
        let chip = div()
            .id(SharedString::from(format!("md-fill-{}-{}", r.descriptor.id, f.field)))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(10.))
            .py(px(3.))
            .rounded_full()
            .border_1()
            .text_size(px(12.))
            .line_height(px(16.));
        let chip = match (f.switchable, f.on) {
            (true, true) => chip
                .bg(theme::accent_alpha(0.12))
                .border_color(theme::accent_alpha(0.35))
                .text_color(theme::text_primary())
                .child(icon(Icon::Check, px(12.), theme::readable(theme::accent())))
                .child(label),
            (true, false) => chip.border_color(theme::ink_a(0.14)).text_color(theme::text_dim()).line_through().child(label),
            (false, _) => chip.bg(theme::ink_a(0.06)).border_color(theme::ink_a(0.0)).text_color(theme::text_muted()).child(label),
        };
        let chip = if f.switchable {
            let (sid, field, on) = (r.descriptor.id.clone(), f.field.clone(), f.on);
            chip.cursor_pointer()
                .hover(|s| s.bg(theme::ink_a(0.08)))
                .on_click(cx.listener(move |d, _, _, cx| super::apply(d, cx, |s| s.set_source_field_enabled(&sid, &field, !on))))
        } else {
            chip
        };
        chips = chips.child(chip);
    }
    div().flex().flex_col().child(head).child(chips)
}
