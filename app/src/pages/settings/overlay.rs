//! Settings → Overlay: the in-game overlay (beta, off by default), its
//! keyboard shortcut, the controller shortcuts and side, and whether this
//! desktop and the controllers can use it. The options live in
//! `overlay.json` (`heronite_data::overlay_prefs`) and reach the running
//! service at once. Settings → Screenshots shows the Guide screenshot rows
//! too ([`pad_shot_card`]), and Settings → Replay the Guide + X one
//! ([`replay_shortcut_card`]).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::overlay_prefs::{ChordButton, OverlayPrefs, Side, DEFAULT_KEYBOARD_ACCELERATOR, VOLUME_STEPS};
use heronite_i18n::{tr, trn};
use heronite_overlay::input::Controller;
use heronite_platform::{supported, Feature};

use super::SettingsPage;
use crate::overlay::{labels, HotkeyState};
use crate::pages::{status_pill, PageCtx};
use crate::theme;
use crate::ui::controls::{caption, card, row, tabs, toggle_row, with_sub_rows};
use crate::ui::fields::{accelerator_from_key, hotkey_box, labeled, meta_text, select_field};

pub struct State {
    pub(super) prefs: OverlayPrefs,
    /// `None` while they are looked for.
    controllers: Option<Vec<Controller>>,
    /// The keyboard shortcut field.
    hotkey: FocusHandle,
    /// The menu of the screenshot button is open.
    button_open: bool,
}

impl State {
    pub fn new(ctx: &PageCtx, cx: &mut Context<SettingsPage>) -> Self {
        // The shortcut's state (registered, failed) changes in the service.
        if let Some(service) = crate::overlay::get(cx) {
            cx.observe(&service, |_, _, cx| cx.notify()).detach();
        }
        // A tiny file in the app's config folder.
        Self { prefs: OverlayPrefs::load(&ctx.app), controllers: None, hotkey: cx.focus_handle(), button_open: false }
    }
}

/// Lists the controllers when the section shows.
pub fn shown(cx: &mut Context<SettingsPage>) {
    cx.spawn(async move |this, cx| {
        let list = cx.background_spawn(async { heronite_overlay::input::list_controllers() }).await;
        this.update(cx, |d, cx| {
            d.overlay.controllers = Some(list);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

impl SettingsPage {
    /// The in-game overlay is on (Settings → Screenshots dims its rows when not).
    pub(super) fn overlay_enabled(&self) -> bool {
        self.overlay.prefs.enabled
    }
}

/// Changes the options, saves them (off the UI thread) and hands them to the
/// overlay.
fn update(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut OverlayPrefs)) {
    change(&mut d.overlay.prefs);
    let prefs = d.overlay.prefs.clone();
    let app = d.ctx.app.clone();
    let saved = prefs.clone();
    cx.background_spawn(async move {
        if let Err(e) = saved.save(&app) {
            log::warn!("saving the overlay options: {e:#}");
        }
    })
    .detach();
    if let Some(service) = crate::overlay::get(cx) {
        service.update(cx, |s, cx| s.set_prefs(prefs, cx));
    }
    cx.notify();
}

fn controllers_text(list: Option<&Vec<Controller>>) -> String {
    match list {
        None => tr!("Looking for controllers…").into(),
        Some(l) if l.is_empty() => tr!("None found. Connect a controller; it is looked for again when a game starts.").into(),
        Some(l) => l.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "),
    }
}

fn step_label(step: u8) -> &'static str {
    match step {
        2 => "2%",
        10 => "10%",
        _ => "5%",
    }
}

/// The button as written after "Guide +".
fn button_short(b: ChordButton) -> String {
    match b {
        ChordButton::Left => "←".into(),
        ChordButton::Right => "→".into(),
        ChordButton::Start => tr!("Menu").into(),
        ChordButton::Select => tr!("View").into(),
        b => b.as_str().into(),
    }
}

/// The button in the menu.
fn button_label(b: ChordButton) -> String {
    match b {
        ChordButton::Left => tr!("D-pad left").into(),
        ChordButton::Right => tr!("D-pad right").into(),
        ChordButton::Start => tr!("Menu (Start)").into(),
        ChordButton::Select => tr!("View (Select / Back)").into(),
        b => b.as_str().into(),
    }
}

fn shot_title(p: &OverlayPrefs) -> String {
    labels::screenshot(&button_short(screenshot_button(p)))
}

/// On Windows Back + Start stand for Guide, so neither can go with it.
fn stands_for_guide(b: ChordButton) -> bool {
    cfg!(windows) && matches!(b, ChordButton::Start | ChordButton::Select)
}

/// The buttons the screenshot can use here.
fn screenshot_buttons() -> Vec<ChordButton> {
    ChordButton::ALL.into_iter().filter(|b| !stands_for_guide(*b)).collect()
}

fn screenshot_button(p: &OverlayPrefs) -> ChordButton {
    if screenshot_buttons().contains(&p.screenshot_button) {
        p.screenshot_button
    } else {
        ChordButton::RB
    }
}

/// A saved Back or Start (from Linux) is replaced on Windows.
fn reserved_note() -> &'static str {
    tr!("Back and Start are reserved. This saved choice uses RB on Windows; choose another button to change it.")
}

fn controller_limits() -> &'static str {
    tr!("Windows does not block controller input: the game also receives Back + Start and the buttons used in the overlay.")
}

fn controller_support() -> &'static str {
    tr!("Only XInput controllers are supported. Back + Start replaces Guide; the real Guide button is not available through XInput and Game Bar or Steam may use it.")
}

