//! The editor's panel: name and colour, the rows of the rules, the games
//! they hold, the pin and the buttons.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::categories::UNCATEGORIZED;
use heronite_data::smart_groups::{self, Cond, Field, Op, ValueKind};
use heronite_data::status::status_label;
use heronite_data::FacetKind;
use heronite_i18n::{tr, trc, trn};

use super::{List, Part, Row, SmartGroupDialog};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button, help_text, icon_action_button, row, segmented, toggle, ButtonKind};
use crate::ui::dialog::{dialog, DialogProps};
use crate::ui::fields::{field_label, input_box, select_field, select_menu_field, SelectOption};
use crate::ui::icon;

/// A list of values this long gets a search box.
const SEARCH_FROM: usize = 9;

pub(super) fn field_name(field: Field) -> &'static str {
    match field {
        Field::Installed => tr!("Installed"),
        Field::Favourite => tr!("Favourite"),
        Field::Played => tr!("Played"),
        Field::HasUpdate => tr!("Update waiting"),
        Field::RunsOffline => tr!("Runs offline"),
        Field::Hidden => tr!("Hidden"),
        Field::HasNotes => tr!("Has notes"),
        Field::AlsoOnSteam => tr!("Also on Steam"),
        Field::FamilyShared => tr!("Shared by Steam Family"),
        Field::Status => tr!("Status"),
        Field::Category => tr!("Category"),
        Field::System => tr!("System"),
        Field::Playtime => tr!("Time played"),
        Field::RecentPlaytime => tr!("Played in the last 30 days"),
        Field::TimesPlayed => tr!("Times played"),
        Field::SessionLength => tr!("Average session"),
        Field::LastPlayed => tr!("Last played"),
        Field::FirstPlayed => tr!("First played"),
        Field::Progress => tr!("Progress to beat it"),
        Field::AgeRating => tr!("Age rating"),
        Field::Hltb => tr!("Time to beat"),
        Field::Added => tr!("Date added"),
        Field::ReleaseYear => tr!("Release year"),
        Field::Rating => tr!("Your rating"),
        Field::CriticScore => tr!("Critic score"),
        Field::InstallSize => tr!("Size on disk"),
        Field::Facet(kind) => match kind {
            FacetKind::Series => tr!("Series"),
            FacetKind::Developer => tr!("Developer"),
            FacetKind::Publisher => tr!("Publisher"),
            FacetKind::Genre => tr!("Genre"),
            FacetKind::Platform => tr!("Platform"),
            FacetKind::Store => tr!("Store"),
            FacetKind::Theme => tr!("Theme"),
            FacetKind::Feature => tr!("Feature"),
            FacetKind::GameMode => tr!("Game mode"),
            FacetKind::Tag => tr!("Steam tag"),
        },
    }
}

/// The fields in the order of their menu, under their headings.
fn field_menu() -> Vec<(Field, &'static str)> {
    let game = tr!("The game");
    let play = tr!("How you play it");
    let details = tr!("Details");
    let fields = tr!("Library fields");
    let mut out = vec![
        (Field::Installed, game),
        (Field::Favourite, game),
        (Field::Status, game),
        (Field::Category, game),
        (Field::Hidden, game),
        (Field::HasNotes, game),
        (Field::HasUpdate, game),
        (Field::RunsOffline, game),
        (Field::AlsoOnSteam, game),
        (Field::FamilyShared, game),
        (Field::Played, play),
        (Field::Playtime, play),
        (Field::RecentPlaytime, play),
        (Field::TimesPlayed, play),
        (Field::SessionLength, play),
        (Field::LastPlayed, play),
        (Field::FirstPlayed, play),
        (Field::Progress, play),
        (Field::Hltb, details),
        (Field::Added, details),
        (Field::ReleaseYear, details),
        (Field::Rating, details),
        (Field::CriticScore, details),
        (Field::AgeRating, details),
        (Field::InstallSize, details),
        (Field::System, details),
    ];
    out.extend(heronite_data::collection::FACET_KINDS.map(|k| (Field::Facet(k), fields)));
    out
}

/// The ways a field compares, with their words.
fn op_menu(field: Field) -> Vec<(Op, &'static str)> {
    match field.kind() {
        ValueKind::Flag => vec![(Op::Is(true), trc!("rule", "yes")), (Op::Is(false), trc!("rule", "no"))],
        ValueKind::Names => Vec::new(),
        ValueKind::Number if field == Field::ReleaseYear => {
            vec![(Op::Lt(0.), trc!("rule", "before")), (Op::Gt(0.), trc!("rule", "after")), (Op::Eq(0.), trc!("rule", "in"))]
        }
        ValueKind::Number => {
            vec![(Op::Lt(0.), trc!("rule", "less than")), (Op::Gt(0.), trc!("rule", "more than")), (Op::Eq(0.), trc!("rule", "exactly"))]
        }
    }
}

