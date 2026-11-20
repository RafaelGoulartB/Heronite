//! Settings → Controller ([`Section::Controls`]): the controller this PC
//! sees (looked for again every few seconds while the page shows), how its
//! buttons are drawn and which one confirms (the controller's options,
//! `ConsolePrefs::controller`: console mode, the overlay and this page
//! follow them, `change_controller`), then one tab at a time (In a game,
//! Outside games, Overlay open, Console mode): the drawn controller with
//! what each button does (`pad`) and its shortcuts in a table (`table`),
//! where a click on a shortcut changes it. Every button is drawn as the
//! controller has it (`glyph`, `face`: ✕ ○ □ △ for PlayStation, B A Y X for
//! Nintendo). What can stop a shortcut sits under the table (Steam's Guide
//! setting). The overlay's options change through its own setters
//! (`overlay::update`); console mode's rows come from its saved theme, where
//! what A does on a cover changes (`change_console_theme`), the rest is
//! fixed.

mod pad;
mod table;

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{ConfirmButton, ConsoleButton, ControllerPrefs, Glyphs};
use heronite_i18n::{tr, tr as translate, trn};
use heronite_platform::{supported, Feature};

use super::{overlay, Section, SettingsPage};
use crate::console::kit::Face;
use crate::console::look;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, tabs, ButtonKind};
use crate::ui::icon;
use table::Edit;

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
    /// Bumped each time the page shows: the loop that looks for controllers
    /// of an earlier showing stops.
    watch: u64,
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
            watch: 0,
        }
    }
}

/// `HERONITE_DEBUG_CONTROLS=<tab>[:<popover>]` (for captures): the tab
/// (`ingame`, `outside`, `overlay`, `console`) and a popover open in it
/// (`shot`, `replay`, `moment`, `volume`, `anywhere`, `console-button`,
/// `key-overlay`, `key-shot`, `key-replay`, `console-a`, `glyphs`,
/// `confirm`).
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
        "console-a" => Some(Edit::ConsoleA),
        "glyphs" => Some(Edit::Glyphs),
        "confirm" => Some(Edit::Confirm(CONFIRM_CHIP)),
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

/// The page is hidden (Settings left for another page): it stops looking
/// for controllers (`shown` starts again when it comes back).
pub(super) fn hidden(d: &mut SettingsPage) {
    d.controls.watch += 1;
}

/// Looks for the controllers, as the Overlay section does, and again every
/// few seconds while the page shows: one plugged in (or its batteries put
/// back) shows up, and the automatic choices follow it.
pub(super) fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    overlay::shown(cx);
    d.controls.watch += 1;
    let watch = d.controls.watch;
    cx.spawn(async move |this, cx| loop {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let showing = this.update(cx, |d, _| d.section == Section::Controls && d.controls.watch == watch).unwrap_or(false);
        if !showing {
            break;
        }
        let list = cx.background_spawn(async { heronite_overlay::input::list_controllers() }).await;
        let alive = this
            .update(cx, |d, cx| {
                if overlay::controllers(d) != Some(list.as_slice()) {
                    look::controllers_found(&list);
                    overlay::set_controllers(d, list);
                    cx.notify();
                }
            })
            .is_ok();
        if !alive {
            break;
        }
    })
    .detach();
}

/// The button held to enter console mode, `Auto` resolved.
fn console_button(d: &SettingsPage) -> ConsoleButton {
    crate::console::themes::prefs(&d.ctx.app).console_button.resolved()
}

/// Console mode's theme as saved (console mode's own copy, `look::theme`,
/// is Classic while it is not running).
fn console_theme(d: &SettingsPage) -> heronite_data::console_theme::ConsoleTheme {
    crate::console::themes::read(&d.ctx.app, |p| p.theme.clone())
}

