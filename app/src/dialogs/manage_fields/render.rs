//! The parts of the fields manager: field tabs, suggestions, the names, the
//! remembered changes and the footer (selection bar).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::field_names::{Field, FieldValue, FIELDS};
use heronite_data::text::facet_key;
use heronite_i18n::{tr, trn};

use super::{ManageFieldsDialog, Pane};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::pages::{caption_with, empty_row, icon_action, tag_pill};
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, help_text, icon_button_sm, tabs, ButtonKind};
use crate::ui::fields::{input_box, search_box, search_box_with};
use crate::ui::icon;

/// Suggestions shown before "Show all".
const FEW_SIMILAR: usize = 3;

pub(super) fn field_label(field: Field) -> &'static str {
    match field {
        Field::Genre => tr!("Genres"),
        Field::Company => tr!("Companies"),
        Field::Series => tr!("Series"),
        Field::Theme => tr!("Themes"),
        Field::Feature => tr!("Features"),
        Field::Platform => tr!("Platforms"),
    }
}

/// Round mark of a selected row (filled accent with a check) or an empty ring.
fn select_mark(on: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(18.))
        .rounded_full()
        .border_1()
        .when(on, |d| d.bg(theme::accent()).border_color(theme::accent()).child(icon(Icon::Check, px(13.), white())))
        .when(!on, |d| d.border_color(theme::ink_a(0.28)))
}

/// A name with its games, as a pill of the suggestions.
fn name_pill(value: &FieldValue, target: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(3.))
        .rounded_full()
        .text_size(px(13.))
        .when(target, |d| d.bg(theme::accent_tint()).border_1().border_color(theme::accent_tint_border()).text_color(theme::text_bright()))
        .when(!target, |d| d.bg(theme::ink_a(0.07)).text_color(theme::text_muted()))
        .child(value.name.clone())
        .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(value.games.to_string()))
}

impl ManageFieldsDialog {
    pub(super) fn render_body(&mut self, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        let items: Vec<(Field, &'static str)> = FIELDS.iter().map(|f| (*f, field_label(*f))).collect();
        let field_tabs = tabs("fields", &items, self.field, move |f, w, cx| {
            this.update(cx, |d, cx| d.set_field(f, w, cx)).ok();
        });
        let pane = match self.pane {
            Pane::Names => self.render_names(cx),
            Pane::Remembered => self.render_remembered(cx),
        };
        // Tall enough that switching to a short field does not shrink the dialog much.
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .min_h(px(460.))
            .child(help_text(tr!(
                "IGDB, Steam and the stores spell the same names in different ways. Merge, rename or remove them here: metadata downloaded later follows the change."
            )))
            .when(self.pane == Pane::Names, |d| d.child(field_tabs))
            .children(self.notice.clone().map(|n| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(icon(Icon::Check, px(15.), theme::accent()))
                    .child(n)
            }))
            .children(self.error.clone().map(|e| div().text_color(theme::danger_text()).text_size(px(13.)).child(e)))
            .child(pane)
    }

