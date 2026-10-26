//! What the theme in use changed from the theme it started from, option by
//! option (the ↺ of a row and the counts of the editor's groups), putting
//! one option back, and the ready-made colour sets.

use heronite_data::appearance::{contrast, readable_on, Rgb};
use heronite_data::console_theme::ACCENT_NAMES;

use super::*;

/// The prefs with `base` as their theme, for reading an option's value in it.
fn with_theme(prefs: &ConsolePrefs, base: &ConsoleTheme) -> ConsolePrefs {
    ConsolePrefs { theme: base.clone(), ..prefs.clone() }
}

/// Whether the option `id` (a plain option or the focus colour) differs from
/// `base`. Startup options are not part of a theme and never differ.
pub fn differs(id: &str, prefs: &ConsolePrefs, base: &ConsoleTheme) -> bool {
    if id == "interface.accent" {
        return prefs.theme.interface.accent != base.interface.accent;
    }
    let Some(o) = opt(id).filter(|o| o.card != Card::Startup) else { return false };
    let b = with_theme(prefs, base);
    match &o.kind {
        Kind::Toggle { get, .. } => get(prefs) != get(&b),
        Kind::Choice { get, .. } => get(prefs) != get(&b),
        Kind::Number { get, .. } => get(prefs) != get(&b),
        Kind::Text { get, .. } | Kind::File { get, .. } => get(prefs) != get(&b),
    }
}

/// The command that puts the option `id` back as `base` has it.
pub fn reset_command(id: &str, prefs: &ConsolePrefs, base: &ConsoleTheme) -> Option<Command> {
    if id == "interface.accent" {
        return Some(Command::new("interface.accent_hex", Arg::Text(base.interface.accent.clone())));
    }
    let o = opt(id)?;
    let b = with_theme(prefs, base);
    Some(match &o.kind {
        Kind::Toggle { get, .. } => Command::changed(id, Value::Toggle(get(&b))),
        Kind::Choice { get, .. } => Command::changed(id, Value::Choice { options: Vec::new(), index: get(&b) }),
        Kind::Number { min, max, step, suffix, get, .. } => {
            Command::changed(id, Value::Number { value: get(&b), min: *min, max: *max, step: *step, suffix })
        }
        Kind::Text { get, .. } | Kind::File { get, .. } => Command::new(id, Arg::Text(get(&b))),
    })
}

/// How many options of `cards` differ from `base` (a changed list of tabs,
/// shelves or menu items counts as one).
pub fn changed_in(cards: &[Card], prefs: &ConsolePrefs, base: &ConsoleTheme) -> usize {
    let t = &prefs.theme;
    cards
        .iter()
        .map(|c| {
            let lists = match c {
                Card::Tabs => usize::from(t.tabs != base.tabs),
                Card::Shelves => usize::from(t.shelves != base.shelves),
                Card::Menu => usize::from(t.menu.items != base.menu.items),
                Card::Interface => usize::from(differs("interface.accent", prefs, base)),
                _ => 0,
            };
            lists + OPTS.iter().filter(|o| o.card == *c && differs(o.id, prefs, base)).count()
        })
        .sum()
}

/// A ready-made set of colours (`#rrggbb`; empty keeps the built-in one).
pub struct ColourSet {
    pub id: &'static str,
    /// English key.
    pub name: &'static str,
    pub background: &'static str,
    pub panel: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    /// A named focus colour or `#rrggbb`.
    pub accent: &'static str,
    pub focus_text: &'static str,
    pub on: &'static str,
}