/// Changes console mode's theme (and the Console mode section's copy, when
/// it was shown).
fn change_console_theme(
    d: &mut SettingsPage,
    cx: &mut Context<SettingsPage>,
    change: impl FnOnce(&mut heronite_data::console_theme::ConsoleTheme),
) {
    let prefs = crate::console::themes::update(&d.ctx.app, |p| change(&mut p.theme));
    if let Some(s) = d.console_mode.as_mut() {
        s.theme.prefs = prefs;
        // Its Undo holds whole themes from before this change.
        s.theme.forget_steps();
    }
    cx.notify();
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

/// The face button a name stands for, by position: Xbox's letters (A
/// south, B east, X west, Y north).
fn face_of(name: &str) -> Option<Face> {
    Some(match name {
        "A" => Face::South,
        "B" => Face::East,
        "X" => Face::West,
        "Y" => Face::North,
        _ => return None,
    })
}

/// How `style` names a button the page writes as Xbox does (LB, Guide,
/// Back, Start, View, Menu...); others stay as they are.
fn button_name(style: Glyphs, name: &str) -> String {
    let pick = |xbox: &str, ps: &str, nintendo: &str, deck: &str| match style {
        Glyphs::PlayStation => ps.to_string(),
        Glyphs::Nintendo => nintendo.to_string(),
        Glyphs::Deck => deck.to_string(),
        _ => xbox.to_string(),
    };
    match name {
        "LB" => pick("LB", "L1", "L", "L1"),
        "RB" => pick("RB", "R1", "R", "R1"),
        "LT" => pick("LT", "L2", "ZL", "L2"),
        "RT" => pick("RT", "R2", "ZR", "R2"),
        "Guide" => pick("Guide", "PS", "Home", "Steam"),
        "Back" => pick("Back", "Create", "−", "View"),
        "Start" => pick("Start", "Options", "+", "Menu"),
        n if n == "View" || n == tr!("View") => pick(tr!("View"), "Create", "−", tr!("View")),
        n if n == "Menu" || n == tr!("Menu") => pick(tr!("Menu"), "Options", "+", tr!("Menu")),
        n => n.to_string(),
    }
}

/// What a face button shows in `style`: its colour (`None`: the neutral
/// fill) and a letter, or `None` for PlayStation's drawn symbol.
fn face_look(style: Glyphs, f: Face) -> (Option<Hsla>, Option<&'static str>) {
    let i = f as usize;
    match style {
        Glyphs::PlayStation => (None, None),
        // The letters follow the positions: Nintendo's south button is B.
        Glyphs::Nintendo => (None, Some(["B", "A", "Y", "X"][i])),
        Glyphs::Deck => (None, Some(["A", "B", "X", "Y"][i])),
        _ => (
            Some(
                match f {
                    Face::South => theme::console_btn_a(),
                    Face::East => theme::console_btn_b(),
                    Face::West => theme::console_btn_x(),
                    Face::North => theme::console_btn_y(),
                }
                .into(),
            ),
            Some(["A", "B", "X", "Y"][i]),
        ),
    }
}

/// A face button in `style`, `size` px round; `lit` false draws it dim
/// (a button the tab does not use).
pub(super) fn face(style: Glyphs, f: Face, size: f32, lit: bool) -> Div {
    let (colour, letter) = face_look(style, f);
    let base = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded_full()
        .text_size(px(size * 0.48))
        .font_weight(theme::W_BOLD);
    let base = match (colour, lit) {
        (Some(c), true) => base.bg(c).text_color(theme::console_on_focus()),
        (None, true) => base.bg(theme::ink_a(0.16)).text_color(theme::text_primary()),
        (_, false) => base.bg(theme::ink_a(0.05)).border_1().border_color(theme::ink_a(0.26)).text_color(theme::text_dim()),
    };
    match letter {
        Some(l) => base.child(l),
        None => {
            let shape: Hsla = if lit { crate::console::kit::shape_color(f).into() } else { theme::text_dim().into() };
            base.child(
                canvas(|_, _, _| (), move |bounds, _, window, _| crate::console::kit::paint_shape(bounds, f, shape, window))
                    .absolute()
                    .inset_0(),
            )
        }
    }
}

/// A controller button as the controller has it (`look::pad_glyphs`): the
/// face buttons round (A, B, X and Y name them by position), the others in
/// a pill under their name for that controller ("LB + RB" names both).
fn glyph(name: &str) -> Div {
    let style = look::pad_glyphs();
    if let Some(f) = face_of(name) {
        return face(style, f, 22., true);
    }
    let text = name.split(" + ").map(|n| button_name(style, n)).collect::<Vec<_>>().join(" + ");
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(22.))
        .min_w(px(22.))
        .px(px(7.))
        .rounded_full()
        .bg(theme::ink_a(0.12))
        .text_size(px(11.))
        .font_weight(theme::W_BOLD)
        .text_color(theme::text_primary())
        .whitespace_nowrap()
        .child(text)
}

/// A button's name as the controller has it (a face button's letter or
/// PlayStation symbol), for tooltips.
fn shown_name(name: &str) -> String {
    let style = look::pad_glyphs();
    if let Some(f) = face_of(name) {
        return match face_look(style, f) {
            (_, Some(l)) => l.to_string(),
            (_, None) => ["✕", "○", "□", "△"][f as usize].to_string(),
        };
    }
    name.split(" + ").map(|n| button_name(style, n)).collect::<Vec<_>>().join(" + ")
}

/// The names of the face buttons, south first, for the menus.
fn face_names(style: Glyphs) -> String {
    match style {
        Glyphs::PlayStation => "✕ ○ □ △".into(),
        Glyphs::Nintendo => "B A Y X".into(),
        Glyphs::Keyboard => "Enter · Esc".into(),
        _ => "A B X Y".into(),
    }
}

