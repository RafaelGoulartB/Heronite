//! Settings → Other launchers: Heroic, Lutris and Faugus, whether each is on
//! this PC and how many games it has, and a switch to bring its games into
//! the Collection (kept in sync while on). Turning one off keeps its games;
//! "Remove them" takes them out. The work is the service's
//! (`services::launcher_imports`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::launchers::Launcher;
use heronite_i18n::{tr, trn};

use super::SettingsPage;
use crate::console::list::{Outcome, Row, Value};
use crate::icons::Icon;
use crate::services::launcher_imports::LauncherImports;
use crate::theme;
use crate::ui::controls::{button_sm, card_box, toggle, ButtonKind};
use crate::ui::icon;

pub struct State {
    svc: Entity<LauncherImports>,
}

impl State {
    pub fn new(cx: &mut Context<SettingsPage>) -> Self {
        let svc = crate::services::launcher_imports(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        Self { svc }
    }
}

/// Looks for the launchers again when the section shows.
pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.launchers.svc.update(cx, |s, cx| s.look(cx));
}

/// What importing a launcher brings, in a line.
pub fn what(launcher: Launcher) -> &'static str {
    match launcher {
        Launcher::Heroic => tr!(
            "The games you added by hand there start here, with their settings. The Epic, GOG and Amazon games it installed show as installed once you log in to the store here."
        ),
        Launcher::Lutris => tr!("Its installed games, with their covers and play time. Lutris keeps starting them, with the settings you gave them there."),
        Launcher::Faugus => tr!("Its games, with their covers and play time. Faugus keeps starting them, with the settings you gave them there."),
    }
}

/// "23 games · Flatpak", or that it is not on this PC.
fn found_line(s: &LauncherImports, launcher: Launcher) -> (String, bool) {
    match (s.found.as_ref(), s.summary(launcher)) {
        (None, _) => (tr!("Looking\u{2026}").into(), true),
        (Some(_), None) => (tr!("Not found on this PC").into(), false),
        (Some(_), Some(f)) => {
            let mut parts = vec![trn!(f.games, "{n} game", "{n} games")];
            if f.flatpak {
                parts.push("Flatpak".into());
            }
            (parts.join("  \u{b7}  "), true)
        }
    }
}

/// When it was last brought in: "In your Collection: 23 · last 19 Oct, 14:02".
fn sync_line(s: &LauncherImports, launcher: Launcher) -> Option<String> {
    let n = s.in_collection.get(&launcher).copied().unwrap_or(0);
    let mut line = trn!(n, "{n} game in your Collection", "{n} games in your Collection");
    if let Some(at) = s.prefs.last.get(&launcher).and_then(|l| chrono::DateTime::parse_from_rfc3339(&l.at).ok()) {
        let at = at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M");
        line = format!("{line}  \u{b7}  {}", tr!("brought in {when}", when = at.to_string()));
    }
    Some(line)
}