/// The colour sets the editor offers, the built-in colours first.
pub const COLOUR_SETS: [ColourSet; 9] = [
    ColourSet { id: "built-in", name: n_!("Built-in"), background: "", panel: "", text: "", muted: "", accent: "white", focus_text: "", on: "" },
    ColourSet {
        id: "midnight",
        name: n_!("Midnight"),
        background: "#0b1020",
        panel: "#151c30",
        text: "#e8ecf6",
        muted: "#9aa6c4",
        accent: "ice",
        focus_text: "",
        on: "#7aa2ff",
    },
    ColourSet {
        id: "nord",
        name: n_!("Nord"),
        background: "#2e3440",
        panel: "#3b4252",
        text: "#eceff4",
        muted: "#c0c8d6",
        accent: "#88c0d0",
        focus_text: "#2e3440",
        on: "#a3be8c",
    },
    ColourSet {
        id: "dusk",
        name: n_!("Dusk"),
        background: "#1e1f29",
        panel: "#282a36",
        text: "#f8f8f2",
        muted: "#b4b8d4",
        accent: "violet",
        focus_text: "",
        on: "#50fa7b",
    },
    ColourSet {
        id: "forest",
        name: n_!("Forest"),
        background: "#0f1a14",
        panel: "#17271e",
        text: "#e6f2ea",
        muted: "#a3bcac",
        accent: "green",
        focus_text: "",
        on: "#5ee3a1",
    },
    ColourSet {
        id: "ember",
        name: n_!("Ember"),
        background: "#1c1016",
        panel: "#2a1820",
        text: "#fbeef0",
        muted: "#cfadb5",
        accent: "amber",
        focus_text: "",
        on: "#f49ac1",
    },
    ColourSet {
        id: "arcade",
        name: n_!("Arcade"),
        background: "#0d0221",
        panel: "#1a0b3a",
        text: "#f5f0ff",
        muted: "#b9a8e0",
        accent: "pink",
        focus_text: "",
        on: "#2de2e6",
    },
    ColourSet {
        id: "paper",
        name: n_!("Paper"),
        background: "#f2f0eb",
        panel: "#ffffff",
        text: "#1d2128",
        muted: "#555c69",
        accent: "#2d5bd7",
        focus_text: "#ffffff",
        on: "#2d5bd7",
    },
    ColourSet {
        id: "frost",
        name: n_!("Frost"),
        background: "#e6eef5",
        panel: "#ffffff",
        text: "#14202c",
        muted: "#4b5a69",
        accent: "#0f6fb5",
        focus_text: "#ffffff",
        on: "#0f6fb5",
    },
];

impl ColourSet {
    pub fn apply(&self, t: &mut ConsoleTheme) {
        let c = &mut t.colors;
        c.background = self.background.into();
        c.panel = self.panel.into();
        c.text = self.text.into();
        c.muted = self.muted.into();
        c.focus_text = self.focus_text.into();
        c.on = self.on.into();
        t.interface.accent = self.accent.into();
    }

    /// The theme has exactly these colours.
    pub fn is_on(&self, t: &ConsoleTheme) -> bool {
        let mut probe = t.clone();
        self.apply(&mut probe);
        probe == *t
    }
}

/// `colors.<role>` of `t`, parsed.
fn colour_of(t: &ConsoleTheme, role: &str) -> Option<Rgb> {
    let c = &t.colors;
    parse_hex(match role {
        "background" => &c.background,
        "panel" => &c.panel,
        "text" => &c.text,
        "muted" => &c.muted,
        _ => return None,
    })
}

/// A text colour of the theme that does not read on a colour it sits on:
/// `(role of the text, role it sits on, contrast ratio)`. Only colours the
/// theme sets are compared (the built-in ones are readable).
pub fn hard_to_read(t: &ConsoleTheme) -> Vec<(&'static str, &'static str, f32)> {
    let mut out = Vec::new();
    for text in ["text", "muted"] {
        let Some(fg) = colour_of(t, text) else { continue };
        for under in ["background", "panel"] {
            let Some(bg) = colour_of(t, under) else { continue };
            let ratio = contrast(fg, bg);
            if ratio < 4.5 {
                out.push((text, under, ratio));
            }
        }
    }
    out
}

/// The text colours moved (same hue) until they read on the background and
/// the panels.
pub fn make_readable(t: &mut ConsoleTheme) {
    let light = t.colors.is_light();
    let bgs: Vec<Rgb> = ["background", "panel"].iter().filter_map(|r| colour_of(t, r)).collect();
    for role in ["text", "muted"] {
        let Some(fg) = colour_of(t, role) else { continue };
        let fixed = heronite_data::appearance::hex(readable_on(fg, &bgs, light));
        match role {
            "text" => t.colors.text = fixed,
            _ => t.colors.muted = fixed,
        }
    }
}

/// Whether `s` is a focus colour a theme can have.
pub fn is_accent(s: &str) -> bool {
    ACCENT_NAMES.contains(&s) || parse_hex(s).is_some()
}
