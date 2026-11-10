//! Console mode's rows of Settings → Metadata: a switch per source with its
//! options, its place among the details sources and its "Test"; what happens
//! when details arrive; and which source each field asks first (dragging
//! does not fit a list of rows, so a field's order is picked by its first
//! source). Built from the rows the page keeps, never from the files.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::sources::{OptionKind, OptionValue, SourceKind, SourceStatus};

use super::super::SettingsPage;
use super::arrive::{board_index, board_options, set_board};
use super::{apply, labels, move_source, persist, set_enabled, set_option, test};
use crate::console::list::{Outcome, Row, Value};
use crate::dialogs::metadata::field_label;

fn masked(value: &str) -> String {
    "\u{2022}".repeat(value.chars().count().min(24))
}

pub(in crate::pages::settings) fn console_rows(d: &SettingsPage, _cx: &App) -> Vec<Row> {
    let st = &d.metadata;
    let m = &d.settings.metadata;
    let mut rows = Vec::new();
    for (kind, title) in
        [(SourceKind::Details, tr!("Game details")), (SourceKind::Pictures, tr!("Pictures")), (SourceKind::Extras, tr!("Extras"))]
    {
        let sources: Vec<_> = st.rows.iter().filter(|r| r.descriptor.kind == kind).collect();
        if sources.is_empty() {
            continue;
        }
        rows.push(Row::header(title));
        let movable = sources.iter().filter(|r| r.reorderable).count();
        for (place, r) in sources.iter().enumerate() {
            let id = &r.descriptor.id;
            let name = labels::name(&r.descriptor.name);
            let desc = format!("{} \u{b7} {}", labels::status_word(r.status), labels::audience(&r.descriptor));
            rows.push(Row::toggle(format!("on:{id}"), name, r.enabled).desc(desc).enabled(r.status != SourceStatus::Unavailable));
            for o in &r.options {
                let row_id = format!("opt:{id}:{}", o.option.key);
                let label = tr(&o.option.label);
                let row = match o.option.kind {
                    OptionKind::Text => Row::text(row_id, label, o.value.as_text().to_string()),
                    OptionKind::Secret => Row::text(row_id, label, masked(o.value.as_text())),
                    OptionKind::Switch => Row::toggle(row_id, label, o.value.as_bool()),
                    OptionKind::Select => {
                        let options = o.option.choices.iter().map(|c| SharedString::from(tr(&c.label).to_string())).collect();
                        let index = o.option.choices.iter().position(|c| c.value == o.value.as_text()).unwrap_or(0);
                        Row::choice(row_id, label, options, index)
                    }
                };
                rows.push(row.desc(o.option.help.as_deref().map(tr).unwrap_or_default().to_string()));
            }
            if r.reorderable && movable > 1 {
                rows.push(
                    Row::number(format!("place:{id}"), tr!("Place in the list"), place as i64 + 1, 1, movable as i64, 1, "").compact(),
                );
            }
            if r.can_test {
                let testing = st.testing.as_deref() == Some(id.as_str());
                let result = st.tested.get(id).map(|t| t.message.clone()).unwrap_or_default();
                rows.push(
                    Row::action(format!("test:{id}"), if testing { tr!("Testing\u{2026}") } else { tr!("Test") })
                        .enabled(!testing)
                        .desc(result),
                );
            }
        }
    }
    // Installing a source needs a file chooser and reading what it may
    // do: the desktop's.
    rows.push(
        Row::info("more", tr!("Get more sources"), tr!("On the desktop"))
            .desc(tr!("Sources made by the community are installed from Settings \u{2192} Metadata on the desktop.")),
    );
    rows.extend(super::shapes::console_rows(d));
    rows.extend([
        Row::header(tr!("When details arrive")),
        Row::toggle("autofill", tr!("Fill missing metadata in the background"), m.auto_fill_missing),
        Row::toggle("download", tr!("Copy covers to disk"), m.download_images).desc(tr!("Off keeps the CDN addresses instead")),
        Row::toggle("adult", tr!("Allow adult covers"), m.allow_adult_covers).desc(super::arrive::adult_help()),
        Row::toggle("age-show", tr!("Show age ratings"), m.show_age_ratings),
        Row::choice(
            "age-board",
            tr!("Rating board shown first"),
            board_options().into_iter().map(|(_, label)| SharedString::from(label)).collect(),
            board_index(d),
        ),
        Row::header(tr!("Source asked first for each field")),
    ]);
    for line in &st.fields {
        let (options, index) = first_choices(d, line);
        rows.push(Row::choice(
            format!("first:{}", line.field),
            field_label(line.field),
            options.into_iter().map(|(_, l)| l).collect(),
            index,
        ));
    }
    rows
}