/// Same comparison, whatever the value.
fn same_op(a: &Op, b: &Op) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b) && (!matches!(a, Op::Is(_)) || a == b)
}

/// `op` with the value of `old` (a number keeps its value).
fn with_value(op: &Op, old: &Op) -> Op {
    let n = match old {
        Op::Lt(n) | Op::Gt(n) | Op::Eq(n) => *n,
        _ => 0.,
    };
    match op {
        Op::Lt(_) => Op::Lt(n),
        Op::Gt(_) => Op::Gt(n),
        Op::Eq(_) => Op::Eq(n),
        other => other.clone(),
    }
}

/// What follows a number.
fn unit(field: Field) -> &'static str {
    match field {
        Field::Playtime | Field::Hltb | Field::RecentPlaytime => tr!("hours"),
        Field::LastPlayed | Field::FirstPlayed | Field::Added => tr!("days ago"),
        Field::InstallSize => "GiB",
        Field::Rating => tr!("stars"),
        Field::TimesPlayed => tr!("sessions"),
        Field::SessionLength => tr!("minutes"),
        Field::Progress => "%",
        Field::AgeRating => tr!("years"),
        _ => "",
    }
}

impl SmartGroupDialog {
    /// Every value a list condition on `field` can take: `(value, label, count)`.
    fn values_of(&self, field: Field) -> Vec<(String, String, Option<usize>)> {
        match field {
            Field::Status => self.ctx.lib.statuses.iter().map(|s| (s.id.clone(), status_label(&s.name).to_string(), None)).collect(),
            Field::Category => self
                .facts
                .categories
                .names()
                .map(|n| (n.to_owned(), n.to_owned(), None))
                .chain([(UNCATEGORIZED.to_owned(), tr!("Uncategorized").to_owned(), None)])
                .collect(),
            Field::Facet(kind) => self.options.get(kind).iter().map(|o| (o.value.clone(), o.value.clone(), Some(o.count))).collect(),
            Field::System => {
                let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
                for system in self.facts.extras.systems.values().filter(|s| !s.is_empty()) {
                    *counts.entry(system).or_default() += 1;
                }
                counts.into_iter().map(|(s, n)| (s.to_owned(), s.to_owned(), Some(n))).collect()
            }
            _ => Vec::new(),
        }
    }

    /// How a value reads (a status by its name).
    fn value_label(&self, field: Field, value: &str) -> String {
        match field {
            Field::Status => self.ctx.lib.statuses.iter().find(|s| s.id == value).map_or(value, |s| status_label(&s.name)).to_string(),
            Field::Category if value == UNCATEGORIZED => tr!("Uncategorized").to_string(),
            _ => value.to_owned(),
        }
    }

