//! Settings → Quick launcher: what it is, its keyboard shortcut (none unless
//! set; `quick-launch.json`, `heronite_data::quick_launch_prefs`), whether
//! the desktop took it, the command to bind where a desktop gives apps no
//! shortcut, and opening it now to try it. The service
//! (`crate::quick_launch::hotkey`) registers the shortcut while the app runs.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::SettingsPage;
use crate::icons::Icon;
use crate::overlay::HotkeyState;
use crate::theme;
use crate::ui::controls::{button, button_sm, caption, card, row, ButtonKind};
use crate::ui::fields::{accelerator_from_key, hotkey_box, labeled, meta_text};
use crate::ui::icon;

pub struct State {
    /// The shortcut field.
    hotkey: FocusHandle,
    /// The combination last pressed in it is another shortcut's.
    taken: Option<String>,
    /// What a key bound in the desktop's settings runs (Linux).
    command: Option<String>,
}

impl State {
    pub fn new(cx: &mut Context<SettingsPage>) -> Self {
        // The registration answers in the background: the status line follows it.
        if let Some(service) = crate::quick_launch::get(cx) {
            cx.observe(&service, |_, _, cx| cx.notify()).detach();
        }
        let command =
            cfg!(target_os = "linux").then(|| super::screenshots::desktop_command(&crate::system::shortcuts::own_exe(), "--quick-launch"));
        Self { hotkey: cx.focus_handle(), taken: None, command }
    }
}

fn shown_accelerator(a: &str) -> String {
    a.replace("CommandOrControl", "Ctrl")
}

fn accelerator(cx: &App) -> String {
    crate::quick_launch::get(cx).map(|s| s.read(cx).accelerator().to_string()).unwrap_or_default()
}

fn set_accelerator(accelerator: String, cx: &mut App) {
    if let Some(service) = crate::quick_launch::get(cx) {
        service.update(cx, |s, cx| s.set_accelerator(accelerator, cx));
    }
}

/// The shortcut set for the quick launcher, as the keyboard says it (the
/// other shortcut fields refuse it).
pub(super) fn shown(cx: &App) -> Option<String> {
    let a = accelerator(cx);
    (!a.is_empty()).then(|| shown_accelerator(&a))
}

/// What else uses `accelerator` (the overlay's, the screenshot's or the
/// replay's shortcut).
fn taken_by(d: &SettingsPage, accelerator: &str, cx: &App) -> Option<String> {
    let o = &d.overlay.prefs;
    let s = &d.settings.screenshots;
    let key = shown_accelerator(accelerator);
    if o.enabled && o.keyboard_shortcut && o.keyboard_accelerator.eq_ignore_ascii_case(accelerator) {
        Some(tr!("{key} already opens the overlay.", key))
    } else if s.capture_enabled && s.capture_accelerator.eq_ignore_ascii_case(accelerator) {
        Some(tr!("{key} already takes a screenshot.", key))
    } else if super::replay::accelerator(d, cx).is_some_and(|r| r.eq_ignore_ascii_case(&key)) {
        Some(tr!("{key} already saves the replay.", key))
    } else {
        None
    }
}

/// One step of "How it works": a key or an icon, then what happens.
fn step(lead: AnyElement, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(8.))
        .child(div().flex_none().w(px(150.)).flex().justify_end().child(lead))
        .child(div().flex_1().min_w_0().text_size(px(13.)).text_color(theme::text_primary()).child(text))
}

