//! What a press asks for ([`activate`]) and applying it ([`run`]).

use super::*;

/// A theme operation that reads or writes files (run by [`host`]).
#[derive(Clone, Debug, PartialEq)]
pub enum StoreOp {
    /// Overwrite the saved theme in use.
    Save,
    SaveAs {
        name: String,
        author: String,
    },
    Rename(String),
    Delete,
    Export,
    Import,
}

/// What a press on an action / text row asks the host for.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Nothing,
    Run(Command),
    /// Type a text, then [`Command`] `id` with [`Arg::Text`] (the theme's
    /// save-as and rename go to [`StoreOp`] instead).
    Text {
        id: String,
        title: String,
        initial: String,
    },
    /// Pick a picture file.
    File {
        id: String,
        title: String,
        start: Option<PathBuf>,
    },
    /// Pick one of `rows`, then [`Command`] `id` with [`Arg::Pick`].
    Pick {
        id: String,
        title: String,
        rows: Vec<Row>,
    },
    /// Ask first; yes runs [`Step::Store`].
    Confirm {
        title: String,
        message: String,
        yes: String,
        then: StoreOp,
    },
    Store(StoreOp),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Value(Value),
    Text(String),
    Press,
    /// Earlier (`-1`) or later (`1`) in its list.
    Move(i32),
    Remove,
    Pick(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub id: String,
    pub arg: Arg,
}

impl Command {
    pub fn new(id: impl Into<String>, arg: Arg) -> Self {
        Self { id: id.into(), arg }
    }

    /// A row's value changed.
    pub fn changed(id: &str, value: Value) -> Self {
        Self::new(id, Arg::Value(value))
    }
}

/// `tab.2.name` with `("tab.", ".name")` gives 2.
fn indexed(id: &str, prefix: &str, suffix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.strip_suffix(suffix)?.parse().ok()
}

fn tab_index(id: &str, suffix: &str) -> Option<usize> {
    indexed(id, "tab.", suffix)
}

fn flat_index(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
}

/// What A on `id` does.
pub fn activate(id: &str, cx: &Ctx) -> Step {
    let p = cx.prefs;
    let press = || Step::Run(Command::new(id, Arg::Press));
    match id {
        "theme.save" => return Step::Store(StoreOp::Save),
        "theme.save-as" => {
            return Step::Text { id: id.into(), title: tr!("Name of the theme").into(), initial: String::new() };
        }
        "theme.rename" => {
            let initial = based_on_saved(p, &cx.data.saved).map(|t| t.name.clone()).unwrap_or_default();
            return Step::Text { id: id.into(), title: tr!("New name").into(), initial };
        }
        "theme.delete" => {
            let name = based_on_saved(p, &cx.data.saved).map(|t| t.name.clone()).unwrap_or_default();
            return Step::Confirm {
                title: tr!("Delete “{name}”?", name),
                message: tr!("The theme file is deleted. The look on screen stays as it is.").into(),
                yes: tr!("Delete").into(),
                then: StoreOp::Delete,
            };
        }
        "theme.export" => return Step::Store(StoreOp::Export),
        "theme.import" => return Step::Store(StoreOp::Import),
        "theme.reset" | "tab.add" => return press(),
        "shelf.add" => {
            return Step::Pick { id: id.into(), title: tr!("Add a shelf").into(), rows: pick_rows(cx) };
        }
        "interface.accent_hex" => {
            return Step::Text { id: id.into(), title: tr!("Custom focus colour").into(), initial: p.theme.interface.accent.clone() };
        }
        _ => {}
    }
    if let Some(i) = tab_index(id, ".name") {
        return Step::Text {
            id: id.into(),
            title: tr!("Name of the tab").into(),
            initial: p.theme.tabs.get(i).map(|t| t.name.clone()).unwrap_or_default(),
        };
    }
    if tab_index(id, ".up").is_some() || tab_index(id, ".remove").is_some() {
        return press();
    }
    match opt(id).map(|o| &o.kind) {
        Some(Kind::Text { get, .. }) => {
            let o = opt(id).expect("found above");
            Step::Text { id: id.into(), title: tr(o.label).into(), initial: get(p) }
        }
        Some(Kind::File { get, .. }) => {
            let current = PathBuf::from(get(p));
            Step::File { id: id.into(), title: tr!("Choose a picture").into(), start: current.parent().map(PathBuf::from) }
        }
        _ => Step::Nothing,
    }
}

/// Whether the row can be moved with X and ↑ ↓.
pub fn movable(id: &str) -> bool {
    flat_index(id, "shelf.").is_some() || flat_index(id, "menu.").is_some()
}

/// Whether LT removes the row.
pub fn removable(id: &str) -> bool {
    flat_index(id, "shelf.").is_some()
}

/// Moves `v[i]` one place; gives its new index.
pub fn shift<T>(v: &mut [T], i: usize, dir: i32) -> Option<usize> {
    let to = if dir < 0 { i.checked_sub(1)? } else { i + 1 };
    (to < v.len() && i < v.len()).then(|| {
        v.swap(i, to);
        to
    })
}

fn first_unused_layout(tabs: &[TabSpec]) -> Layout {
    Layout::ALL.iter().copied().find(|l| tabs.iter().all(|t| t.layout != *l)).unwrap_or(Layout::Shelves)
}

/// Applies `cmd` to the prefs; whether anything changed.
pub fn run(prefs: &mut ConsolePrefs, cmd: &Command, data: &Data) -> bool {
    let before = prefs.clone();
    apply(prefs, cmd, data);
    *prefs != before
}