/// The face button that confirms, and its name ("A", "✕"...).
fn confirm_face() -> (Face, &'static str) {
    let east = look::confirm_east();
    let face = if east { Face::East } else { Face::South };
    let name = match (look::pad_glyphs(), east) {
        (Glyphs::PlayStation, false) => "✕",
        (Glyphs::PlayStation, true) => "○",
        (Glyphs::Nintendo, false) => "B",
        (Glyphs::Nintendo, true) => "A",
        (_, false) => "A",
        (_, true) => "B",
    };
    (face, name)
}

/// The id of the confirm button's chip at the top of the page.
const CONFIRM_CHIP: &str = "controls-confirm";

/// Changes the controller's options (console mode, the overlay and this
/// page follow at once), and the Console mode section's copy.
fn change_controller(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut ControllerPrefs)) {
    let prefs = crate::console::themes::update(&d.ctx.app, |p| change(&mut p.controller));
    if let Some(s) = d.console_mode.as_mut() {
        s.theme.prefs = prefs;
    }
    cx.notify();
}

/// The Controller page.
pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    follow_search(d);
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(device_bar(d, cx))
        .child(show_bar(d, cx))
        // Narrower, the callouts get too short for their text: the table says it all.
        .when(super::room(window) >= pad::MIN_W, |c| c.child(pad::map(d, cx)))
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

/// The controller found (from the overlay's detection), then how its
/// buttons are drawn and which one confirms: a chip each, whose popover
/// picks it.
fn device_bar(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let list = overlay::controllers(d);
    let (name, hint, found) = match list {
        None => (tr!("Looking for controllers…").to_string(), None, true),
        Some([]) => (
            tr!("No controller found").to_string(),
            Some(tr!("Connect one by cable or Bluetooth: it shows here once found.").to_string()),
            false,
        ),
        Some([one, rest @ ..]) => {
            let hint = if !rest.is_empty() {
                Some(trn!(rest.len(), "And {n} more: Overlay lists them.", "And {n} more: Overlay lists them."))
            } else if cfg!(windows) {
                Some(tr!("XInput controller. Back + Start replaces Guide.").to_string())
            } else {
                Some(tr!("{family} layout", family = family_name(look::glyphs_for_controller(&one.name))))
            };
            (tr!("{name} · connected", name = &one.name), hint, true)
        }
    };
    let dot = div().flex_none().size(px(9.)).rounded_full().bg(if found { theme::success() } else { theme::warning() });
    let head = div().flex().items_center().gap(px(14.)).pb(px(10.)).child(dot).child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(name))
            .children(hint.map(|h| div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_muted()).child(h))),
    );
    let c = look::controls();
    let open = d.controls.edit;
    let glyphs_chip = {
        let st = table::ChipState { open: open == Some(Edit::Glyphs), ..Default::default() };
        let content = div().text_size(px(12.5)).text_color(theme::text_primary()).child(glyphs_label(c.glyphs));
        let chip = table::chip("controls-glyphs", content, st, Edit::Glyphs, cx);
        let pop = st.open.then(|| glyphs_popover(c.glyphs, cx));
        table::with_popover(d, chip, pop, cx)
    };
    let confirm_chip = confirm_chip(d, CONFIRM_CHIP, true, cx);
    let row = |label: &'static str, desc: String, control: AnyElement| {
        div().border_t_1().border_color(theme::row_separator()).child(crate::ui::controls::row(label, Some(&desc), control))
    };
    crate::ui::controls::card_box()
        .py(px(12.))
        .child(head)
        .child(row(
            tr!("Buttons drawn as"),
            tr!("Automatic follows the controller in use. Console mode and the overlay show them the same way.").to_string(),
            glyphs_chip,
        ))
        .child(row(
            tr!("Confirm button"),
            tr!("The other of the two goes back. Console mode and the overlay follow it.").to_string(),
            confirm_chip,
        ))
}

/// A controller family's name.
fn family_name(g: Glyphs) -> &'static str {
    match g {
        Glyphs::PlayStation => tr!("PlayStation"),
        Glyphs::Nintendo => tr!("Nintendo"),
        Glyphs::Deck => tr!("Steam Deck"),
        Glyphs::Keyboard => tr!("Keyboard"),
        Glyphs::Xbox | Glyphs::Auto => tr!("Xbox"),
    }
}

/// "Buttons drawn as": `Auto` with what it resolves to.
fn glyphs_label(g: Glyphs) -> String {
    match g {
        Glyphs::Auto => tr!("Automatic ({glyphs})", glyphs = family_name(look::glyphs())),
        g => family_name(g).to_string(),
    }
}