/// The Guide screenshot toggle and its button, here and in Settings →
/// Screenshots.
pub(super) fn pad_shot_card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let p = &d.overlay.prefs;
    let options: Vec<(SharedString, bool)> =
        screenshot_buttons().iter().map(|b| (button_label(*b).into(), *b == screenshot_button(p))).collect();
    let select = select_field(
        "overlay-shot-button",
        None,
        button_label(screenshot_button(p)),
        false,
        d.overlay.button_open,
        options,
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.overlay.button_open = !d.overlay.button_open;
            cx.notify();
        },
        |d: &mut SettingsPage, i, _, cx| {
            d.overlay.button_open = false;
            update(d, cx, |p| p.screenshot_button = screenshot_buttons()[i]);
        },
    );
    card([with_sub_rows(
        toggle_row(
            "overlay-shot",
            shot_title(p),
            Some(tr!("Saved in the game's folder of Settings → Screenshots, with a notice in the corner.")),
            p.screenshot_shortcut,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.screenshot_shortcut = !p.screenshot_shortcut)))
        .into_any_element(),
        vec![row(
            labels::button_held(),
            Some(if screenshot_button(p) != p.screenshot_button { reserved_note() } else { labels::screenshot_button_desc() }),
            div().w(px(220.)).child(select),
        )
        .into_any_element()],
    )])
}

fn moment_desc(p: &OverlayPrefs) -> &'static str {
    if p.screenshot_shortcut && p.screenshot_button == ChordButton::Y {
        labels::moment_taken_by_screenshot()
    } else {
        tr!("Adds the time and a screenshot to the game's journal, to write about later in the overlay's Notes tab or the game's Journal.")
    }
}

fn replay_desc(p: &OverlayPrefs) -> &'static str {
    if p.screenshot_shortcut && p.screenshot_button == ChordButton::X {
        tr!("Off while Guide + X takes the screenshot: pick another button for it.")
    } else if !p.enabled {
        tr!("Needs the overlay: turn it on in Settings → Overlay.")
    } else {
        tr!("Saves the replay (Settings → Replay) and marks it as a moment in the game's journal.")
    }
}

