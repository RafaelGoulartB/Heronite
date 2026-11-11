//! The fields: a row per field that has a value now or from a source. The
//! value that will be saved (two lines at most; covers and backgrounds as
//! pictures), where it comes from and the lock. Sources that have the same
//! value are one choice ("IGDB + Lutris"), and so is what the game has
//! ("· current"); with two choices or more they are segments to pick from
//! (the segment under the pointer shows its value), with one a line says
//! where it comes from ("Only IGDB", "IGDB, Lutris · same as now"). The
//! rows are one height unless the segments wrap, and the list scrolls.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::found_as::busy_line;
use super::{field_label, format_value, is_picture, Busy, MetadataDialog, KEEP};
use crate::dialogs::metadata_update::words;
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::theme;
use crate::ui::icon;

const ROW_H: f32 = 50.;
const LABEL_W: f32 = 110.;
const SOURCE_W: f32 = 270.;
const LOCK_W: f32 = 28.;

/// One value of a field and who has it: source ids, and [`KEEP`] when the
/// game has it now.
struct Choice {
    ids: Vec<String>,
    /// The id a click picks: the merge's pick when it is one of `ids`, else
    /// the first source, else [`KEEP`].
    pick: String,
}

impl Choice {
    fn keeps(&self) -> bool {
        self.ids.iter().any(|id| id == KEEP)
    }

    fn sources(&self) -> impl Iterator<Item = &String> {
        self.ids.iter().filter(|id| *id != KEEP)
    }
}

impl MetadataDialog {
    /// The values `field` can take, in the order of the sources, what the
    /// game has last; sources with the same value are one choice.
    fn choices(&self, field: &str) -> Vec<Choice> {
        let mut ids: Vec<String> = self
            .preview
            .as_ref()
            .map(|p| p.candidates.iter().filter(|c| !format_value(Some(&c.metadata), field).is_empty()).map(|c| c.source.clone()).collect())
            .unwrap_or_default();
        if !format_value(self.current(), field).is_empty() {
            ids.push(KEEP.to_string());
        }
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for id in ids {
            let value = self.value_of(field, &id);
            match groups.iter_mut().find(|(v, _)| *v == value) {
                Some((_, members)) => members.push(id),
                None => groups.push((value, vec![id])),
            }
        }
        let default = self.default_pick(field);
        groups
            .into_iter()
            .map(|(_, ids)| {
                let pick = if ids.contains(&default) {
                    default.clone()
                } else {
                    ids.iter().find(|id| *id != KEEP).cloned().unwrap_or_else(|| KEEP.to_string())
                };
                Choice { ids, pick }
            })
            .collect()
    }

    fn names(&self, choice: &Choice, sep: &str) -> String {
        choice.sources().map(|id| words::source_name(&self.service, id)).collect::<Vec<_>>().join(sep)
    }