/// "Confirm button": the button and, when automatic, that it is.
fn confirm_label(c: ConfirmButton) -> &'static str {
    match (c, look::confirm_east()) {
        (ConfirmButton::Auto, true) => tr!("Automatic (east)"),
        (ConfirmButton::Auto, false) => tr!("Automatic (south)"),
        (_, true) => tr!("East"),
        (_, false) => tr!("South"),
    }
}

/// The menu of "Buttons drawn as": each style with its face buttons.
fn glyphs_popover(current: Glyphs, cx: &mut Context<SettingsPage>) -> Div {
    let mut list = div().mt(px(10.)).flex().flex_col().gap(px(2.));
    for (i, g) in Glyphs::ALL.iter().copied().enumerate() {
        let on = g == current;
        let label = match g {
            Glyphs::Auto => glyphs_label(Glyphs::Auto),
            g => family_name(g).to_string(),
        };
        let shown = if g == Glyphs::Auto { look::glyphs() } else { g };
        list = list.child(
            crate::ui::dropdown::menu_item(SharedString::from(format!("controls-glyphs-{i}")), on)
                .child(div().flex_1().child(label))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(face_names(shown)))
                .child(div().w(px(14.)).children(on.then(|| icon(Icon::Check, px(14.), theme::text_primary()))))
                .on_click(cx.listener(move |d, _, _, cx| {
                    change_controller(d, cx, |c| c.glyphs = g);
                    table::close_edit(d, cx);
                })),
        );
    }
    table::popover(tr!("Buttons drawn as"), None).child(list)
}

/// A chip of the confirm button (its glyph; `words` adds what it is) whose
/// popover picks it. `id` tells the chips apart: the page's and the rows'
/// of Overlay open and Console mode.
fn confirm_chip(d: &SettingsPage, id: &'static str, words: bool, cx: &mut Context<SettingsPage>) -> AnyElement {
    let edit = Edit::Confirm(id);
    let c = look::controls();
    let st = table::ChipState { open: d.controls.edit == Some(edit), ..Default::default() };
    let (f, _) = confirm_face();
    let content = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(face(look::pad_glyphs(), f, 20., true))
        .when(words, |row| row.child(div().text_size(px(12.5)).text_color(theme::text_primary()).child(confirm_label(c.confirm))));
    let chip = table::chip(id, content, st, edit, cx);
    let pop = st.open.then(|| confirm_menu(cx));
    table::with_popover(d, chip, pop, cx)
}

/// The menu of the confirm button: automatic, south or east, each with its
/// glyph.
fn confirm_menu(cx: &mut Context<SettingsPage>) -> Div {
    let current = look::controls().confirm;
    let style = look::pad_glyphs();
    let mut list = div().mt(px(10.)).flex().flex_col().gap(px(2.));
    for (i, c) in ConfirmButton::ALL.iter().copied().enumerate() {
        let on = c == current;
        let f = match c {
            ConfirmButton::Auto => confirm_face().0,
            ConfirmButton::South => Face::South,
            ConfirmButton::East => Face::East,
        };
        let label = match c {
            ConfirmButton::Auto => tr!("Automatic: east on Nintendo controllers").to_string(),
            c => translate(crate::console::theme_rows::confirm_name(c)).to_string(),
        };
        list = list.child(
            crate::ui::dropdown::menu_item(SharedString::from(format!("controls-confirm-{i}")), on)
                .child(face(style, f, 18., true))
                .child(div().flex_1().child(label))
                .child(div().w(px(14.)).children(on.then(|| icon(Icon::Check, px(14.), theme::text_primary()))))
                .on_click(cx.listener(move |d, _, _, cx| {
                    change_controller(d, cx, |p| p.confirm = c);
                    table::close_edit(d, cx);
                })),
        );
    }
    table::popover(tr!("Confirm button"), Some(tr!("The other one goes back.").to_string())).child(list)
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
        Show::InGame => tr!(
            "Hold {guide} and press a button for a shortcut. Click a shortcut to change it.",
            guide = button_name(look::pad_glyphs(), "Guide")
        )
        .to_string(),
        Show::Outside => tr!("On this PC, while no game runs. Click a shortcut to change it.").to_string(),
        Show::Overlay => tr!("While the overlay is on screen. Only the confirm button changes; the others are fixed.").to_string(),
        Show::Console => {
            let name = console_theme(d).name;
            if name.trim().is_empty() {
                tr!("Set by console mode's theme. Click a shortcut to change it; the others are fixed.").to_string()
            } else {
                tr!("Set by console mode's theme ({name}). Click a shortcut to change it; the others are fixed.", name = name.trim())
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