/// The Guide + X toggle, here and in Settings → Replay.
pub(super) fn replay_shortcut_row(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = &d.overlay.prefs;
    toggle_row("overlay-replay", tr!("Guide + X: save the replay"), Some(replay_desc(p)), p.replay_shortcut)
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.replay_shortcut = !p.replay_shortcut)))
        .into_any_element()
}

/// [`replay_shortcut_row`] in a card of its own.
pub(super) fn replay_shortcut_card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    card([replay_shortcut_row(d, cx)])
}

/// Console row of Guide + X, here and in Settings → Replay.
pub(super) fn console_replay_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    if !supported(Feature::Replay) {
        return Vec::new();
    }
    let p = &d.overlay.prefs;
    vec![crate::console::list::Row::toggle("pad-replay", tr!("Guide + X: save the replay"), p.replay_shortcut).desc(replay_desc(p))]
}

/// Applies a change of [`console_replay_rows`]; `false` when it is not one.
pub(super) fn console_apply_replay(d: &mut SettingsPage, outcome: &crate::console::list::Outcome, cx: &mut Context<SettingsPage>) -> bool {
    use crate::console::list::{Outcome, Value};
    if !supported(Feature::Replay) {
        return false;
    }
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "pad-replay" => {
            let on = *on;
            update(d, cx, |p| p.replay_shortcut = on);
            true
        }
        _ => false,
    }
}

