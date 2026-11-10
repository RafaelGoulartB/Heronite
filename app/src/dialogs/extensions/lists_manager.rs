//! "Lists…": the lists the gallery reads (the app's own, which stays, then
//! the ones added), a field to add one by its address with a word of
//! caution, and Remove on each added list. The sources installed from a
//! list stay when it is removed.

use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{words, ExtensionsDialog, Snapshot};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::fields::{field_label, input_box, meta_text};
use crate::ui::icon;

impl ExtensionsDialog {
    fn list_row(&self, url: &str, builtin: bool, snapshot: &Snapshot, cx: &mut Context<Self>) -> Div {
        let copy = snapshot.copies.iter().find(|c| c.url == url);
        let entries = copy.and_then(|c| c.index.as_ref()).map(|i| i.entries.len());
        let state = match (copy.and_then(|c| c.error.as_ref()), entries, copy.and_then(|c| c.read_at)) {
            (Some(e), None, _) => words::list_error(e),
            (_, Some(n), Some(at)) => {
                format!("{} \u{b7} {}", heronite_i18n::trn!(n, "{n} source", "{n} sources"), words::read_ago(at, SystemTime::now()))
            }
            _ => tr!("Not read yet").into(),
        };
        let name = words::list_name(url, builtin, self.hub.read(cx).extensions.is_checkout(url));
        let remove = (!builtin).then(|| {
            let url = url.to_string();
            action_button(
                SharedString::from(format!("ext-list-remove-{url}")),
                ButtonKind::Ghost,
                tr!("Remove"),
                true,
                cx.listener(move |this, _, _, cx| this.remove_list(url.clone(), cx)),
            )
        });
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .py(px(10.))
            .child(icon(if builtin { Icon::Check } else { Icon::Link }, px(16.), theme::text_muted()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(name))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(url.to_string()))
                    .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(state)),
            )
            .children(remove)
    }

    pub(super) fn render_lists(&self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut rows = div().flex().flex_col();
        for (i, (url, builtin)) in snapshot.in_use.iter().enumerate() {
            if i > 0 {
                rows = rows.child(div().h(px(1.)).w_full().bg(theme::row_separator()));
            }
            rows = rows.child(self.list_row(url, *builtin, snapshot, cx));
        }
        let add = div()
            .flex()
            .items_end()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(field_label(tr!("Add a list by its address")))
                    .child(input_box(&self.list_input, false)),
            )
            .child(
                button("ext-list-add", ButtonKind::Glass, tr!("Add"))
                    .mb(px(2.))
                    .on_click(cx.listener(|this, _, window, cx| this.add_list(window, cx))),
            );
        let error = self.list_error.clone().map(|e| div().text_size(px(12.5)).text_color(theme::danger_text()).child(e));
        div()
            .id("ext-lists")
            .flex()
            .flex_col()
            .gap(px(14.))
            .size_full()
            .overflow_y_scroll()
            .p(px(18.))
            .rounded(px(16.))
            .bg(theme::ink_a(0.03))
            .border_1()
            .border_color(theme::ink_a(0.08))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(16.)).text_color(theme::text_primary()).child(tr!("Lists in use")))
                    .child(button("ext-lists-back", ButtonKind::Ghost, tr!("Back to the sources")).on_click(cx.listener(|this, _, _, cx| {
                        this.lists_open = false;
                        cx.notify();
                    }))),
            )
            .child(rows)
            .child(add)
            .children(error)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(div().pt(px(1.)).child(icon(Icon::Warning, px(14.), theme::readable(theme::warning()))))
                    .child(div().flex_1().min_w_0().child(meta_text(tr!(
                        "Anyone can publish a list. Heronite checks that a source's files are the ones its list names, not what the source does: add only lists from people you trust. Removing a list keeps the sources installed from it."
                    )))),
            )
    }
}