fn apply(prefs: &mut ConsolePrefs, cmd: &Command, data: &Data) {
    let id = cmd.id.as_str();
    let t = &mut prefs.theme;
    match (id, &cmd.arg) {
        ("theme.use", Arg::Value(Value::Choice { index, .. })) => {
            let list = theme_list(&data.saved);
            if let Some((tid, _)) = list.get(*index) {
                if let Some(next) = presets::by_id(tid).or_else(|| data.saved.iter().find(|s| s.id == *tid).cloned()) {
                    prefs.theme = next;
                    prefs.based_on = tid.clone();
                }
            }
        }
        ("theme.reset", Arg::Press) => {
            prefs.theme = presets::classic();
            prefs.based_on = "classic".into();
        }
        ("tab.add", Arg::Press) => {
            if t.tabs.len() < MAX_TABS {
                let layout = first_unused_layout(&t.tabs);
                t.tabs.push(TabSpec { name: tr(layout_name(layout)).to_string(), layout, source: "all".into() });
            }
        }
        ("interface.accent", Arg::Value(Value::Choice { index, .. })) => {
            t.interface.accent = match ACCENTS.get(*index) {
                Some((n, _)) => (*n).to_string(),
                None if accent_index(&t.interface.accent) == ACCENTS.len() => t.interface.accent.clone(),
                None => "#a5dceb".into(),
            };
        }
        ("interface.accent_hex", Arg::Text(s)) => {
            if let Some(c) = normalize_colour(s).filter(|c| !c.is_empty()) {
                t.interface.accent = c;
            }
        }
        ("shelf.add", Arg::Pick(kind)) => {
            if !t.shelves.iter().any(|s| s.kind == *kind) {
                t.shelves.push(ShelfSpec::new(kind, true));
            }
        }
        _ => apply_indexed(prefs, cmd, data),
    }
}

fn apply_indexed(prefs: &mut ConsolePrefs, cmd: &Command, data: &Data) {
    let id = cmd.id.as_str();
    let t = &mut prefs.theme;
    if let Some(i) = tab_index(id, ".name") {
        if let (Arg::Text(s), Some(tab)) = (&cmd.arg, t.tabs.get_mut(i)) {
            tab.name = s.trim().chars().take(20).collect();
        }
    } else if let Some(i) = tab_index(id, ".layout") {
        if let (Arg::Value(Value::Choice { index, .. }), Some(tab)) = (&cmd.arg, t.tabs.get_mut(i)) {
            if let Some(l) = Layout::ALL.get(*index) {
                // A tab still called by its layout's name follows it.
                if tr(&tab.name) == tr(layout_name(tab.layout)) {
                    tab.name = tr(layout_name(*l)).to_string();
                }
                tab.layout = *l;
            }
        }
    } else if let Some(i) = tab_index(id, ".source") {
        if let (Arg::Value(Value::Choice { index, .. }), Some(tab)) = (&cmd.arg, t.tabs.get_mut(i)) {
            if let Some((source, _)) = source_options(&data.facts, &tab.source).get(*index) {
                tab.source = source.clone();
            }
        }
    } else if let Some(i) = tab_index(id, ".up") {
        shift(&mut t.tabs, i, -1);
    } else if let Some(i) = tab_index(id, ".remove") {
        if t.tabs.len() > 1 && i < t.tabs.len() {
            t.tabs.remove(i);
        }
    } else if let Some(i) = flat_index(id, "shelf.") {
        match &cmd.arg {
            Arg::Value(Value::Toggle(on)) => {
                if let Some(s) = t.shelves.get_mut(i) {
                    s.on = *on;
                }
            }
            Arg::Text(title) => {
                if let Some(s) = t.shelves.get_mut(i) {
                    s.title = title.trim().chars().take(32).collect();
                }
            }
            Arg::Move(dir) => {
                shift(&mut t.shelves, i, *dir);
            }
            Arg::Remove if i < t.shelves.len() => {
                t.shelves.remove(i);
            }
            _ => {}
        }
    } else if let Some(i) = flat_index(id, "menu.") {
        match &cmd.arg {
            Arg::Value(Value::Toggle(on)) => {
                if let Some(m) = t.menu.items.get_mut(i).filter(|m| !MENU_LOCKED.contains(&m.item.as_str()) && m.item != "separator") {
                    m.on = *on;
                }
            }
            Arg::Move(dir) => {
                shift(&mut t.menu.items, i, *dir);
            }
            _ => {}
        }
    } else if let Some(o) = opt(id) {
        apply_opt(prefs, o, &cmd.arg);
    }
}

fn apply_opt(prefs: &mut ConsolePrefs, o: &Opt, arg: &Arg) {
    match (&o.kind, arg) {
        (Kind::Toggle { set, .. }, Arg::Value(Value::Toggle(on))) => set(prefs, *on),
        (Kind::Choice { names, set, .. }, Arg::Value(Value::Choice { index, .. })) if *index < names().len() => set(prefs, *index),
        (Kind::Number { min, max, set, .. }, Arg::Value(Value::Number { value, .. })) => set(prefs, (*value).clamp(*min, *max)),
        (Kind::Text { set, .. } | Kind::File { set, .. }, Arg::Text(s)) => {
            set(prefs, s);
        }
        _ => {}
    }
}

/// The file name an export suggests: `<id>.heronite-theme.json`.
pub fn export_name(theme: &ConsoleTheme) -> String {
    let id = if theme.id.is_empty() { slug(&theme.name) } else { theme.id.clone() };
    format!("{id}.{}", heronite_data::console_theme::store::EXTENSION)
}