    fn render_row(&self, list: List, i: usize, r: &Row, cx: &mut Context<Self>) -> Div {
        let key = match list {
            List::Match => "m",
            List::Except => "x",
        };
        let id = |part: &str| format!("sg-{key}-{i}-{part}");
        let is_open = |part: Part| self.open == Some((list, i, part));
        let cond = &r.cond;
        let fields = field_menu();
        let field_options = fields
            .iter()
            .map(|(f, group)| SelectOption {
                label: field_name(*f).into(),
                selected: *f == cond.field,
                group: Some((*group).into()),
                ..Default::default()
            })
            .collect();
        let field = select_menu_field(
            &id("field"),
            None,
            field_name(cond.field),
            false,
            is_open(Part::Field),
            field_options,
            Some(&self.filter),
            cx,
            move |this, w, cx| this.toggle_menu(list, i, Part::Field, w, cx),
            move |this, at, w, cx| {
                if let Some((f, _)) = field_menu().get(at) {
                    this.set_field(list, i, *f, w, cx);
                }
            },
        );
        let mut el = div().flex().items_start().gap(px(8.)).child(div().w(px(215.)).flex_none().child(field));
        let ops = op_menu(cond.field);
        if !ops.is_empty() {
            let current = ops.iter().find(|(op, _)| same_op(op, &cond.op)).map_or("", |(_, l)| *l);
            let options = ops.iter().map(|(op, label)| (SharedString::from(*label), same_op(op, &cond.op))).collect();
            let old = cond.op.clone();
            let picks: Rc<Vec<Op>> = Rc::new(ops.into_iter().map(|(op, _)| with_value(&op, &old)).collect());
            let op = select_field(
                &id("op"),
                None,
                current,
                false,
                is_open(Part::Op),
                options,
                cx,
                move |this, w, cx| this.toggle_menu(list, i, Part::Op, w, cx),
                move |this, at, _, cx| {
                    if let Some(op) = picks.get(at) {
                        this.set_op(list, i, op.clone(), cx);
                    }
                },
            );
            el = el.child(div().w(px(130.)).flex_none().child(op));
        }
        let value = match (&cond.op, r.number.as_ref()) {
            (Op::In(selected), _) => {
                Some(self.render_values(list, i, cond, selected, &id("value"), is_open(Part::Value), cx).into_any_element())
            }
            (_, Some(input)) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(96.)).flex_none().child(input_box(input, false)))
                    .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(unit(cond.field)))
                    .into_any_element(),
            ),
            _ => None,
        };
        el = el.child(div().flex_1().min_w_0().children(value));
        el.child(
            div().pt(px(6.)).child(
                crate::pages::icon_action(SharedString::from(id("del")), Icon::Close, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_row(list, i, cx))),
            ),
        )
    }

    /// The values of a list condition: a select whose menu ticks several.
    #[allow(clippy::too_many_arguments)]
    fn render_values(&self, list: List, i: usize, cond: &Cond, selected: &[String], id: &str, open: bool, cx: &mut Context<Self>) -> Div {
        let mut values = self.values_of(cond.field);
        // Values no game has any more stay listed, to take them out.
        for v in selected.iter().rev() {
            if !values.iter().any(|(value, _, _)| value == v) {
                values.insert(0, (v.clone(), self.value_label(cond.field, v), Some(0)));
            }
        }
        let shown = if selected.is_empty() {
            tr!("Choose…").to_string()
        } else {
            selected.iter().map(|v| self.value_label(cond.field, v)).collect::<Vec<_>>().join(", ")
        };
        let options = values
            .iter()
            .map(|(value, label, count)| SelectOption {
                label: label.clone().into(),
                selected: selected.contains(value),
                note: count.map(|n| n.to_string().into()),
                ..Default::default()
            })
            .collect();
        let filter = (values.len() >= SEARCH_FROM).then_some(&self.filter);
        let values: Rc<Vec<String>> = Rc::new(values.into_iter().map(|(v, _, _)| v).collect());
        select_menu_field(
            id,
            None,
            shown,
            false,
            open,
            options,
            filter,
            cx,
            move |this, w, cx| this.toggle_menu(list, i, Part::Value, w, cx),
            move |this, at, _, cx| {
                if let Some(v) = values.get(at) {
                    this.toggle_value(list, i, v.clone(), cx);
                }
            },
        )
    }

    fn render_list(&self, list: List, cx: &mut Context<Self>) -> Div {
        let rows = self.list(list);
        let add = match list {
            List::Match => tr!("Add a condition"),
            List::Except => tr!("Leave out more games"),
        };
        div().flex().flex_col().gap(px(8.)).children(rows.iter().enumerate().map(|(i, r)| self.render_row(list, i, r, cx))).child(
            div()
                .id(match list {
                    List::Match => "sg-add-match",
                    List::Except => "sg-add-except",
                })
                .flex()
                .items_center()
                .gap(px(6.))
                .py(px(4.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(theme::accent())
                .hover(|s| s.text_color(theme::text_bright()))
                .child(icon(Icon::Add, px(16.), theme::accent()))
                .child(add)
                .on_click(cx.listener(move |this, _, w, cx| this.add_row(list, w, cx))),
        )
    }

    fn render_colours(&self, cx: &mut Context<Self>) -> Div {
        let swatch = |id: SharedString, colour: Option<&'static str>, on: bool| {
            let fill = colour.and_then(theme::parse_hex).unwrap_or_else(theme::accent);
            div()
                .id(id)
                .flex_none()
                .size(px(18.))
                .rounded_full()
                .bg(fill)
                .cursor_pointer()
                .border_2()
                .border_color(if on { theme::text_bright() } else { theme::ink_a(0.) })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.colour = colour.map(str::to_owned);
                    cx.notify();
                }))
        };
        let current = self.colour.as_deref();
        div().flex().items_center().gap(px(8.)).child(swatch("sg-colour-accent".into(), None, current.is_none())).children(
            theme::GROUP_COLOURS
                .iter()
                .enumerate()
                .map(|(i, c)| swatch(SharedString::from(format!("sg-colour-{i}")), Some(*c), current == Some(*c))),
        )
    }

    /// The statuses and categories the rules name that this PC does not have
    /// (made on another PC that syncs, or removed here).
    fn render_missing(&self) -> Option<Div> {
        let rule = self.rule();
        let statuses = &self.ctx.lib.statuses;
        let mut missing: Vec<String> = smart_groups::names_used(&rule, Field::Status)
            .into_iter()
            .filter(|id| !statuses.iter().any(|s| s.id == *id))
            .map(str::to_owned)
            .collect();
        missing.extend(smart_groups::missing_categories(&rule, &self.facts.categories).into_iter().map(str::to_owned));
        (!missing.is_empty()).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.5))
                .text_color(theme::warning())
                .child(icon(Icon::Warning, px(15.), theme::warning()))
                .child(tr!("Not on this PC: {names}. These conditions match no game here.", names = missing.join(", ")).to_string())
        })
    }

    fn render_count(&self) -> Div {
        let titles = match self.sample.as_slice() {
            [] => tr!("No game matches these rules yet.").to_string(),
            some if self.count > some.len() => {
                tr!("{titles} and {n} more", titles = some.join(", "), n = self.count - some.len()).to_string()
            }
            some => some.join(", "),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p(px(14.))
            .rounded(px(theme::R_TILE))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_bright()).child(trn!(
                self.count,
                "{n} game",
                "{n} games"
            )))
            .child(div().text_size(px(12.5)).text_color(theme::text_muted()).line_clamp(2).child(titles))
    }
}