fn hotkey_status(d: &SettingsPage, cx: &App) -> Option<(String, bool)> {
    let p = &d.overlay.prefs;
    if !p.enabled || !p.keyboard_shortcut {
        return None;
    }
    let state = crate::overlay::get(cx).map(|s| s.read(cx).hotkey_state().clone()).unwrap_or_default();
    Some(match state {
        HotkeyState::Idle => (tr!("The shortcut activates automatically when a game starts.").into(), false),
        HotkeyState::Registering => (tr!("Registering the shortcut…").into(), false),
        HotkeyState::Ready(_) => (tr!("Shortcut registered and ready.").into(), false),
        HotkeyState::Failed(e) => (e, true),
    })
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = d.overlay.prefs.clone();
    let steps: Vec<(u8, &'static str)> = VOLUME_STEPS.iter().map(|s| (*s, step_label(*s))).collect();
    let step_tabs = {
        let page = cx.entity().downgrade();
        tabs("overlay-step", &steps, p.volume_step, move |v, _, cx| {
            page.update(cx, |d, cx| update(d, cx, |p| p.volume_step = v)).ok();
        })
    };
    let side_tabs = {
        let page = cx.entity().downgrade();
        tabs("overlay-side", &[(Side::Left, tr!("Left")), (Side::Right, tr!("Right"))], p.side, move |v, _, cx| {
            page.update(cx, |d, cx| update(d, cx, |p| p.side = v)).ok();
        })
    };
    let (desktop, desktop_ok) = crate::overlay::desktop_status();
    let status_line = |text: String, ok: bool| {
        div().text_size(px(13.)).text_color(if ok { theme::text_primary() } else { theme::danger_text() }).max_w(px(420.)).child(text)
    };
    let found = d.overlay.controllers.as_ref().is_some_and(|l| !l.is_empty()) || d.overlay.controllers.is_none();

    let beta = div()
        .flex()
        .items_start()
        .gap(px(10.))
        .mx(px(6.))
        .mb(px(4.))
        .child(status_pill(tr!("Beta"), theme::console_btn_y()))
        .child(div().flex_1().min_w_0().child(meta_text(tr!(
            "The overlay is new and still being tested: it may not work with every controller, game or desktop. It stays off until you turn it on here."
        ))));

    let main = card([with_sub_rows(
        toggle_row("overlay-enabled", tr!("Turn on the overlay"), Some(labels::turn_on_desc()), p.enabled)
            .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.enabled = !p.enabled)))
            .into_any_element(),
        vec![toggle_row("overlay-without-game", tr!("Open it without a game too"), Some(labels::without_game_desc()), p.without_game)
            .when(!p.enabled, |r| r.opacity(0.5))
            .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.without_game = !p.without_game)))
            .into_any_element()],
    )]);

    let focused = d.overlay.hotkey.is_focused(window);
    let hotkey = hotkey_box(
        div().id("overlay-hotkey").track_focus(&d.overlay.hotkey).cursor_pointer().on_key_down(cx.listener(
            |d, ev: &KeyDownEvent, _, cx| {
                cx.stop_propagation();
                if let Some(a) = accelerator_from_key(ev) {
                    update(d, cx, |p| p.keyboard_accelerator = a);
                }
            },
        )),
        p.keyboard_accelerator.clone(),
        focused,
    );
    let keyboard = card([with_sub_rows(
        toggle_row(
            "overlay-keyboard",
            tr!("Open it with a keyboard shortcut"),
            Some(if cfg!(windows) {
                tr!("The same shortcut closes it. The panel takes keyboard focus: arrows, Enter, Esc and Q / E for the tabs. Games using raw input may still receive keys.")
            } else {
                tr!("The same shortcut closes it. Opened from the keyboard, the keyboard drives it: arrows, Enter, Esc and Q / E for the tabs; the game gets no keys until it closes.")
            }),
            p.keyboard_shortcut,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.keyboard_shortcut = !p.keyboard_shortcut)))
        .into_any_element(),
        vec![div()
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(labeled(tr!("Keyboard shortcut"), hotkey))
            .child(meta_text(tr!(
                "Click the field and press the key combination ({default} unless changed). It is registered with the desktop only while the overlay can open.",
                default = DEFAULT_KEYBOARD_ACCELERATOR
            )))
            .children(hotkey_status(d, cx).map(|(text, error)| {
                div().text_size(px(12.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
            }))
            .into_any_element()],
    )]);

    let shot = pad_shot_card(d, cx);
    let volume = card([
        toggle_row(
            "overlay-volume",
            labels::volume(),
            Some(tr!("Each press moves the system volume one step and shows the level.")),
            p.volume_shortcut,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.volume_shortcut = !p.volume_shortcut)))
        .into_any_element(),
        row(tr!("Volume step"), Some(tr!("Also used by the volume rows of the overlay.")), step_tabs).into_any_element(),
    ]);

    let moment = card([toggle_row("overlay-moment", labels::moment(), Some(moment_desc(&p)), p.moment_shortcut)
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.moment_shortcut = !p.moment_shortcut)))
        .into_any_element()]);

    let look = card([row(tr!("Side of the screen"), None, side_tabs).into_any_element()]);

    let status = card([
        row(tr!("Desktop"), None, status_line(desktop, desktop_ok)).into_any_element(),
        row(tr!("Controllers"), None, status_line(controllers_text(d.overlay.controllers.as_ref()), found)).into_any_element(),
    ]);

    let notes = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(6.))
        .when(cfg!(windows), |d| d.child(meta_text(controller_limits())).child(meta_text(controller_support())))
        .when(!cfg!(windows), |d| {
            d.child(meta_text(tr!("While the overlay is open, or the Guide button is held, the game does not get the controller's buttons.")))
                .child(meta_text(tr!(
                    "If Steam is open with Steam Input on, it uses the Guide button too: turn off \"Guide Button Focuses Steam\" in Steam → Settings → Controller."
                )))
                .child(meta_text(tr!(
                    "Games that read a controller directly (some PlayStation and Switch controllers) may still get the presses."
                )))
        });

    let dim = |enabled: bool, el: Div| div().when(!enabled, |d| d.opacity(0.5)).child(el);
    div()
        .flex()
        .flex_col()
        .child(beta)
        .child(caption(tr!("Overlay")))
        .child(main)
        .child(caption(tr!("Keyboard")))
        .child(dim(p.enabled, keyboard))
        .child(caption(tr!("Shortcuts without opening it")))
        .child(dim(
            p.enabled,
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(shot)
                .child(volume)
                .child(moment)
                .when(supported(Feature::Replay), |col| col.child(replay_shortcut_card(d, cx))),
        ))
        .child(caption(tr!("Position")))
        .child(dim(p.enabled, look))
        .child(caption(tr!("Status")))
        .child(status)
        .child(caption(tr!("Good to know")))
        .child(notes)
        .into_any_element()
}

