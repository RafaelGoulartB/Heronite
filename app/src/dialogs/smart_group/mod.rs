//! `SmartGroupDialog`: makes or changes a smart group
//! (`heronite_data::smart_groups`). Its name and colour, whether it is
//! pinned to the Collection's header, and its rules as rows: conditions that
//! must all (or any) match, then the games to leave out. The number of games
//! it holds follows every change. A rule nested deeper than that (written by
//! hand or by a newer version) is kept as it is, and only the name, colour
//! and pin can change. Drawn in `render.rs`.

mod render;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use heronite_data::categories::Categories;
use heronite_data::collection::{collect_facet_options, FacetOptions};
use heronite_data::hltb::WikiCache;
use heronite_data::organize::{GameExtras, Organize};
use heronite_data::smart_groups::{self, Cond, Facts, Field, Op, Rule, Simple, SmartGroup, ValueKind};
use heronite_i18n::tr;

use super::{DialogCtx, DialogEvent};

/// What the rules read besides the games, copied from the Collection.
pub struct GroupCtx {
    pub categories: Categories,
    pub extras: Arc<GameExtras>,
    pub ratings: HashMap<String, u8>,
    pub wiki: Arc<WikiCache>,
    pub updates: HashSet<String>,
}

/// The groups file changed; `show`: the Collection shows this group now (a
/// new one, or one just changed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSaved {
    pub show: Option<String>,
}

/// The two lists of rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    /// What a game must match.
    Match,
    /// What leaves a game out.
    Except,
}

/// The parts of a row that open a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Field,
    Op,
    Value,
}

struct Row {
    cond: Cond,
    /// The box of a number condition.
    number: Option<Entity<InputState>>,
}

pub struct SmartGroupDialog {
    ctx: DialogCtx,
    facts: GroupCtx,
    /// As it was opened: its id, pin and the keys this version does not read.
    group: SmartGroup,
    is_new: bool,
    name: Entity<InputState>,
    colour: Option<String>,
    pinned: bool,
    /// `false`: the rule is nested deeper than the rows show and stays as it is.
    editable: bool,
    any: bool,
    rows: Vec<Row>,
    except: Vec<Row>,
    /// The menu open: which list, row and part.
    open: Option<(List, usize, Part)>,
    /// Search box of a long list of values.
    filter: Entity<InputState>,
    /// Values of every library field, with counts.
    options: FacetOptions,
    /// Games the rules hold now, and the first titles.
    count: usize,
    sample: Vec<String>,
    error: Option<SharedString>,
    confirm_delete: bool,
}

impl EventEmitter<DialogEvent> for SmartGroupDialog {}
impl EventEmitter<GroupSaved> for SmartGroupDialog {}

fn text_input(window: &mut Window, cx: &mut App, value: &str, placeholder: &str) -> Entity<InputState> {
    let (value, placeholder) = (value.to_string(), placeholder.to_string());
    cx.new(|cx| {
        let mut s = InputState::new(window, cx);
        s.set_placeholder(placeholder, window, cx);
        s.set_value(value, window, cx);
        s
    })
}

/// A number as typed in a box (`1,5` reads as `1.5`); empty or not a number: `None`.
fn parse_number(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|n| n.is_finite())
}

