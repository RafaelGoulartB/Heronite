//! The fields: a row per field that has a value now or from a source. The
//! value that will be saved (two lines at most; covers and backgrounds as
//! pictures), the sources that have one as small pills to pick from (the
//! picked one ringed, "Keep current" when the game has a value, a score's
//! number in its pill; the pill under the pointer shows its value) and the
//! lock. The rows keep one height, and the list scrolls.

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
const PILLS_W: f32 = 250.;
const LOCK_W: f32 = 28.;

impl MetadataDialog {
    fn pill(&self, field: &'static str, source: String, picked: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let frozen = self.frozen(field);
        let keep = source == KEEP;
        let name = if keep { tr!("Keep current").to_string() } else { words::source_name(&self.service, &source) };
        let score = (field == "criticScore" && !keep).then(|| self.value_of(field, &source));
        let (hover_field, hover_source) = (field, source.clone());
        let pick_source = source.clone();
        div()
            .id(SharedString::from(format!("md-pill-{field}-{source}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .h(px(26.))
            .pl(px(if keep { 9. } else { 3. }))
            .pr(px(if keep || score.is_some() { 9. } else { 3. }))
            .rounded_full()
            .border_1()
            .border_color(if picked { theme::accent().into() } else { theme::ink_a(0.1) })
            .when(picked, |d| d.bg(theme::accent_alpha(0.14)))
            .when(!picked, |d| d.bg(theme::ink_a(0.03)))
            .tooltip(crate::ui::tooltip(name.clone()))
            .when(frozen, |d| d.opacity(0.4))
            .when(!frozen, |d| {
                d.cursor_pointer()
                    .hover(|s| s.border_color(theme::ink_a(0.3)))
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
                        this.set_pick(field, &pick_source);
                        cx.notify();
                    }))
            })
            .when(keep, |d| d.text_size(px(11.5)).text_color(theme::text_muted()).child(name.clone()))
            .when(!keep, |d| d.child(labels::mark(&source, &name, 20.)))
            .children(score.map(|n| div().text_size(px(12.)).text_color(theme::text_primary()).child(n)))
    }

    /// The value shown in the middle: the pill under the pointer's, else
    /// the pick's.
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
            let (w, h) = if field == "cover" { (30., 40.) } else { (71., 40.) };
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
        let picked = self.pick(field);
        let mut pills = div().flex().items_center().gap(px(5.)).w(px(PILLS_W)).flex_none().overflow_hidden();
        let has_current = !format_value(self.current(), field).is_empty();
        let sources: Vec<String> = self
            .preview
            .as_ref()
            .map(|p| p.candidates.iter().filter(|c| !format_value(Some(&c.metadata), field).is_empty()).map(|c| c.source.clone()).collect())
            .unwrap_or_default();
        for source in sources {
            let on = picked == source;
            pills = pills.child(self.pill(field, source, on, cx));
        }
        if has_current {
            pills = pills.child(self.pill(field, KEEP.to_string(), picked == KEEP, cx));
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(ROW_H))
            .px(px(14.))
            .border_b_1()
            .border_color(theme::row_separator())
            .child(
                div().w(px(LABEL_W)).flex_none().truncate().text_size(px(12.5)).text_color(theme::text_muted()).child(field_label(field)),
            )
            .child(div().flex().flex_1().min_w_0().items_center().child(self.shown_value(field, cx)))
            .child(pills)
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
            .child(head(tr!("Take it from")).w(px(PILLS_W)).flex_none())
            .child(div().w(px(LOCK_W)).flex_none());
        let fields = self.visible_fields();
        let mut list = div().id("md-fields").flex().flex_col().flex_1().min_h_0().overflow_y_scroll();
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