/// A field's choices of its first source: "List order", then the details
/// sources that are on and fill it. The index is "List order" while the user
/// set no order for the field.
fn first_choices(d: &SettingsPage, line: &super::FieldLine) -> (Vec<(Option<String>, SharedString)>, usize) {
    let mut options = vec![(None, SharedString::from(tr!("List order")))];
    options.extend(
        d.metadata
            .rows
            .iter()
            .filter(|r| r.descriptor.kind == SourceKind::Details && r.enabled && r.fields.iter().any(|f| f.field == line.field && f.on))
            .map(|r| (Some(r.descriptor.id.clone()), SharedString::from(labels::name(&r.descriptor.name).to_string()))),
    );
    let first = line.sources.first().filter(|_| line.customised);
    let index = first.and_then(|f| options.iter().position(|(id, _)| id.as_ref() == Some(f))).unwrap_or(0);
    (options, index)
}

/// Opens the on-screen keyboard for a text option.
fn edit_text(d: &mut SettingsPage, id: String, key: String, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let Some(o) = d.metadata.row(&id).and_then(|r| r.options.iter().find(|o| o.option.key == key)) else { return };
    // A secret is typed again, never shown.
    let initial = if o.option.kind == OptionKind::Secret { String::new() } else { o.value.as_text().to_string() };
    let title = tr(&o.option.label).to_string();
    d.console_osk(&title, &initial, window, cx, move |d, text, window, cx| {
        let text = text.trim().to_string();
        if let Some(input) = d.metadata.inputs.get(&(id.clone(), key.clone())) {
            input.update(cx, |s, cx| s.set_value(text.clone(), window, cx));
        }
        set_option(d, &id, &key, OptionValue::Text(text), cx);
    });
}

pub(in crate::pages::settings) fn console_apply(
    d: &mut SettingsPage,
    outcome: Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) {
    match outcome {
        Outcome::Activate(id) => {
            if super::shapes::console_apply(d, &id, None, cx) {
            } else if let Some(source) = id.strip_prefix("test:") {
                test(d, source, window, cx);
            } else if let Some((source, key)) = id.strip_prefix("opt:").and_then(|r| r.split_once(':')) {
                edit_text(d, source.to_string(), key.to_string(), window, cx);
            }
        }
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "autofill" => persist(d, cx, |m| m.auto_fill_missing = on),
            "download" => persist(d, cx, |m| m.download_images = on),
            "adult" => persist(d, cx, |m| m.allow_adult_covers = on),
            "age-show" => persist(d, cx, |m| m.show_age_ratings = on),
            other => {
                if super::shapes::console_apply(d, other, Some(on), cx) {
                } else if let Some(source) = other.strip_prefix("on:") {
                    set_enabled(d, source, on, cx);
                } else if let Some((source, key)) = other.strip_prefix("opt:").and_then(|r| r.split_once(':')) {
                    set_option(d, source, key, OptionValue::Switch(on), cx);
                }
            }
        },
        Outcome::Changed(id, Value::Number { value, .. }) => {
            if let Some(source) = id.strip_prefix("place:") {
                move_source(d, source, (value - 1).max(0) as usize, cx);
            }
        }
        Outcome::Changed(id, Value::Choice { index, .. }) => {
            if id.as_ref() == "age-board" {
                set_board(d, index, cx);
            } else if let Some(field) = id.strip_prefix("first:") {
                let Some(line) = d.metadata.fields.iter().find(|l| l.field == field) else { return };
                let field = line.field;
                let (options, _) = first_choices(d, line);
                match options.into_iter().nth(index).and_then(|(id, _)| id) {
                    Some(first) => apply(d, cx, |s| s.set_field_order(field, &[first])),
                    None => apply(d, cx, |s| s.reset_field_order(field)),
                }
            } else if let Some((source, key)) = id.strip_prefix("opt:").and_then(|r| r.split_once(':')) {
                let value = d
                    .metadata
                    .row(source)
                    .and_then(|r| r.options.iter().find(|o| o.option.key == key))
                    .and_then(|o| o.option.choices.get(index))
                    .map(|c| c.value.clone());
                if let Some(value) = value {
                    set_option(d, source, key, OptionValue::Text(value), cx);
                }
            }
        }
        _ => {}
    }
    cx.notify();
}