/// How a number is shown in its box: no ".0".
fn number_text(n: f64) -> String {
    if n.fract() == 0. {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

/// A new condition on `field`, with a value worth starting from.
fn default_cond(field: Field) -> Cond {
    let op = match field {
        Field::Playtime => Op::Lt(1.),
        Field::LastPlayed => Op::Gt(180.),
        Field::Hltb => Op::Lt(10.),
        Field::Added => Op::Lt(30.),
        Field::ReleaseYear => Op::Gt(2020.),
        Field::Rating => Op::Gt(3.),
        Field::CriticScore => Op::Gt(80.),
        Field::InstallSize => Op::Gt(50.),
        Field::FirstPlayed => Op::Gt(365.),
        Field::TimesPlayed => Op::Lt(3.),
        Field::SessionLength => Op::Lt(30.),
        Field::RecentPlaytime => Op::Gt(1.),
        Field::Progress => Op::Gt(50.),
        Field::AgeRating => Op::Lt(13.),
        _ => return Cond::empty(field),
    };
    Cond::new(field, op)
}

/// The number a condition compares with.
fn op_number(op: &Op) -> Option<f64> {
    match op {
        Op::Lt(n) | Op::Gt(n) | Op::Eq(n) => Some(*n),
        _ => None,
    }
}

impl SmartGroupDialog {
    pub fn new(ctx: DialogCtx, facts: GroupCtx, group: SmartGroup, is_new: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = text_input(window, cx, &group.name, tr!("Name of the category"));
        let filter = text_input(window, cx, "", tr!("Search"));
        cx.subscribe(&filter, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let simple = Simple::of(&group.rule);
        let options = collect_facet_options(&ctx.lib.games);
        let mut this = Self {
            colour: group.colour.clone(),
            pinned: group.pinned,
            editable: simple.is_some(),
            any: simple.as_ref().is_some_and(|s| s.any),
            rows: Vec::new(),
            except: Vec::new(),
            open: None,
            filter,
            options,
            count: 0,
            sample: Vec::new(),
            error: None,
            confirm_delete: false,
            ctx,
            facts,
            group,
            is_new,
            name,
        };
        if let Some(simple) = simple {
            this.rows = simple.conds.into_iter().map(|c| this.row(c, window, cx)).collect();
            this.except = simple.except.into_iter().map(|c| this.row(c, window, cx)).collect();
        }
        this.recount();
        // `HERONITE_DEBUG_SMARTGROUP_MENU=field[:<text>]`: the first row's field menu open, `<text>` searched.
        if let Some(spec) = crate::env::value("HERONITE_DEBUG_SMARTGROUP_MENU") {
            if !this.rows.is_empty() && spec.starts_with("field") {
                this.open = Some((List::Match, 0, Part::Field));
                let text = spec.strip_prefix("field:").unwrap_or_default().to_string();
                this.filter.update(cx, |s, cx| s.set_value(text, window, cx));
            }
        }
        this
    }

    fn row(&self, cond: Cond, window: &mut Window, cx: &mut Context<Self>) -> Row {
        let number = op_number(&cond.op).map(|n| {
            let input = text_input(window, cx, &number_text(n), "0");
            cx.subscribe(&input, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let value = parse_number(&input.read(cx).value());
                    this.set_number(&input, value, cx);
                }
            })
            .detach();
            input
        });
        Row { cond, number }
    }

    fn list(&self, list: List) -> &Vec<Row> {
        match list {
            List::Match => &self.rows,
            List::Except => &self.except,
        }
    }

    fn list_mut(&mut self, list: List) -> &mut Vec<Row> {
        match list {
            List::Match => &mut self.rows,
            List::Except => &mut self.except,
        }
    }

    /// The rule the rows make now; the stored one when they cannot show it.
    fn rule(&self) -> Rule {
        if !self.editable {
            return self.group.rule.clone();
        }
        let cond = |r: &Row| r.cond.clone();
        Simple { any: self.any, conds: self.rows.iter().map(cond).collect(), except: self.except.iter().map(cond).collect() }.to_rule()
    }

    /// Counts the games of the rules (after every change, not every frame).
    fn recount(&mut self) {
        let rule = self.rule();
        let lib = self.ctx.lib.clone();
        let facts = Facts {
            organize: Organize {
                extras: Some(&self.facts.extras),
                ratings: Some(&self.facts.ratings),
                categories: Some(&self.facts.categories),
                statuses: &lib.statuses,
            },
            wiki: Some(&self.facts.wiki),
            updates: Some(&self.facts.updates),
            today: chrono::Local::now().date_naive(),
        };
        let games: Vec<_> = lib.games.iter().filter(|g| rule.matches(g, &facts)).collect();
        self.count = games.len();
        self.sample = games.iter().take(4).map(|g| g.title.clone()).collect();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.recount();
        cx.notify();
    }

    fn set_number(&mut self, input: &Entity<InputState>, value: Option<f64>, cx: &mut Context<Self>) {
        let Some(value) = value else { return };
        let row = self.rows.iter_mut().chain(self.except.iter_mut()).find(|r| r.number.as_ref() == Some(input));
        if let Some(row) = row {
            row.cond.op = match row.cond.op {
                Op::Lt(_) => Op::Lt(value),
                Op::Gt(_) => Op::Gt(value),
                _ => Op::Eq(value),
            };
            self.changed(cx);
        }
    }

    fn toggle_menu(&mut self, list: List, i: usize, part: Part, window: &mut Window, cx: &mut Context<Self>) {
        let at = Some((list, i, part));
        self.open = if self.open == at { None } else { at };
        // The fields' menu has a search box (a long list of values too): typing goes there.
        let search = self.open.is_some() && (part == Part::Field || part == Part::Value);
        self.filter.update(cx, |s, cx| {
            s.set_value("", window, cx);
            if search {
                s.focus(window, cx);
            }
        });
        cx.notify();
    }

    fn add_row(&mut self, list: List, window: &mut Window, cx: &mut Context<Self>) {
        let field = if list == List::Except { Field::Favourite } else { Field::Installed };
        let row = self.row(default_cond(field), window, cx);
        self.list_mut(list).push(row);
        let i = self.list(list).len() - 1;
        // The field is what to pick first.
        self.open = Some((list, i, Part::Field));
        self.changed(cx);
    }

    fn remove_row(&mut self, list: List, i: usize, cx: &mut Context<Self>) {
        let rows = self.list_mut(list);
        if i < rows.len() {
            rows.remove(i);
        }
        self.open = None;
        self.changed(cx);
    }

    fn set_field(&mut self, list: List, i: usize, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        if self.list(list).get(i).is_none_or(|r| r.cond.field == field) {
            self.open = None;
            cx.notify();
            return;
        }
        let row = self.row(default_cond(field), window, cx);
        self.list_mut(list)[i] = row;
        // A list of values is picked next.
        self.open = (field.kind() == ValueKind::Names).then_some((list, i, Part::Value));
        self.filter.update(cx, |s, cx| s.set_value("", window, cx));
        self.changed(cx);
    }

    fn set_op(&mut self, list: List, i: usize, op: Op, cx: &mut Context<Self>) {
        if let Some(row) = self.list_mut(list).get_mut(i) {
            row.cond.op = op;
        }
        self.open = None;
        self.changed(cx);
    }

    /// Adds or takes out one value of a list condition (its menu stays open).
    fn toggle_value(&mut self, list: List, i: usize, value: String, cx: &mut Context<Self>) {
        if let Some(Row { cond: Cond { op: Op::In(values), .. }, .. }) = self.list_mut(list).get_mut(i) {
            match values.iter().position(|v| *v == value) {
                Some(at) => {
                    values.remove(at);
                }
                None => values.push(value),
            }
        }
        self.changed(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some(tr!("The category needs a name").into());
            cx.notify();
            return;
        }
        let empty_list = self.rows.iter().chain(&self.except).any(|r| matches!(&r.cond.op, Op::In(v) if v.is_empty()));
        if self.editable && empty_list {
            self.error = Some(tr!("Pick at least one value in each condition, or remove it.").into());
            cx.notify();
            return;
        }
        let group = SmartGroup { name, colour: self.colour.clone(), pinned: self.pinned, rule: self.rule(), ..self.group.clone() };
        let id = group.id.clone();
        match smart_groups::upsert(&self.ctx.data_paths, group) {
            Ok(()) => {
                cx.emit(GroupSaved { show: Some(id) });
                cx.emit(DialogEvent::Close);
            }
            Err(err) => {
                self.error = Some(format!("{err:#}").into());
                cx.notify();
            }
        }
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        match smart_groups::remove(&self.ctx.data_paths, &self.group.id) {
            Ok(()) => {
                cx.emit(GroupSaved { show: None });
                cx.emit(DialogEvent::Close);
            }
            Err(err) => {
                self.error = Some(format!("{err:#}").into());
                self.confirm_delete = false;
                cx.notify();
            }
        }
    }

    /// Esc, offered by the host before it closes the dialog: closes an open
    /// menu or the question about deleting.
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let used = self.open.take().is_some() || std::mem::take(&mut self.confirm_delete);
        if used {
            cx.notify();
        }
        used
    }
}

#[cfg(test)]
mod tests {
    use super::{default_cond, number_text, op_number, parse_number};
    use heronite_data::smart_groups::{Field, Op, ValueKind};

    #[test]
    fn numbers_read_as_typed() {
        assert_eq!(parse_number(" 1,5 "), Some(1.5));
        assert_eq!(parse_number("50"), Some(50.));
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("abc"), None);
        assert_eq!(number_text(8.), "8");
        assert_eq!(number_text(2.5), "2.5");
    }

    #[test]
    fn a_new_condition_starts_from_a_useful_value() {
        assert_eq!(default_cond(Field::LastPlayed).op, Op::Gt(180.));
        assert_eq!(default_cond(Field::Installed).op, Op::Is(true));
        assert_eq!(default_cond(Field::Facet(heronite_data::FacetKind::Genre)).op, Op::In(Vec::new()));
        for field in Field::all() {
            assert_eq!(op_number(&default_cond(field).op).is_some(), field.kind() == ValueKind::Number, "{field:?}");
        }
    }
}