impl Render for SmartGroupDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(help_text(tr!("A smart category keeps itself up to date: games join and leave it as they match its rules.")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(div().flex_1().min_w_0().child(input_box(&self.name, false)))
                    .child(self.render_colours(cx)),
            );
        if self.editable {
            let any = self.any;
            body = body
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .flex_wrap()
                                .gap(px(8.))
                                .text_size(px(13.5))
                                .text_color(theme::text_primary())
                                .child(tr!("Games that match"))
                                .child(segmented("sg-any", &[(false, trc!("rule", "all")), (true, trc!("rule", "any"))], any, true, {
                                    let weak = weak.clone();
                                    move |any, _, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.any = any;
                                            this.changed(cx);
                                        })
                                        .ok();
                                    }
                                }))
                                .child(tr!("of these conditions")),
                        )
                        .child(self.render_list(List::Match, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(field_label(tr!("Except games that match any of these").to_string()))
                        .child(self.render_list(List::Except, cx)),
                );
        } else {
            body = body.child(help_text(tr!(
                "The rules of this category are nested deeper than this editor shows. They are kept as they are; the name, colour and pin can change."
            )));
        }
        body = body.children(self.render_missing()).child(self.render_count()).child(row(
            tr!("Pin to the Collection header"),
            Some(tr!("One click away, next to the filters. Unpinned categories are in the View menu.")),
            toggle("sg-pin", self.pinned, false).on_click(cx.listener(|this, _, _, cx| {
                this.pinned = !this.pinned;
                cx.notify();
            })),
        ));
        if let Some(err) = self.error.clone() {
            body = body.child(div().text_size(px(13.)).text_color(theme::danger_text()).child(err));
        }
        let left = if self.is_new {
            None
        } else if self.confirm_delete {
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme::text_primary())
                            .child(tr!("Delete this smart category? Its games stay.")),
                    )
                    .child(button("sg-del-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_delete = false;
                        cx.notify();
                    })))
                    .child(button("sg-del-yes", ButtonKind::Danger, tr!("Delete")).on_click(cx.listener(|this, _, _, cx| this.delete(cx)))),
            )
        } else {
            Some(div().child(icon_action_button(
                "sg-del",
                ButtonKind::Danger,
                Icon::DeleteOutline,
                tr!("Delete category"),
                cx.listener(|this, _, _, cx| {
                    this.confirm_delete = true;
                    cx.notify();
                }),
            )))
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .children(left)
            .child(div().flex_1())
            .when(!self.confirm_delete, |d| {
                d.child(
                    button("sg-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|_, _, _, cx| cx.emit(DialogEvent::Close))),
                )
                .child(button("sg-save", ButtonKind::Primary, tr!("Save")).on_click(cx.listener(|this, _, _, cx| this.save(cx))))
            })
            .into_any_element();
        let title = if self.is_new { tr!("New smart category") } else { tr!("Edit smart category") };
        dialog(
            DialogProps { id: "smart-group".into(), title: title.into(), width: 720., height: None },
            body.into_any_element(),
            Some(footer),
            move |_, cx| {
                weak.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{field_menu, same_op, with_value};
    use heronite_data::smart_groups::{Field, Op};

    #[test]
    fn every_field_has_a_place_in_the_menu() {
        let menu: Vec<Field> = field_menu().into_iter().map(|(f, _)| f).collect();
        for field in Field::all() {
            assert!(menu.contains(&field), "{field:?}");
        }
        assert_eq!(menu.len(), Field::all().len());
    }

    #[test]
    fn changing_the_comparison_keeps_the_number() {
        assert_eq!(with_value(&Op::Gt(0.), &Op::Lt(8.)), Op::Gt(8.));
        assert_eq!(with_value(&Op::Is(false), &Op::Is(true)), Op::Is(false));
        assert!(same_op(&Op::Lt(1.), &Op::Lt(9.)));
        assert!(!same_op(&Op::Is(true), &Op::Is(false)));
    }
}