    /// A segment: the names of the sources that have this value (or "Keep
    /// current"), "· current" when the game has it too.
    fn segment(&self, field: &'static str, choice: &Choice, picked: bool, frozen: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let names = self.names(choice, " + ");
        let only_keep = names.is_empty();
        let source = choice.pick.clone();
        let (hover_field, hover_source) = (field, source.clone());
        let accent: Hsla = theme::accent().into();
        div()
            .id(SharedString::from(format!("md-seg-{field}-{source}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(26.))
            .px(px(10.))
            .rounded_full()
            .border_1()
            .text_size(px(12.5))
            .whitespace_nowrap()
            .border_color(if picked { Hsla { a: 0.55, ..accent } } else { theme::ink_a(0.0) })
            .when(picked, |d| d.bg(theme::accent_alpha(0.16)).text_color(theme::text_bright()))
            .when(!picked, |d| d.text_color(theme::ink_a(0.68)))
            .when(!frozen, |d| {
                d.cursor_pointer()
                    .when(!picked, |d| d.hover(|s| s.text_color(theme::text_bright()).bg(theme::ink_a(0.05))))
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
            .child(if only_keep { tr!("Keep current").to_string() } else { names })
            .when(!only_keep && choice.keeps(), |d| {
                d.child(div().text_color(theme::text_dim()).child(format!("\u{b7} {}", tr!("current"))))
            })
    }

    /// Where `field` comes from: segments when there is a choice, else a
    /// line naming the sources.
    fn source_picker(&self, field: &'static str, cx: &mut Context<Self>) -> Div {
        let choices = self.choices(field);
        let column = div().flex().items_center().w(px(SOURCE_W)).flex_none().min_w_0();
        if choices.len() > 1 {
            let picked = self.pick(field);
            let frozen = self.frozen(field);
            let mut track =
                div().flex().flex_wrap().items_center().gap(px(2.)).p(px(3.)).max_w_full().rounded(px(16.)).bg(theme::ink_a(0.06));
            for choice in &choices {
                let on = choice.ids.contains(&picked);
                track = track.child(self.segment(field, choice, on, frozen, cx));
            }
            return column.child(track.when(frozen, |d| d.opacity(0.4)));
        }
        let Some(choice) = choices.first() else { return column };
        let names = self.names(choice, ", ");
        let line = match (choice.sources().count(), choice.keeps()) {
            (0, _) => tr!("No source has it").to_string(),
            (1, false) => tr!("Only {source}", source = names),
            _ => names,
        };
        let mut row = div().flex().items_center().gap(px(5.)).min_w_0().text_size(px(12.)).text_color(theme::text_muted());
        for id in choice.sources() {
            row = row.child(labels::mark(id, &words::source_name(&self.service, id), 16.));
        }
        row = row.child(div().min_w_0().truncate().child(line));
        if choice.keeps() && choice.sources().count() > 0 {
            row = row.child(div().flex_none().text_color(theme::text_dim()).child(format!("\u{b7} {}", tr!("same as now"))));
        }
        column.child(row)
    }

    /// The value shown in the middle: the segment under the pointer's,
    /// else the pick's.
    fn shown_value(&mut self, field: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let source = match &self.hover {
            Some((f, s)) if *f == field => s.clone(),
            _ => self.pick(field),
        };
        let value = self.value_of(field, &source);
        if value.is_empty() {
            return div().text_size(px(12.5)).text_color(theme::text_dim()).child(tr!("Empty")).into_any_element();
        }
        if is_picture(field) {
            let (w, h) = match field {
                "cover" => (27., 40.),
                "coverSquare" => (40., 40.),
                _ => (71., 40.),
            };
            return self.thumb(&value, w, h, cx).into_any_element();
        }
        div()
            .text_size(px(13.))
            .line_height(px(17.))
            .max_h(px(34.))
            .overflow_hidden()
            .text_color(theme::text_primary())
            .child(value)
            .into_any_element()
    }

    fn lock_button(&self, field: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let locked = self.locked(field);
        let warn: Hsla = theme::warning().into();
        div()
            .id(SharedString::from(format!("md-lock-{field}")))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(LOCK_W))
            .rounded(px(8.))
            .cursor_pointer()
            .when(locked, |d| d.bg(Hsla { a: 0.18, ..warn }))
            .hover(|s| s.bg(theme::ink_a(0.08)))
            .tooltip(crate::ui::tooltip(if locked {
                tr!("Locked: later updates leave this field as it is. Click to unlock.")
            } else {
                tr!("Lock this field")
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.busy.is_none() {
                    this.toggle_lock(field);
                    cx.notify();
                }
            }))
            .child(icon(Icon::Lock, px(13.), if locked { theme::readable(warn) } else { theme::ink_a(0.3) }))
    }

    fn field_row(&mut self, field: &'static str, cx: &mut Context<Self>) -> Div {
        let source = self.source_picker(field, cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .min_h(px(ROW_H))
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
                    .when(field == "cover", |d| {
                        d.child(
                            div()
                                .id("md-choose-covers")
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(theme::accent())
                                .cursor_pointer()
                                .hover(|s| s.text_color(theme::text_bright()))
                                .child(tr!("Choose covers\u{2026}"))
                                .on_click(cx.listener(|this, _, window, cx| this.open_covers(window, cx))),
                        )
                    }),
            )
            .child(div().flex().flex_1().min_w_0().items_center().child(self.shown_value(field, cx)))
            .child(source)
            .child(self.lock_button(field, cx))
    }

    pub(super) fn render_fields(&mut self, cx: &mut Context<Self>) -> Div {
        let head = |text: &'static str| div().text_size(px(11.5)).text_color(theme::text_dim()).child(text);
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(36.))
            .px(px(14.))
            .border_b_1()
            .border_color(theme::row_separator())
            .child(head(tr!("Field")).w(px(LABEL_W)).flex_none())
            .child(head(tr!("What will be saved")).flex_1())
            .child(head(tr!("Source")).w(px(SOURCE_W)).flex_none())
            .child(div().w(px(LOCK_W)).flex_none());
        let fields = self.visible_fields();
        let mut list = div().id("md-fields").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.fields_scroll);
        if self.scroll_to_covers && self.kept.len() > 1 {
            if let Some(i) = fields.iter().position(|f| *f == "cover") {
                self.fields_scroll.scroll_to_item(i + self.kept.len());
            }
        }
        if self.preview.is_none() {
            if self.busy == Some(Busy::Preview) {
                list = list.child(div().p(px(14.)).child(busy_line(tr!("The fields appear when the sources answered."))));
            }
        } else if fields.is_empty() {
            list = list.child(
                div()
                    .p(px(16.))
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(theme::text_muted())
                    .child(tr!("No source found this game, and it has no details yet. Search a source by another name, on the left.")),
            );
        } else {
            for field in fields {
                list = list.child(self.field_row(field, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .rounded(px(theme::R_CARD))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .overflow_hidden()
            .child(header)
            .child(list)
    }
}
