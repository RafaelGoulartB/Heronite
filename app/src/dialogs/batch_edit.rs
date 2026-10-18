//! F3 with several games selected: their series and tag fields at once
//! (Playnite's multi-game edit). A field where the games differ stays as it
//! is unless edited. A tag only some of the games have is dashed with "3/7":
//! a click gives it to all of them; × takes a tag out of all of them; typing
//! a name and Enter adds it to all (a name still in its box when Apply is
//! pressed counts too). Closing with changes asks first. Nothing is written
//! here: the Collection applies [`BatchEditApply`] with an Undo.

use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::models::CollectionGameMetadata;
use heronite_i18n::{tr, trn};
use heronite_metadata::edit::{BatchDetails, ListDelta, ListField};

use super::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, button, close_button, dialog_title, help_text, kicker, ButtonKind};
use crate::ui::dialog::{drawer, DrawerProps};
use crate::ui::fields::{field_label, input_box, new_input};
use crate::ui::icon;

/// The edit to apply to the selected games.
pub struct BatchEditApply(pub BatchDetails);

fn label_of(field: ListField) -> SharedString {
    match field {
        ListField::Genres => tr!("Genres"),
        ListField::Developers => tr!("Developers"),
        ListField::Publishers => tr!("Publishers"),
        ListField::Themes => tr!("Themes"),
        ListField::GameModes => tr!("Game modes"),
        ListField::Platforms => tr!("Platforms"),
        ListField::Features => tr!("Features"),
    }
    .into()
}

struct TagList {
    field: ListField,
    /// Each name the games have and on how many of them, most common first.
    counts: Vec<(String, usize)>,
    /// Given to all: typed names and partial tags clicked.
    add: Vec<String>,
    remove: Vec<String>,
    input: Entity<InputState>,
}

impl TagList {
    /// The name typed in the box and not added with Enter yet (unless Enter
    /// would change nothing: added already, or on all `total` games).
    fn pending(&self, total: usize, cx: &App) -> Option<String> {
        let name = self.input.read(cx).value().trim().to_string();
        let same = |n: &String| n.eq_ignore_ascii_case(&name);
        let everywhere = self.counts.iter().any(|(n, c)| same(n) && *c >= total) && !self.remove.iter().any(same);
        (!name.is_empty() && !everywhere && !self.add.iter().any(same)).then_some(name)
    }
}

pub struct BatchEditDialog {
    total: usize,
    series: Entity<InputState>,
    /// The series every game shares ("" when they differ or have none).
    series_initial: String,
    series_edited: bool,
    lists: Vec<TagList>,
    /// "Discard changes?" is asked (Esc, ×, a click outside or Cancel with changes).
    confirm_discard: bool,
}

impl EventEmitter<DialogEvent> for BatchEditDialog {}
impl EventEmitter<BatchEditApply> for BatchEditDialog {}