    fn render_names(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(data) = self.data.clone() else {
            return div().child(empty_row(tr!("Loading…")));
        };
        let query = facet_key(self.search.read(cx).value().trim());
        let fd = data.field(self.field);
        let shown: Vec<&FieldValue> = fd.values.iter().filter(|v| query.is_empty() || v.key.contains(&query)).collect();
        let count = if query.is_empty() {
            trn!(fd.values.len(), "{n} name", "{n} names")
        } else {
            tr!("{shown} of {total}", shown = shown.len(), total = fd.values.len())
        };
        let mut col = div().flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(self.render_search(&query, cx))
                .child(div().flex_none().text_size(px(12.5)).text_color(theme::text_muted()).child(count)),
        );
        if query.is_empty() && !fd.similar.is_empty() {
            col = col.child(self.render_similar(&fd.similar, &fd.values, cx));
        }
        col = col.child(caption(if query.is_empty() { tr!("All names") } else { tr!("Matching names") }));
        if shown.is_empty() {
            let text = if fd.values.is_empty() { tr!("No game has one yet.") } else { tr!("No name matches the search.") };
            return col.child(empty_row(text));
        }
        let rows: Vec<AnyElement> = shown.iter().enumerate().map(|(i, v)| self.render_row(i, v, cx).into_any_element()).collect();
        col.child(card(rows))
    }

    /// The search box, with a clear button once something is typed.
    fn render_search(&self, query: &str, cx: &mut Context<Self>) -> Div {
        if query.is_empty() {
            return search_box(&self.search);
        }
        let clear = div()
            .id("fields-search-clear")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.bg(theme::ink_a(0.1)))
            .child(icon(Icon::Close, px(14.), theme::ink_a(0.7)))
            .on_click(cx.listener(|this, _, window, cx| {
                this.search.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                    s.focus(window, cx);
                });
                cx.notify();
            }));
        search_box_with(&self.search, clear.into_any_element())
    }

    fn render_similar(&self, groups: &[Vec<usize>], values: &[FieldValue], cx: &mut Context<Self>) -> Div {
        let more = groups.len().saturating_sub(FEW_SIMILAR);
        let toggle = (more > 0).then(|| {
            let label = if self.all_similar { tr!("Show fewer").to_string() } else { tr!("Show all ({count})", count = groups.len()) };
            div()
                .id("fields-similar-all")
                .cursor_pointer()
                .mr(px(6.))
                .text_size(px(12.5))
                .text_color(theme::accent())
                .hover(|s| s.text_color(theme::text_bright()))
                .child(label)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.all_similar = !this.all_similar;
                    cx.notify();
                }))
        });
        let take = if self.all_similar { groups.len() } else { FEW_SIMILAR };
        let rows: Vec<AnyElement> = groups
            .iter()
            .take(take)
            .enumerate()
            .map(|(i, g)| {
                let keys: Vec<String> = g.iter().map(|&j| values[j].key.clone()).collect();
                let pills = div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .children(g.iter().enumerate().map(|(n, &j)| name_pill(&values[j], n == 0)));
                let (k1, k2, k3) = (keys.clone(), keys.clone(), keys);
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(9.))
                    .child(pills)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .child(icon_button_sm(
                                ("fields-merge", i),
                                ButtonKind::Glass,
                                Icon::CallMerge,
                                tr!("Merge"),
                                !self.busy,
                                cx.listener(move |this, _, _, cx| this.merge_group(k1.clone(), cx)),
                            ))
                            .child(
                                button_sm(("fields-other", i), ButtonKind::Ghost, tr!("Other name…"))
                                    .on_click(cx.listener(move |this, _, w, cx| this.select_group(k2.clone(), w, cx))),
                            )
                            .child(
                                button_sm(("fields-apart", i), ButtonKind::Ghost, tr!("Not the same"))
                                    .on_click(cx.listener(move |this, _, _, cx| this.keep_apart(k3.clone(), cx))),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let title = trn!(groups.len(), "{n} possible duplicate", "{n} possible duplicates");
        div()
            .flex()
            .flex_col()
            .child(caption_with(title, div().children(toggle)))
            .child(help_text(tr!("Merge keeps the highlighted name, the most used one.")).mx(px(6.)).mb(px(6.)).text_size(px(12.)))
            .child(card(rows))
    }

    fn render_row(&self, i: usize, v: &FieldValue, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = v.key.clone();
        let row = div().id(("fields-row", i)).flex().items_center().gap(px(12.)).py(px(6.)).min_h(px(44.));
        if let Some((_, input)) = self.renaming.as_ref().filter(|(k, _)| *k == key) {
            return row
                .child(div().flex_1().min_w_0().child(input_box(input, false)))
                .child(div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Enter to rename")));
        }
        let confirming = self.confirm_remove.as_ref().is_some_and(|keys| keys.len() == 1 && keys[0] == key && self.selected != *keys);
        if confirming {
            return row
                .child(div().flex_1().min_w_0().text_size(px(13.5)).child(self.remove_question(std::slice::from_ref(&key))))
                .child(button_sm(("fields-row-cancel", i), ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_remove = None;
                    cx.notify();
                })))
                .child(
                    button_sm(("fields-row-remove", i), ButtonKind::Danger, tr!("Remove"))
                        .on_click(cx.listener(|this, _, _, cx| this.remove_confirmed(cx))),
                );
        }
        let selected = self.selected.contains(&key);
        let (k_sel, k_ren, k_del) = (key.clone(), key.clone(), key);
        row.cursor_pointer()
            .on_click(cx.listener(move |this, _, w, cx| this.toggle_selected(&k_sel, w, cx)))
            .child(select_mark(selected))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .text_color(if selected { theme::text_bright() } else { theme::text_primary().into() })
                    .child(v.name.clone()),
            )
            .child(div().flex_none().text_size(px(12.5)).text_color(theme::text_muted()).child(trn!(v.games, "{n} game", "{n} games")))
            .child(icon_action(("fields-rename", i), Icon::Edit, !self.busy).on_click(cx.listener(move |this, _, w, cx| {
                cx.stop_propagation();
                this.start_rename(&k_ren, w, cx);
            })))
            .child(icon_action(("fields-remove", i), Icon::FaTrash, !self.busy).on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.ask_remove(vec![k_del.clone()], cx);
            })))
    }

    fn render_remembered(&self, cx: &mut Context<Self>) -> Div {
        let rules = self.data.as_ref().map(|d| d.rules.clone()).unwrap_or_default();
        let mut col = div().flex().flex_col().child(caption(tr!("Remembered changes"))).child(
            help_text(tr!("Metadata downloaded later follows these changes. Forgetting one leaves the games as they are now."))
                .mx(px(6.))
                .mb(px(8.))
                .text_size(px(12.)),
        );
        if rules.is_empty() {
            return col.child(empty_row(tr!("No changes yet.")));
        }
        let rows: Vec<AnyElement> = rules
            .into_iter()
            .enumerate()
            .map(|(i, rule)| {
                let to = match &rule.to {
                    Some(to) => div().text_color(theme::text_primary()).child(to.clone()),
                    None => div().text_color(theme::text_dim()).child(tr!("removed")),
                };
                let (field, from) = (rule.field, rule.from.clone());
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(8.))
                    .text_size(px(13.5))
                    .child(div().w(px(96.)).flex_none().child(tag_pill(field_label(rule.field))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .flex_wrap()
                            .child(div().text_color(theme::text_muted()).child(rule.from.clone()))
                            .child(icon(Icon::ChevronRight, px(14.), theme::text_dim()))
                            .child(to),
                    )
                    .child(
                        button_sm(("fields-forget", i), ButtonKind::Ghost, tr!("Forget"))
                            .on_click(cx.listener(move |this, _, _, cx| this.forget(field, from.clone(), cx))),
                    )
                    .into_any_element()
            })
            .collect();
        col = col.child(card(rows));
        col
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> Div {
        let done =
            button_sm("fields-done", ButtonKind::Primary, tr!("Done")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close)));
        let left = if self.pane == Pane::Remembered {
            div().child(button_sm("fields-back", ButtonKind::Ghost, tr!("Back to names")).on_click(cx.listener(|this, _, _, cx| {
                this.pane = Pane::Names;
                cx.notify();
            })))
        } else if self.confirm_remove.as_ref().is_some_and(|k| !k.is_empty() && *k == self.selected) {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().text_size(px(13.5)).mr(px(4.)).child(self.remove_question(&self.selected)))
                .child(button_sm("fields-sel-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                    this.confirm_remove = None;
                    cx.notify();
                })))
                .child(
                    button_sm("fields-sel-remove", ButtonKind::Danger, tr!("Remove"))
                        .on_click(cx.listener(|this, _, _, cx| this.remove_confirmed(cx))),
                )
        } else if !self.selected.is_empty() {
            self.render_selection_bar(cx)
        } else {
            let n = self.data.as_ref().map_or(0, |d| d.rules.len());
            div().child(button_sm("fields-rules", ButtonKind::Ghost, tr!("Remembered changes ({count})", count = n)).on_click(cx.listener(
                |this, _, _, cx| {
                    this.pane = Pane::Remembered;
                    this.notice = None;
                    cx.notify();
                },
            )))
        };
        div().flex().flex_1().items_center().justify_between().gap(px(12.)).child(left).child(done)
    }

    fn render_selection_bar(&self, cx: &mut Context<Self>) -> Div {
        let n = self.selected.len();
        let mut bar = div().flex().items_center().gap(px(8.)).child(
            div().flex_none().text_size(px(13.)).text_color(theme::text_muted()).mr(px(4.)).child(trn!(
                n,
                "{n} name selected",
                "{n} names selected"
            )),
        );
        if n >= 2 {
            bar = bar.child(div().w(px(220.)).child(input_box(&self.merge_name, false))).child(icon_button_sm(
                "fields-sel-merge",
                ButtonKind::Primary,
                Icon::CallMerge,
                tr!("Merge"),
                !self.busy,
                cx.listener(|this, _, _, cx| this.merge_selected(cx)),
            ));
        } else {
            let key = self.selected[0].clone();
            bar = bar.child(
                button_sm("fields-sel-rename", ButtonKind::Glass, tr!("Rename"))
                    .on_click(cx.listener(move |this, _, w, cx| this.start_rename(&key, w, cx))),
            );
        }
        bar.child(button_sm("fields-sel-remove-ask", ButtonKind::Danger, tr!("Remove")).on_click(cx.listener(|this, _, _, cx| {
            let keys = this.selected.clone();
            this.ask_remove(keys, cx);
        })))
        .child(
            button_sm("fields-sel-clear", ButtonKind::Ghost, tr!("Clear")).on_click(cx.listener(|this, _, _, cx| this.clear_selection(cx))),
        )
    }
}