/// Shortcuts offered in console mode (a controller cannot record one).
const CONSOLE_ACCELERATORS: [&str; 4] = if cfg!(windows) {
    ["CommandOrControl+Shift+G", "CommandOrControl+Shift+O", "CommandOrControl+Alt+G", "CommandOrControl+Alt+O"]
} else {
    ["Super+G", "Super+O", "Super+Shift+G", "CommandOrControl+Shift+G"]
};

fn console_accelerators(current: &str) -> Vec<String> {
    let mut v: Vec<String> = CONSOLE_ACCELERATORS.iter().map(|s| s.to_string()).collect();
    if !v.iter().any(|s| s == current) {
        v.insert(0, current.to_string());
    }
    v
}

/// Console rows of the Guide screenshot, here and in Settings → Screenshots.
pub(super) fn console_pad_shot_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let p = &d.overlay.prefs;
    let index = screenshot_buttons().iter().position(|b| *b == screenshot_button(p)).unwrap_or(0);
    vec![
        Row::toggle("pad-shot", shot_title(p), p.screenshot_shortcut).desc(if cfg!(windows) { controller_limits() } else { "" }),
        Row::choice(
            "pad-shot-button",
            labels::button_held(),
            screenshot_buttons().iter().map(|b| button_label(*b).into()).collect(),
            index,
        )
        .desc(if screenshot_button(p) != p.screenshot_button { reserved_note() } else { "" }),
    ]
}

/// Applies a change of [`console_pad_shot_rows`]; `false` when it is not one.
pub(super) fn console_apply_pad_shot(
    d: &mut SettingsPage,
    outcome: &crate::console::list::Outcome,
    cx: &mut Context<SettingsPage>,
) -> bool {
    use crate::console::list::{Outcome, Value};
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "pad-shot" => {
            let on = *on;
            update(d, cx, |p| p.screenshot_shortcut = on);
        }
        Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "pad-shot-button" => {
            let button = screenshot_buttons().get(*index).copied().unwrap_or_default();
            update(d, cx, |p| p.screenshot_button = button);
        }
        _ => return false,
    }
    true
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let p = &d.overlay.prefs;
    let step_index = VOLUME_STEPS.iter().position(|s| *s == p.volume_step).unwrap_or(1);
    let accelerators = console_accelerators(&p.keyboard_accelerator);
    let accel_index = accelerators.iter().position(|a| *a == p.keyboard_accelerator).unwrap_or(0);
    let (desktop, desktop_ok) = crate::overlay::desktop_status();
    let found = match &d.overlay.controllers {
        None => String::new(),
        Some(l) => trn!(l.len(), "{n} controller found", "{n} controllers found"),
    };
    let mut rows = vec![
        Row::info("beta", tr!("Beta"), tr!("Being tested")).desc(tr!("New and still being tested; off until you turn it on.")),
        Row::toggle("enabled", tr!("Turn on the overlay"), p.enabled).desc(labels::turn_on_short()),
        Row::toggle("without-game", tr!("Open it without a game too"), p.without_game).desc(labels::without_game_short()),
        Row::header(tr!("Keyboard")),
        Row::toggle("keyboard", tr!("Open it with a keyboard shortcut"), p.keyboard_shortcut)
            .desc(tr!("The keyboard drives it while open: arrows, Enter, Esc, Q / E")),
        Row::choice("accelerator", tr!("Keyboard shortcut"), accelerators.into_iter().map(SharedString::from).collect(), accel_index)
            .desc(hotkey_status(d, cx).map(|(text, _)| text).unwrap_or_default()),
        Row::header(tr!("Shortcuts without opening it")),
    ];
    rows.extend(console_pad_shot_rows(d));
    rows.extend([
        Row::toggle("volume", labels::volume(), p.volume_shortcut),
        Row::toggle("moment", labels::moment(), p.moment_shortcut).desc(moment_desc(p)),
    ]);
    rows.extend(console_replay_rows(d));
    rows.extend([
        Row::choice("step", tr!("Volume step"), VOLUME_STEPS.iter().map(|s| SharedString::from(step_label(*s))).collect(), step_index),
        Row::header(tr!("Position")),
        Row::choice("side", tr!("Side of the screen"), vec![tr!("Left").into(), tr!("Right").into()], usize::from(p.side == Side::Right)),
        Row::header(tr!("Status")),
        Row::info("desktop", tr!("Desktop"), if desktop_ok { tr!("Supported") } else { tr!("Not supported") }).desc(desktop),
        Row::info("controllers", tr!("Controllers"), found).desc(controllers_text(d.overlay.controllers.as_ref())),
    ]);
    if cfg!(windows) {
        rows.extend([
            Row::header(tr!("Good to know")),
            Row::info("input-limits", tr!("Controller input"), "").desc(controller_limits()),
            Row::info("input-support", tr!("Controllers"), "").desc(controller_support()),
        ]);
    }
    rows
}

