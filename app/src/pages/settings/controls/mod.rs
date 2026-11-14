//! Settings → Controller ([`Section::Controls`]): the controller this PC
//! sees and how console mode draws its buttons, then one tab at a time (In a
//! game, Outside games, Overlay open, Console mode): the drawn controller
//! with what each button does (`pad`) and its shortcuts in a table (`table`),
//! where a click on a shortcut changes it. What can stop a shortcut sits
//! under the table (Steam's Guide setting). The overlay's options change
//! through its own setters (`overlay::update`); console mode's rows are
//! read-only: they belong to its theme (Console mode).

mod pad;
mod table;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::ConsoleButton;
use heronite_i18n::{tr, trn};
use heronite_platform::{supported, Feature};

use super::{overlay, Section, SettingsPage};
use crate::theme;
use crate::ui::controls::{button_sm, tabs, ButtonKind};

/// What the Controller page shows and which of its popovers is open.
pub struct State {
    /// The tab: where the shortcuts work.
    show: Show,
    /// The button picked on the drawing or in the table: its callout and its
    /// row light up.
    selected: Option<pad::Part>,
    /// The popover open in the table.
    edit: Option<table::Edit>,
    /// The box that records a keyboard shortcut.
    key_focus: FocusHandle,
    /// The open popover (Esc closes it).
    pop_focus: FocusHandle,
    /// It has the focus (read each frame).
    recording: bool,
    /// Why the keys just pressed were not taken.
    key_error: Option<String>,
    /// The Settings search's jump this page last followed to its tab.
    found: u64,
}

impl State {
    pub fn new(cx: &mut Context<SettingsPage>) -> Self {
        let (show, edit) = debug_start();
        Self {
            show: show.unwrap_or(if supported(Feature::Overlay) { Show::InGame } else { Show::Outside }),
            selected: None,
            edit,
            key_focus: cx.focus_handle(),
            pop_focus: cx.focus_handle(),
            recording: false,
            key_error: None,
            found: 0,
        }
    }
}

/// `HERONITE_DEBUG_CONTROLS=<tab>[:<popover>]` (for captures): the tab
/// (`ingame`, `outside`, `overlay`, `console`) and a popover open in it
/// (`shot`, `replay`, `moment`, `volume`, `anywhere`, `console-button`,
/// `key-overlay`, `key-shot`, `key-replay`).
fn debug_start() -> (Option<Show>, Option<table::Edit>) {
    use table::{Edit, Fixed, Key};
    let Some(v) = crate::env::value("HERONITE_DEBUG_CONTROLS") else { return (None, None) };
    let (tab, popover) = v.split_once(':').unwrap_or((v.as_str(), ""));
    let show = match tab {
        "ingame" => Some(Show::InGame),
        "outside" => Some(Show::Outside),
        "overlay" => Some(Show::Overlay),
        "console" => Some(Show::Console),
        _ => None,
    };
    let edit = match popover {
        "shot" => Some(Edit::ShotButton),
        "replay" => Some(Edit::Fixed(Fixed::Replay)),
        "moment" => Some(Edit::Fixed(Fixed::Moment)),
        "volume" => Some(Edit::Fixed(Fixed::Volume)),
        "anywhere" => Some(Edit::Fixed(Fixed::Anywhere)),
        "console-button" => Some(Edit::ConsoleButton),
        "key-overlay" => Some(Edit::Key(Key::Overlay)),
        "key-shot" => Some(Edit::Key(Key::Shot)),
        "key-replay" => Some(Edit::Key(Key::Replay)),
        _ => None,
    };
    (show, edit)
}

/// Where a shortcut works: the page's tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Show {
    /// During a game, through the overlay.
    InGame,
    /// Outside games, on this PC.
    Outside,
    /// While the overlay is open.
    Overlay,
    /// In console mode.
    Console,
}

/// The button(s) that stand for Guide on this system, as key caps (names:
/// not translated).
const GUIDE: &[&str] = if cfg!(windows) { &["Back", "Start"] } else { &["Guide"] };

/// Looks for the controllers, as the Overlay section does.
pub(super) fn shown(cx: &mut Context<SettingsPage>) {
    overlay::shown(cx);
}

/// The button held to enter console mode, `Auto` resolved.
fn console_button(d: &SettingsPage) -> ConsoleButton {
    crate::console::themes::prefs(&d.ctx.app).console_button.resolved()
}

/// The overlay's button (Guide, Back + Start on Windows) is also the one
/// held to enter console mode.
fn shares_guide(d: &SettingsPage) -> bool {
    let overlay_button = if cfg!(windows) { ConsoleButton::BackStart } else { ConsoleButton::Guide };
    supported(Feature::Overlay) && console_button(d) == overlay_button
}

/// Outside games, a tap of that button opens the overlay and a hold enters
/// console mode (the overlay opens without a game too): two gestures, one
/// button, no clash.
pub(super) fn tap_and_hold(d: &SettingsPage) -> bool {
    let p = &d.overlay.prefs;
    shares_guide(d) && p.enabled && p.without_game
}

/// A controller button as printed on it: A, B, X and Y round in their
/// colours, the others in a pill.
fn glyph(name: &str) -> Div {
    let face = match name {
        "A" => Some(theme::console_btn_a()),
        "B" => Some(theme::console_btn_b()),
        "X" => Some(theme::console_btn_x()),
        "Y" => Some(theme::console_btn_y()),
        _ => None,
    };
    let g = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.))
        .rounded_full()
        .text_size(px(11.))
        .font_weight(theme::W_BOLD)
        .whitespace_nowrap();
    match face {
        Some(c) => g.w(px(22.)).bg(c).text_color(theme::console_on_focus()),
        None => g.min_w(px(22.)).px(px(7.)).bg(theme::ink_a(0.12)).text_color(theme::text_primary()),
    }
    .child(name.to_string())
}