fn keys(names: &[&'static str]) -> AnyElement {
    div().flex().gap(px(4.)).children(names.iter().map(|n| crate::ui::controls::kbd(*n))).into_any_element()
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let accelerator = accelerator(cx);
    let focused = d.quick_launch.hotkey.is_focused(window);
    let text = match (accelerator.is_empty(), focused) {
        (false, _) => shown_accelerator(&accelerator),
        (true, true) => tr!("Press the keys…").into(),
        (true, false) => tr!("None").into(),
    };
    let field = hotkey_box(
        div().id("quick-launch-hotkey").track_focus(&d.quick_launch.hotkey).cursor_pointer().on_key_down(cx.listener(
            |d, ev: &KeyDownEvent, _, cx| {
                cx.stop_propagation();
                let k = &ev.keystroke;
                let plain = !(k.modifiers.control || k.modifiers.alt || k.modifiers.shift || k.modifiers.platform);
                let accelerator = if plain && matches!(k.key.as_str(), "backspace" | "delete") {
                    String::new()
                } else {
                    let Some(a) = accelerator_from_key(ev) else { return };
                    a
                };
                d.quick_launch.taken = (!accelerator.is_empty()).then(|| taken_by(d, &accelerator, cx)).flatten();
                if d.quick_launch.taken.is_none() {
                    set_accelerator(accelerator, cx);
                }
                cx.notify();
            },
        )),
        text,
        focused,
    )
    .when(accelerator.is_empty() && !focused, |b| b.text_color(theme::text_dim()));
    let clear = (!accelerator.is_empty()).then(|| {
        button("quick-launch-hotkey-clear", ButtonKind::Glass, tr!("Remove")).on_click(cx.listener(|d, _, _, cx| {
            d.quick_launch.taken = None;
            set_accelerator(String::new(), cx);
        }))
    });
    let status: Option<(String, bool)> = match &d.quick_launch.taken {
        Some(why) => Some((why.clone(), true)),
        None if accelerator.is_empty() => None,
        None => {
            let state = crate::quick_launch::get(cx).map(|s| s.read(cx).hotkey_state().clone()).unwrap_or_default();
            match state {
                HotkeyState::Idle => None,
                HotkeyState::Registering => Some((tr!("Registering the shortcut…").into(), false)),
                HotkeyState::Ready(_) => Some((tr!("Shortcut registered and ready.").into(), false)),
                HotkeyState::Failed(e) => Some((e, true)),
            }
        }
    };
    let shortcut = div()
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(labeled(
            tr!("Keyboard shortcut"),
            div().flex().items_center().gap(px(8.)).child(div().flex_1().child(field)).children(clear),
        ))
        .child(meta_text(tr!(
            "Opens the quick launcher from anywhere while Heronite runs, in the tray too; press it again to close it. Click the field and press the key combination; Backspace removes it."
        )))
        .children(status.map(|(text, error)| {
            div().text_size(px(12.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
        }));
    let command = d.quick_launch.command.clone().map(|command| {
        let description = tr!(
            "Where the desktop takes no shortcut from Heronite, bind a key to this command in its keyboard settings. It also starts Heronite when it is not running: {command}",
            command = &command
        );
        row(
            tr!("Open from a key of the desktop"),
            Some(&description),
            button_sm("quick-launch-copy-command", ButtonKind::Glass, tr!("Copy command"))
                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(command.clone()))),
        )
        .into_any_element()
    });
    let try_it = row(
        tr!("Try it"),
        Some(tr!("Opens it now, over this window.")),
        button_sm("quick-launch-open", ButtonKind::Primary, tr!("Open the quick launcher"))
            .on_click(|_, _, cx| crate::quick_launch::open(cx)),
    )
    .into_any_element();

    let enter_play = keys(&["\u{21b5}"]);
    let shift_enter = keys(&["\u{21e7}", "\u{21b5}"]);
    let how = card([div()
        .py(px(4.))
        .flex()
        .flex_col()
        .child(step(
            match shown(cx) {
                Some(a) => keys_owned(a),
                None => icon(Icon::FaSearch, px(16.), theme::accent()).into_any_element(),
            },
            tr!("Open it and type a few letters of a game, a page or a setting."),
        ))
        .child(step(enter_play, tr!("Enter plays the game, or stops it while it runs.")))
        .child(step(shift_enter, tr!("Shift Enter shows the game in Heronite; → lists its actions.")))
        .child(step(
            icon(Icon::OpenInNew, px(16.), theme::text_dim()).into_any_element(),
            tr!("Items marked like this open Heronite and run there."),
        ))
        .into_any_element()]);

    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(caption(tr!("How it works")))
        .child(how)
        .child(caption(tr!("Opening it")))
        .child(card([shortcut.into_any_element()].into_iter().chain(command).chain([try_it])))
        .into_any_element()
}

/// The keys of a shortcut ("Ctrl+Alt+Space") as one key cap each.
fn keys_owned(accelerator: String) -> AnyElement {
    div()
        .flex()
        .gap(px(4.))
        .children(accelerator.split('+').map(|k| crate::ui::controls::kbd(SharedString::from(k.to_string()))))
        .into_any_element()
}