pub(super) fn console_apply(d: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    use crate::console::list::{Outcome, Value};
    if console_apply_pad_shot(d, &outcome, cx) || console_apply_replay(d, &outcome, cx) {
        return;
    }
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "enabled" => update(d, cx, |p| p.enabled = on),
            "keyboard" => update(d, cx, |p| p.keyboard_shortcut = on),
            "without-game" => update(d, cx, |p| p.without_game = on),
            "volume" => update(d, cx, |p| p.volume_shortcut = on),
            "moment" => update(d, cx, |p| p.moment_shortcut = on),
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index, options }) => match id.as_ref() {
            "accelerator" => {
                if let Some(a) = options.get(index) {
                    let a = a.to_string();
                    update(d, cx, |p| p.keyboard_accelerator = a);
                }
            }
            "step" => update(d, cx, |p| p.volume_step = VOLUME_STEPS.get(index).copied().unwrap_or(5)),
            "side" => update(d, cx, |p| p.side = if index == 1 { Side::Right } else { Side::Left }),
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        console_accelerators, screenshot_button, screenshot_buttons, ChordButton, OverlayPrefs, CONSOLE_ACCELERATORS,
        DEFAULT_KEYBOARD_ACCELERATOR,
    };

    #[test]
    fn console_shortcuts_include_the_default_and_keep_custom_values() {
        assert!(console_accelerators(DEFAULT_KEYBOARD_ACCELERATOR).iter().any(|a| a == DEFAULT_KEYBOARD_ACCELERATOR));
        let custom = console_accelerators("Alt+Shift+F8");
        assert_eq!(custom[0], "Alt+Shift+F8");
        assert_eq!(custom.len(), CONSOLE_ACCELERATORS.len() + 1);
    }

    #[test]
    fn screenshot_buttons_leave_the_windows_guide_chord_free() {
        for button in [ChordButton::Start, ChordButton::Select] {
            let p = OverlayPrefs { screenshot_button: button, ..OverlayPrefs::default() };
            assert_eq!(screenshot_buttons().contains(&button), !cfg!(windows));
            assert_eq!(screenshot_button(&p), if cfg!(windows) { ChordButton::RB } else { button });
            assert_eq!(p.screenshot_button, button, "keep the saved choice for Linux");
        }
        for button in screenshot_buttons() {
            let p = OverlayPrefs { screenshot_button: button, ..OverlayPrefs::default() };
            assert_eq!(screenshot_button(&p), button);
        }
    }
}