impl BatchEditDialog {
    /// `metadata`: one entry per selected game.
    pub fn new(metadata: Vec<Option<Arc<CollectionGameMetadata>>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let total = metadata.len();
        let mut series: Vec<String> = metadata.iter().map(|m| m.as_ref().and_then(|m| m.series.clone()).unwrap_or_default()).collect();
        series.sort();
        series.dedup();
        let same = series.len() == 1;
        let series_initial = if same { series[0].clone() } else { String::new() };
        let placeholder = if same { String::new() } else { tr!("Mixed: {n} values", n = series.len()) };
        let series_input = new_input(window, cx, &series_initial, &placeholder);
        cx.subscribe_in(&series_input, window, |this, state, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                this.series_edited = state.read(cx).value().trim() != this.series_initial.trim();
                cx.notify();
            }
        })
        .detach();
        let lists = ListField::ALL
            .iter()
            .enumerate()
            .map(|(i, &field)| {
                let mut counts: Vec<(String, usize)> = Vec::new();
                for md in metadata.iter().flatten() {
                    for name in field.get(md) {
                        match counts.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                            Some((_, c)) => *c += 1,
                            None => counts.push((name.clone(), 1)),
                        }
                    }
                }
                counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
                let input = new_input(window, cx, "", tr!("Add to all…"));
                cx.subscribe_in(&input, window, move |this, state, ev: &InputEvent, window, cx| {
                    // The count of changes includes what is typed.
                    if matches!(ev, InputEvent::Change) {
                        cx.notify();
                    }
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        let name = state.read(cx).value().trim().to_string();
                        if !name.is_empty() {
                            this.add_name(i, name);
                            state.update(cx, |s, cx| s.set_value("", window, cx));
                            cx.notify();
                        }
                    }
                })
                .detach();
                TagList { field, counts, add: Vec::new(), remove: Vec::new(), input }
            })
            .collect();
        Self { total, series: series_input, series_initial, series_edited: false, lists, confirm_discard: false }
    }

    fn add_name(&mut self, list: usize, name: String) {
        let l = &mut self.lists[list];
        l.remove.retain(|r| !r.eq_ignore_ascii_case(&name));
        let everywhere = l.counts.iter().any(|(n, c)| n.eq_ignore_ascii_case(&name) && *c == self.total);
        if !everywhere && !l.add.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            l.add.push(name);
        }
    }

    /// What Apply writes: the chips plus a name still in its box.
    fn edit(&self, cx: &App) -> BatchDetails {
        let series = self.series_edited.then(|| self.series.read(cx).value().trim().to_string());
        let lists = self
            .lists
            .iter()
            .map(|l| {
                let mut add = l.add.clone();
                if let Some(name) = l.pending(self.total, cx) {
                    add.push(name);
                }
                let remove = l.remove.iter().filter(|r| !add.iter().any(|a| a.eq_ignore_ascii_case(r))).cloned().collect();
                (l.field, ListDelta { add, remove })
            })
            .filter(|(_, d)| !d.is_empty())
            .collect();
        BatchDetails { series, lists }
    }

    fn changes(&self, cx: &App) -> usize {
        let edit = self.edit(cx);
        usize::from(edit.series.is_some()) + edit.lists.iter().map(|(_, d)| d.add.len() + d.remove.len()).sum::<usize>()
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let edit = self.edit(cx);
        if !edit.is_empty() {
            cx.emit(BatchEditApply(edit));
        }
        cx.emit(DialogEvent::Close);
    }

    /// Esc, offered by the host before it closes the dialog: with changes it
    /// asks first; a second Esc goes back to editing.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let n = self.changes(cx);
        if self.confirm_discard && n > 0 {
            self.confirm_discard = false;
        } else if n > 0 {
            self.confirm_discard = true;
        } else {
            return false;
        }
        cx.notify();
        true
    }

    /// ×, a click outside and Cancel: closes, or asks first when it would
    /// lose changes.
    fn request_close(&mut self, cx: &mut Context<Self>) {
        if self.changes(cx) > 0 && !self.confirm_discard {
            self.confirm_discard = true;
            cx.notify();
        } else {
            cx.emit(DialogEvent::Close);
        }
    }

    fn chip(id: SharedString, text: SharedString) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(28.))
            .pl(px(10.))
            .pr(px(4.))
            .rounded_full()
            .border_1()
            .border_color(transparent_black())
            .text_size(px(12.5))
            .child(text)
    }

    fn chip_x(id: SharedString) -> Stateful<Div> {
        div()
            .id(id)
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.bg(theme::white_a(0.14)))
            .child(icon(Icon::Close, px(12.), theme::text_muted()))
    }

    fn tag_field(&self, li: usize, cx: &mut Context<Self>) -> Div {
        let l = &self.lists[li];
        let total = self.total;
        let note = |s: String| div().text_size(px(11.)).text_color(theme::text_dim()).child(s);
        let mut chips = div().flex().flex_wrap().gap(px(6.));
        for (ci, (name, n)) in l.counts.iter().enumerate() {
            let id = |part: &str| SharedString::from(format!("be-{li}-{ci}-{part}"));
            let name = name.clone();
            let removed = l.remove.contains(&name);
            let given = l.add.iter().any(|a| a.eq_ignore_ascii_case(&name));
            let chip = Self::chip(id("c"), name.clone().into());
            let el = if removed {
                // Struck through: a click keeps it.
                let keep = name.clone();
                chip.pr(px(10.))
                    .line_through()
                    .text_color(theme::text_dim())
                    .border_color(theme::white_a(0.08))
                    .cursor_pointer()
                    .child(icon(Icon::Replay, px(13.), theme::text_dim()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.lists[li].remove.retain(|r| *r != keep);
                        cx.notify();
                    }))
            } else if *n >= total || given {
                let (drop, drop_add) = (name.clone(), name.clone());
                chip.bg(if given { theme::accent_tint() } else { theme::white_a(0.1) })
                    .when(given, |d| d.border_color(theme::accent_tint_border()).child(note(tr!("all {n}", n = total))))
                    .child(Self::chip_x(id("x")).on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let l = &mut this.lists[li];
                        if given {
                            l.add.retain(|a| !a.eq_ignore_ascii_case(&drop_add));
                        } else {
                            l.remove.push(drop.clone());
                        }
                        cx.notify();
                    })))
            } else {
                // Only on some: dashed, a click gives it to all.
                let (give, drop) = (name.clone(), name.clone());
                chip.border_dashed()
                    .border_color(theme::white_a(0.3))
                    .text_color(theme::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.border_color(theme::accent()).text_color(white()))
                    .child(note(format!("{n}/{total}")))
                    .child(Self::chip_x(id("x")).on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.lists[li].remove.push(drop.clone());
                        cx.notify();
                    })))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.add_name(li, give.clone());
                        cx.notify();
                    }))
            };
            chips = chips.child(el);
        }
        for (ai, name) in l.add.iter().enumerate().filter(|(_, a)| !l.counts.iter().any(|(n, _)| n.eq_ignore_ascii_case(a))) {
            let drop = name.clone();
            chips = chips.child(
                Self::chip(SharedString::from(format!("be-{li}-new-{ai}")), name.clone().into())
                    .bg(theme::accent_tint())
                    .border_color(theme::accent_tint_border())
                    .child(note(tr!("new").to_string()))
                    .child(Self::chip_x(SharedString::from(format!("be-{li}-new-{ai}-x"))).on_click(cx.listener(move |this, _, _, cx| {
                        this.lists[li].add.retain(|a| *a != drop);
                        cx.notify();
                    }))),
            );
        }
        let changed = !l.add.is_empty() || !l.remove.is_empty() || l.pending(total, cx).is_some();
        let field = l.field;
        Self::field(
            label_of(field),
            changed,
            cx.listener(move |this, _, window, cx| {
                let l = &mut this.lists[li];
                l.add.clear();
                l.remove.clear();
                l.input.update(cx, |s, cx| s.set_value("", window, cx));
                cx.notify();
            }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .when(!l.counts.is_empty() || !l.add.is_empty(), |d| d.child(chips))
                .child(input_box(&l.input, false)),
        )
    }

    /// A field: its label (with ↺ while it has changes) above `control`.
    fn field(label: SharedString, changed: bool, on_reset: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Div {
        let reset = div()
            .id(SharedString::from(format!("be-reset-{label}")))
            .ml_auto()
            .cursor_pointer()
            .text_color(theme::text_dim())
            .hover(|s| s.text_color(theme::accent()))
            .child(icon(Icon::Replay, px(15.), theme::text_dim()))
            .on_click(on_reset);
        div()
            .flex()
            .flex_col()
            .pl(px(10.))
            .border_l_2()
            .border_color(if changed { theme::accent().into() } else { transparent_black() })
            .child(div().flex().items_center().child(field_label(label).mb(px(0.))).when(changed, |d| d.child(reset)))
            .child(div().mt(px(6.)))
    }
}