/// The Controller page.
pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    follow_search(d);
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(device_bar(d))
        .child(show_bar(d, cx))
        .child(pad::map(d, cx))
        .child(table::render(d, window, cx))
        .into_any_element()
}

/// A setting found by the Settings search shows on its tab.
fn follow_search(d: &mut SettingsPage) {
    let Some((text, generation)) = crate::ui::find::target() else { return };
    if generation == d.controls.found {
        return;
    }
    d.controls.found = generation;
    if let Some(show) = table::tab_of(&text) {
        d.controls.show = show;
        d.controls.edit = None;
    }
}

/// The controller found (from the overlay's detection) and how console
/// mode draws its buttons (its theme's, read here).
fn device_bar(d: &SettingsPage) -> Div {
    let list = overlay::controllers(d);
    let (name, hint, found) = match list {
        None => (tr!("Looking for controllers…").to_string(), None, true),
        Some([]) => (
            tr!("No controller found").to_string(),
            Some(tr!("Connect one: it is looked for again when a game starts.").to_string()),
            false,
        ),
        Some([one, rest @ ..]) => {
            let hint = if !rest.is_empty() {
                Some(trn!(rest.len(), "And {n} more: Overlay lists them.", "And {n} more: Overlay lists them."))
            } else if cfg!(windows) {
                Some(tr!("XInput controller. Back + Start replaces Guide.").to_string())
            } else {
                None
            };
            (tr!("{name} · connected", name = &one.name), hint, true)
        }
    };
    let dot = div().flex_none().size(px(9.)).rounded_full().bg(if found { theme::success() } else { theme::warning() });
    let glyphs = div()
        .flex()
        .flex_col()
        .items_end()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(6.))
                .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(tr!("Buttons drawn as")))
                .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(glyphs_name())),
        )
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(tr!("Set by console mode's theme")));
    crate::ui::controls::card_box().py(px(12.)).child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.))
            .child(dot)
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(name))
                    .children(hint.map(|h| div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_muted()).child(h))),
            )
            .child(glyphs),
    )
}

/// Console mode's "Buttons drawn as", `Auto` with what it resolves to.
fn glyphs_name() -> String {
    use heronite_data::console_theme::Glyphs;
    let name = |g: Glyphs| match g {
        Glyphs::Auto => tr!("Automatic (the controller's)"),
        Glyphs::Xbox => tr!("Xbox"),
        Glyphs::PlayStation => tr!("PlayStation"),
        Glyphs::Nintendo => tr!("Nintendo"),
        Glyphs::Deck => tr!("Steam Deck"),
        Glyphs::Keyboard => tr!("Keyboard"),
    };
    match crate::console::look::theme().controls.glyphs {
        Glyphs::Auto => tr!("Auto ({glyphs})", glyphs = name(crate::console::look::glyphs())),
        g => name(g).to_string(),
    }
}

/// The tabs, and a line on how the tab's shortcuts work.
fn show_bar(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let mut items = Vec::new();
    if supported(Feature::Overlay) {
        items.push((Show::InGame, tr!("In a game")));
    }
    items.push((Show::Outside, tr!("Outside games")));
    if supported(Feature::Overlay) {
        items.push((Show::Overlay, tr!("Overlay open")));
    }
    items.push((Show::Console, tr!("Console mode")));
    let page = cx.entity().downgrade();
    let seg = tabs("controls-show", &items, d.controls.show, move |v, _, cx| {
        page.update(cx, |d, cx| {
            d.controls.show = v;
            d.controls.selected = None;
            d.controls.edit = None;
            cx.notify();
        })
        .ok();
    });
    let hint = match d.controls.show {
        Show::InGame if cfg!(windows) => {
            tr!("Hold Back + Start and press a button for a shortcut. Click a shortcut to change it.").to_string()
        }
        Show::InGame => tr!("Hold Guide and press a button for a shortcut. Click a shortcut to change it.").to_string(),
        Show::Outside => tr!("On this PC, while no game runs. Click a shortcut to change it.").to_string(),
        Show::Overlay => tr!("While the overlay is on screen. These buttons are fixed.").to_string(),
        Show::Console => {
            let name = crate::console::look::theme().name.clone();
            if name.trim().is_empty() {
                tr!("Set by console mode's theme: read-only here.").to_string()
            } else {
                tr!("Set by console mode's theme ({name}): read-only here.", name = name.trim())
            }
        }
    };
    let change = (d.controls.show == Show::Console).then(|| {
        button_sm("controls-console-look", ButtonKind::Glass, tr!("Change in Console mode"))
            .on_click(cx.listener(|d, _, window, cx| d.select(Section::ConsoleMode, window, cx)))
    });
    div().flex().flex_col().gap(px(10.)).pt(px(4.)).child(div().child(seg)).child(
        // As tall on every tab (Console mode's has a button): the drawing
        // does not jump.
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .min_h(px(30.))
            .child(div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_muted()).child(hint))
            .children(change),
    )
}

/// Picks `part` on the drawing and in the table, or lets it go.
fn toggle_selected(d: &mut SettingsPage, part: pad::Part, cx: &mut Context<SettingsPage>) {
    d.controls.selected = if d.controls.selected == Some(part) { None } else { Some(part) };
    cx.notify();
}
