//! Settings → Overlay: the in-game overlay (beta, off by default), its
//! shortcuts and side, and whether this desktop and the controllers can use
//! it. The options live in `overlay.json` (`heronite_data::overlay_prefs`)
//! and reach the running service at once.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::overlay_prefs::{OverlayPrefs, Side, VOLUME_STEPS};
use heronite_i18n::{tr, trn};
use heronite_overlay::input::Controller;

use super::SettingsPage;
use crate::pages::{status_pill, PageCtx};
use crate::theme;
use crate::ui::controls::{caption, card, row, tabs, toggle_row};
use crate::ui::fields::meta_text;

pub struct State {
    prefs: OverlayPrefs,
    /// `None` while they are looked for.
    controllers: Option<Vec<Controller>>,
}

impl State {
    pub fn new(ctx: &PageCtx) -> Self {
        // A tiny file in the app's config folder.
        Self { prefs: OverlayPrefs::load(&ctx.app), controllers: None }
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

/// Saves the options (off the UI thread) and hands them to the overlay.
fn save(d: &mut SettingsPage, prefs: OverlayPrefs, cx: &mut Context<SettingsPage>) {
    d.overlay.prefs = prefs;
    let app = d.ctx.app.clone();
    cx.background_spawn(async move {
        if let Err(e) = prefs.save(&app) {
            log::warn!("saving the overlay options: {e:#}");
        }
    })
    .detach();
    if let Some(service) = crate::overlay::get(cx) {
        service.update(cx, |s, cx| s.set_prefs(prefs, cx));
    }
    cx.notify();
}

/// Whether this desktop can draw the overlay over games: it needs Wayland's
/// layer-shell, which GNOME does not have.
fn desktop_status() -> (String, bool) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return (tr!("Not supported on X11: the overlay needs a Wayland session.").into(), false);
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop.to_ascii_lowercase().contains("gnome") {
        return (tr!("GNOME cannot draw over games (it lacks the layer-shell protocol).").into(), false);
    }
    (tr!("Supported (Wayland with layer-shell, like Hyprland, KDE Plasma and Sway).").into(), true)
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

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = d.overlay.prefs;
    let steps: Vec<(u8, &'static str)> = VOLUME_STEPS.iter().map(|s| (*s, step_label(*s))).collect();
    let step_tabs = {
        let page = cx.entity().downgrade();
        tabs("overlay-step", &steps, p.volume_step, move |v, _, cx| {
            page.update(cx, |d, cx| {
                let prefs = OverlayPrefs { volume_step: v, ..d.overlay.prefs };
                save(d, prefs, cx);
            })
            .ok();
        })
    };
    let side_tabs = {
        let page = cx.entity().downgrade();
        tabs("overlay-side", &[(Side::Left, tr!("Left")), (Side::Right, tr!("Right"))], p.side, move |v, _, cx| {
            page.update(cx, |d, cx| {
                let prefs = OverlayPrefs { side: v, ..d.overlay.prefs };
                save(d, prefs, cx);
            })
            .ok();
        })
    };
    let (desktop, desktop_ok) = desktop_status();
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

    let main = card([toggle_row(
        "overlay-enabled",
        tr!("Turn on the overlay"),
        Some(tr!("During a game opened by Heronite, press the controller's Guide button (the centre one) to open it; press it again or B to close it.")),
        p.enabled,
    )
    .on_click(cx.listener(|d, _, _, cx| {
        let prefs = OverlayPrefs { enabled: !d.overlay.prefs.enabled, ..d.overlay.prefs };
        save(d, prefs, cx);
    }))
    .into_any_element()]);

    let shortcuts = card([
        toggle_row(
            "overlay-shot",
            tr!("Guide + RB: take a screenshot"),
            Some(tr!("Saved in the game's folder of Settings → Screenshots, with a notice in the corner.")),
            p.screenshot_shortcut,
        )
        .on_click(cx.listener(|d, _, _, cx| {
            let prefs = OverlayPrefs { screenshot_shortcut: !d.overlay.prefs.screenshot_shortcut, ..d.overlay.prefs };
            save(d, prefs, cx);
        }))
        .into_any_element(),
        toggle_row(
            "overlay-volume",
            tr!("Guide + ↑ / ↓: change the volume"),
            Some(tr!("Each press moves the system volume one step and shows the level.")),
            p.volume_shortcut,
        )
        .on_click(cx.listener(|d, _, _, cx| {
            let prefs = OverlayPrefs { volume_shortcut: !d.overlay.prefs.volume_shortcut, ..d.overlay.prefs };
            save(d, prefs, cx);
        }))
        .into_any_element(),
        row(tr!("Volume step"), Some(tr!("Also used by the volume rows of the overlay.")), step_tabs).into_any_element(),
    ]);

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
        .child(meta_text(tr!("While the overlay is open, or the Guide button is held, the game does not get the controller's buttons.")))
        .child(meta_text(tr!(
            "If Steam is open with Steam Input on, it uses the Guide button too: turn off \"Guide Button Focuses Steam\" in Steam → Settings → Controller."
        )))
        .child(meta_text(tr!(
            "Games that read a controller directly (some PlayStation and Switch controllers) may still get the presses."
        )));

    let enabled = p.enabled;
    div()
        .flex()
        .flex_col()
        .child(beta)
        .child(caption(tr!("Overlay")))
        .child(main)
        .child(caption(tr!("Shortcuts without opening it")))
        .child(div().when(!enabled, |d| d.opacity(0.5)).child(shortcuts))
        .child(caption(tr!("Position")))
        .child(div().when(!enabled, |d| d.opacity(0.5)).child(look))
        .child(caption(tr!("Status")))
        .child(status)
        .child(caption(tr!("Good to know")))
        .child(notes)
        .into_any_element()
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let p = d.overlay.prefs;
    let step_index = VOLUME_STEPS.iter().position(|s| *s == p.volume_step).unwrap_or(1);
    let (desktop, desktop_ok) = desktop_status();
    let found = match &d.overlay.controllers {
        None => String::new(),
        Some(l) => trn!(l.len(), "{n} controller found", "{n} controllers found"),
    };
    vec![
        Row::info("beta", tr!("Beta"), tr!("Being tested")).desc(tr!("New and still being tested; off until you turn it on.")),
        Row::toggle("enabled", tr!("Turn on the overlay"), p.enabled)
            .desc(tr!("The Guide button opens it during a game opened by Heronite")),
        Row::header(tr!("Shortcuts without opening it")),
        Row::toggle("shot", tr!("Guide + RB: take a screenshot"), p.screenshot_shortcut),
        Row::toggle("volume", tr!("Guide + ↑ / ↓: change the volume"), p.volume_shortcut),
        Row::choice("step", tr!("Volume step"), VOLUME_STEPS.iter().map(|s| SharedString::from(step_label(*s))).collect(), step_index),
        Row::header(tr!("Position")),
        Row::choice("side", tr!("Side of the screen"), vec![tr!("Left").into(), tr!("Right").into()], usize::from(p.side == Side::Right)),
        Row::header(tr!("Status")),
        Row::info("desktop", tr!("Desktop"), if desktop_ok { tr!("Supported") } else { tr!("Not supported") }).desc(desktop),
        Row::info("controllers", tr!("Controllers"), found).desc(controllers_text(d.overlay.controllers.as_ref())),
    ]
}

pub(super) fn console_apply(d: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    use crate::console::list::{Outcome, Value};
    let mut p = d.overlay.prefs;
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "enabled" => p.enabled = on,
            "shot" => p.screenshot_shortcut = on,
            "volume" => p.volume_shortcut = on,
            _ => return,
        },
        Outcome::Changed(id, Value::Choice { index, .. }) => match id.as_ref() {
            "step" => p.volume_step = VOLUME_STEPS.get(index).copied().unwrap_or(5),
            "side" => p.side = if index == 1 { Side::Right } else { Side::Left },
            _ => return,
        },
        _ => return,
    }
    save(d, p, cx);
}