impl Render for BatchEditDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let close = {
            let entity = entity.clone();
            move |_: &mut Window, cx: &mut App| {
                entity.update(cx, |this, cx| this.request_close(cx)).ok();
            }
        };
        let header = div()
            .flex()
            .items_start()
            .gap(px(14.))
            .px(px(20.))
            .pt(px(20.))
            .pb(px(6.))
            .child(div().flex().flex_col().flex_1().min_w_0().gap(px(1.)).child(kicker(tr!("Selection"))).child(dialog_title(trn!(
                self.total,
                "Edit {n} game",
                "Edit {n} games"
            ))))
            .child(close_button("be-close-x").on_click({
                let close = close.clone();
                move |_, w, cx| close(w, cx)
            }));
        let series = Self::field(
            tr!("Series").into(),
            self.series_edited,
            cx.listener(|this, _, window, cx| {
                let initial = this.series_initial.clone();
                this.series.update(cx, |s, cx| s.set_value(initial, window, cx));
                this.series_edited = false;
                cx.notify();
            }),
        )
        .child(input_box(&self.series, false));
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(help_text(tr!(
                "Only what you change is written. Empty fields differ between the games and stay as they are. A dashed tag is on some of them: click it to give it to all; × takes a tag out of all of them."
            )))
            .child(series);
        for li in 0..self.lists.len() {
            body = body.child(self.tag_field(li, cx));
        }
        let n = self.changes(cx);
        // Changes undone meanwhile: nothing left to ask about.
        let footer = if self.confirm_discard && n > 0 {
            div()
                .flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(theme::text_primary()).child(trn!(
                    n,
                    "Discard {n} change?",
                    "Discard {n} changes?"
                )))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(button("be-discard-back", ButtonKind::Ghost, tr!("Keep editing")).on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_discard = false;
                            cx.notify();
                        })))
                        .child(
                            button("be-discard-yes", ButtonKind::Danger, tr!("Discard"))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
                        ),
                )
        } else {
            div()
                .flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(if n == 0 {
                    tr!("No changes yet").to_string()
                } else {
                    trn!(n, "{n} change", "{n} changes")
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            button("be-cancel", ButtonKind::Glass, tr!("Cancel"))
                                .on_click(cx.listener(|this, _, _, cx| this.request_close(cx))),
                        )
                        .child(action_button(
                            "be-apply",
                            ButtonKind::Primary,
                            trn!(self.total, "Apply to {n} game", "Apply to {n} games"),
                            n > 0,
                            cx.listener(|this, _, _, cx| this.apply(cx)),
                        )),
                )
        };
        let vw = f32::from(window.viewport_size().width);
        div().absolute().inset_0().child(drawer(
            DrawerProps { id: "collection-batch-edit".into(), width: 540f32.min(vw * 0.96) },
            header.into_any_element(),
            body.into_any_element(),
            Some(footer.into_any_element()),
            close,
        ))
    }
}