fn launcher_card(d: &SettingsPage, launcher: Launcher, cx: &mut Context<SettingsPage>) -> Div {
    let s = d.launchers.svc.read(cx);
    let on = s.prefs.enabled.contains(&launcher);
    let busy = s.busy.contains(&launcher);
    let (found, here) = found_line(s, launcher);
    let in_collection = s.in_collection.get(&launcher).copied().unwrap_or(0);
    let error = s.errors.get(&launcher).cloned();
    let id = launcher.id();
    let svc = d.launchers.svc.clone();
    let switch = toggle(SharedString::from(format!("launchers-{id}-sw")), on, !here && !on).when(here || on, |t| {
        let svc = svc.clone();
        t.cursor_pointer().on_click(move |_, _, cx| svc.update(cx, |s, cx| s.set_enabled(launcher, !on, cx)))
    });
    let mut details = div().flex().flex_col().gap(px(6.)).flex_1().min_w_0().child(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().font_family(theme::font_heading()).font_weight(theme::W_BOLD).text_size(px(16.)).child(launcher.name()))
            .child(div().text_size(px(12.5)).text_color(if here { theme::text_muted() } else { theme::text_dim() }).child(found)),
    );
    details = details.child(div().text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_dim()).child(what(launcher)));
    if on {
        let svc = svc.clone();
        details = details.child(
            div()
                .mt(px(4.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().flex_1().text_size(px(12.5)).text_color(theme::text_muted()).children(sync_line(s, launcher)))
                .child(
                    button_sm(
                        SharedString::from(format!("launchers-{id}-now")),
                        ButtonKind::Glass,
                        if busy { tr!("Bringing in\u{2026}") } else { tr!("Bring in now") },
                    )
                    .when(!busy, |b| b.on_click(move |_, _, cx| svc.update(cx, |s, cx| s.import(launcher, cx)))),
                ),
        );
    } else if in_collection > 0 {
        let svc = svc.clone();
        details = details.child(
            div()
                .mt(px(4.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().flex_1().text_size(px(12.5)).text_color(theme::text_muted()).child(trn!(
                    in_collection,
                    "{n} game from {launcher} is still in your Collection, no longer kept in sync.",
                    "{n} games from {launcher} are still in your Collection, no longer kept in sync.",
                    launcher = launcher.name()
                )))
                .child(
                    button_sm(SharedString::from(format!("launchers-{id}-remove")), ButtonKind::Danger, tr!("Remove them"))
                        .when(!busy, |b| b.on_click(move |_, _, cx| svc.update(cx, |s, cx| s.remove(launcher, cx)))),
                ),
        );
    }
    if let Some(e) = error {
        details = details.child(div().text_size(px(12.5)).text_color(theme::danger_text()).child(e));
    }
    card_box().py(px(16.)).when(!here && !on, |c| c.opacity(0.6)).child(
        div()
            .flex()
            .items_start()
            .gap(px(14.))
            .child(
                div()
                    .flex_none()
                    .size(px(42.))
                    .rounded(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::accent_alpha(0.14))
                    .child(icon(launcher_icon(launcher), px(22.), theme::accent())),
            )
            .child(details)
            .child(div().flex_none().pt(px(2.)).child(switch)),
    )
}

pub fn launcher_icon(launcher: Launcher) -> Icon {
    match launcher {
        Launcher::Heroic => Icon::CloudDownload,
        Launcher::Lutris => Icon::SportsEsports,
        Launcher::Faugus => Icon::WineBar,
    }
}

pub fn render(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let cards: Vec<Div> = Launcher::ALL.iter().map(|l| launcher_card(d, *l, cx)).collect();
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .children(cards)
        .child(div().mx(px(6.)).mt(px(4.)).text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_dim()).child(tr!(
            "Nothing is changed in the other launchers. A game removed there stays here as not installed; its play time and notes are kept."
        )))
        .into_any_element()
}

/// Console mode rows: a switch per launcher found, and "Bring in now".
pub fn console_rows(d: &SettingsPage, cx: &App) -> Vec<Row> {
    let s = d.launchers.svc.read(cx);
    let mut rows = Vec::new();
    for launcher in Launcher::ALL {
        let (found, here) = found_line(s, launcher);
        let on = s.prefs.enabled.contains(&launcher);
        rows.push(Row::header(launcher.name()));
        if !here && !on {
            rows.push(Row::info(SharedString::from(format!("{}-none", launcher.id())), launcher.name(), found));
            continue;
        }
        rows.push(Row::toggle(SharedString::from(launcher.id()), tr!("Bring its games in"), on).desc(found));
        if on {
            let busy = s.busy.contains(&launcher);
            rows.push(
                Row::action(
                    SharedString::from(format!("{}-now", launcher.id())),
                    if busy { tr!("Bringing in\u{2026}") } else { tr!("Bring in now") },
                )
                .desc(sync_line(s, launcher).unwrap_or_default())
                .icon(Icon::Refresh)
                .enabled(!busy),
            );
        }
    }
    rows
}

pub fn console_apply(d: &mut SettingsPage, outcome: Outcome, cx: &mut Context<SettingsPage>) {
    let svc = d.launchers.svc.clone();
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) => {
            if let Some(l) = Launcher::from_source(&id) {
                svc.update(cx, |s, cx| s.set_enabled(l, on, cx));
            }
        }
        Outcome::Activate(id) => {
            if let Some(l) = id.strip_suffix("-now").and_then(Launcher::from_source) {
                svc.update(cx, |s, cx| s.import(l, cx));
            }
        }
        _ => {}
    }
}
