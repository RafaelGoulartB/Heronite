//! Settings → Controller ([`Section::Controls`]): the controller this PC
//! sees, the checks (Steam Input, Guide's two jobs outside games), the drawn
//! controller with what each button does (`pad`) and every binding in a
//! table by context (`table`). The overlay's chords are set there with the
//! overlay's own setters (`overlay::update`), the button that enters console
//! mode too; console mode's rows are read-only: they belong to its theme
//! (Console mode).

mod pad;
mod table;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::ConsoleButton;
use heronite_data::overlay_prefs::ChordButton;
use heronite_i18n::{tr, trn};
use heronite_platform::{supported, Feature};

use super::{overlay, Section, SettingsPage};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, tabs, ButtonKind};
use crate::ui::icon;

/// What the Controller page shows and which of its menus is open.
#[derive(Default)]
pub struct State {
    /// The menu of the button that enters console mode.
    console_button_open: bool,
    /// The context the drawing and the table show.
    show: Show,
    /// The button picked on the drawing or in the table: its callout and its
    /// rows light up.
    selected: Option<pad::Part>,
    /// The sheet that changes the screenshot's button is open, with the
    /// button picked in it so far.
    shot_pick: Option<ChordButton>,
}

/// Where a binding works: the "Show" choice above the drawing.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Show {
    /// Every context at once.
    #[default]
    All,
    /// During a game, through the overlay.
    InGame,
    /// Outside games, on this PC.
    Outside,
    /// While the overlay is open.
    Overlay,
    /// In console mode.
    Console,
}

impl Show {
    /// `context` is shown.
    fn has(self, context: Show) -> bool {
        self == Show::All || self == context
    }
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
/// console mode: the overlay opens without a game too.
pub(super) fn clash(d: &SettingsPage) -> bool {
    let p = &d.overlay.prefs;
    shares_guide(d) && p.enabled && p.without_game
}

/// The Controller page.
pub(super) fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(device_bar(d, cx))
        .child(checks(d, cx))
        .child(show_bar(d, cx))
        .child(pad::map(d, window, cx))
        .child(table::render(d, window, cx))
        .into_any_element()
}

/// The controller found (from the overlay's detection) and how console
/// mode draws its buttons (its theme's, read here).
fn device_bar(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
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
                trn!(rest.len(), "And {n} more: Overlay lists them.", "And {n} more: Overlay lists them.")
            } else if cfg!(windows) {
                tr!("XInput controller. Back + Start replaces Guide.").to_string()
            } else {
                tr!("Each button and what it does is below.").to_string()
            };
            (tr!("{name} · connected", name = &one.name), Some(hint), true)
        }
    };
    let dot = div()
        .flex_none()
        .size(px(9.))
        .rounded_full()
        .bg(if found { theme::success() } else { theme::warning() })
        .border_2()
        .border_color(theme::ink_a(0.08));
    let glyphs = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(tr!("Buttons drawn as")))
        .child(
            div()
                .px(px(12.))
                .py(px(6.))
                .rounded(px(10.))
                .bg(theme::ink_a(0.06))
                .text_size(px(13.))
                .text_color(theme::text_primary())
                .child(glyphs_name()),
        )
        .child(
            crate::ui::controls::scope_chip(
                "controls-glyphs-look",
                Icon::Tv,
                tr!("Part of the console theme"),
                tr!("Console mode's theme sets it: Console mode."),
            )
            .cursor_pointer()
            .on_click(cx.listener(|d, _, window, cx| d.select(Section::ConsoleMode, window, cx))),
        );
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
                    .min_w(px(200.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(name))
                    .children(hint.map(|h| div().text_size(px(12.5)).text_color(theme::text_muted()).child(h))),
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

/// A line of the checks: an (i) or a warning sign, the text, its detail and
/// an action.
fn check_line(problem: bool, text: &'static str, detail: &'static str, action: Option<AnyElement>) -> AnyElement {
    let mark = if problem {
        icon(Icon::Warning, px(15.), theme::warning()).into_any_element()
    } else {
        icon(Icon::InfoOutlined, px(15.), theme::text_dim()).into_any_element()
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(10.))
        .child(div().flex_none().child(mark))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(text))
                .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(detail)),
        )
        .children(action)
        .into_any_element()
}

/// What can stop a button from doing what the table says: Steam Input (a
/// note: Steam's setting is not read), Game Bar on Windows, and Guide's two
/// jobs outside games when the overlay opens without a game.
fn checks(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let mut lines = Vec::new();
    if supported(Feature::Overlay) {
        lines.push(if cfg!(windows) {
            check_line(
                false,
                tr!("Game Bar may take the Guide button, so Back + Start stands in for it."),
                tr!("The game also gets Back + Start and the buttons pressed with it."),
                None,
            )
        } else {
            check_line(
                false,
                tr!("With Steam's \"Guide Button Focuses Steam\" on, Guide opens Steam instead of the overlay."),
                tr!("Turn it off in Steam → Settings → Controller while Steam is open."),
                None,
            )
        });
    }
    let clash = clash(d);
    if clash {
        let fix = button_sm("controls-clash-fix", ButtonKind::Glass, tr!("Use Hold LB + RB for console mode")).on_click(cx.listener(
            |d, _, _, cx| {
                crate::console::startup::set_console_button(&d.ctx.app, ConsoleButton::Bumpers);
                crate::console::summon::options_changed(cx);
                cx.notify();
            },
        ));
        let detail = tr!("Because \"Open the overlay without a game too\" is on.");
        let text = if cfg!(windows) {
            tr!("Back + Start does two things outside games: a tap opens the overlay and a hold enters console mode.")
        } else {
            tr!("Guide does two things outside games: a tap opens the overlay and a hold enters console mode.")
        };
        lines.push(check_line(true, text, detail, Some(fix.into_any_element())));
    } else if shares_guide(d) {
        let text = if cfg!(windows) {
            tr!("Back + Start: a hold enters console mode, and the overlay opens only in games, so they do not clash.")
        } else {
            tr!("Guide: a hold enters console mode, and the overlay opens only in games, so they do not clash.")
        };
        lines.push(check_line(false, text, tr!("Turning on \"Open the overlay without a game too\" changes that."), None));
    }
    let count = if clash { trn!(1, "{n} problem", "{n} problems") } else { tr!("No problems").to_string() };
    let head = div()
        .flex()
        .items_baseline()
        .gap(px(8.))
        .pt(px(10.))
        .pb(px(2.))
        .child(div().font_family(theme::font_heading()).text_size(px(13.)).text_color(theme::text_primary()).child(tr!("Checks")))
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).child(count));
    let mut c = crate::ui::controls::card_box().child(head);
    for l in lines {
        c = c.child(div().h(px(1.)).w_full().bg(theme::row_separator())).child(l);
    }
    c
}

/// "Show" and its contexts.
fn show_bar(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let mut items = vec![(Show::All, tr!("All"))];
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
            cx.notify();
        })
        .ok();
    });
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(10.))
        .pt(px(4.))
        .child(div().text_size(px(12.5)).text_color(theme::text_dim()).child(tr!("Show")))
        .child(seg)
}

/// Picks `part` on the drawing and in the table, or lets it go.
fn toggle_selected(d: &mut SettingsPage, part: pad::Part, cx: &mut Context<SettingsPage>) {
    d.controls.selected = if d.controls.selected == Some(part) { None } else { Some(part) };
    cx.notify();
}
