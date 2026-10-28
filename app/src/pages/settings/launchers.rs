//! Settings → Other launchers: Heroic, Lutris, Bottles and Faugus, whether each is on
//! this PC and how many games it has, and a switch to bring its games into
//! the Collection (kept in sync while on). Turning one off keeps its games;
//! "Remove them" takes them out. Games then in the Collection twice (added
//! here by hand, and from Lutris, Bottles or Faugus) are listed below the cards, to
//! merge, replace or keep both. The work is the service's
//! (`services::launcher_imports`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::playtime_label;
use heronite_data::launchers::Launcher;
use heronite_i18n::{tr, trn};
use heronite_import::duplicates::{Duplicate, Why};

use super::SettingsPage;
use crate::console::list::{Outcome, Row, Value};
use crate::icons::Icon;
use crate::services::launcher_imports::{LauncherImports, Resolve};
use crate::theme;
use crate::ui::controls::{button_sm, card_box, toggle, ButtonKind};
use crate::ui::icon;

pub struct State {
    svc: Entity<LauncherImports>,
    /// The launcher whose "Remove them" is waiting for its answer.
    confirm: Option<Launcher>,
}

impl State {
    pub fn new(cx: &mut Context<SettingsPage>) -> Self {
        let svc = crate::services::launcher_imports(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        Self { svc, confirm: None }
    }

    /// Drops the question about removing games; true when there was one.
    pub(super) fn dismiss(&mut self) -> bool {
        self.confirm.take().is_some()
    }

    /// `ask:<launcher id>` of `HERONITE_DEBUG_SETTINGS_ACTION`: its "Remove
    /// them" question opens, nothing is removed.
    pub(super) fn debug_ask(&mut self, id: &str) {
        self.confirm = Launcher::ALL.into_iter().find(|l| l.id() == id);
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
        Launcher::Bottles => tr!(
            "The programs you added to its bottles, with the covers of its Library. Bottles keeps starting them, in their bottle and with its settings."
        ),
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
        t.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
            this.launchers.confirm = None;
            svc.update(cx, |s, cx| s.set_enabled(launcher, !on, cx))
        }))
    });
    let mut details = div().flex().flex_col().gap(px(6.)).flex_1().min_w_0().child(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            // Flashes when the Settings search jumps to it.
            .child(
                div()
                    .relative()
                    .children(crate::ui::find::flash(launcher.name(), 6.))
                    .font_family(theme::font_heading())
                    .font_weight(theme::W_BOLD)
                    .text_size(px(16.))
                    .child(launcher.name()),
            )
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
        let asking = d.launchers.confirm == Some(launcher) && !busy;
        let mut line = div().mt(px(4.)).flex().items_center().gap(px(12.));
        if asking {
            line = line
                .child(div().flex_1().text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_muted()).child(trn!(
                    in_collection,
                    "{n} game from {launcher} leaves your Collection, with its status and notes. Its play time and journal come back if you bring it in again. Nothing changes in {launcher}.",
                    "{n} games from {launcher} leave your Collection, with their status and notes. Their play time and journal come back if you bring them in again. Nothing changes in {launcher}.",
                    launcher = launcher.name()
                )))
                .child(
                    button_sm(SharedString::from(format!("launchers-{id}-remove-no")), ButtonKind::Ghost, tr!("Cancel")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.launchers.confirm = None;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button_sm(SharedString::from(format!("launchers-{id}-remove-yes")), ButtonKind::Danger, tr!("Remove them")).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.launchers.confirm = None;
                            svc.update(cx, |s, cx| s.remove(launcher, cx));
                        }),
                    ),
                );
        } else {
            line = line
                .child(div().flex_1().text_size(px(12.5)).text_color(theme::text_muted()).child(trn!(
                    in_collection,
                    "{n} game from {launcher} is still in your Collection, no longer kept in sync.",
                    "{n} games from {launcher} are still in your Collection, no longer kept in sync.",
                    launcher = launcher.name()
                )))
                .child(button_sm(SharedString::from(format!("launchers-{id}-remove")), ButtonKind::Danger, tr!("Remove them")).when(
                    !busy,
                    |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.launchers.confirm = Some(launcher);
                            cx.notify();
                        }))
                    },
                ));
        }
        details = details.child(line);
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
        Launcher::Bottles => Icon::Inbox,
        Launcher::Faugus => Icon::WineBar,
    }
}

/// Why the two are taken for the same game.
fn why_text(dup: &Duplicate) -> String {
    let launcher = dup.launcher.name();
    match dup.why {
        Why::Prefix => tr!("It runs in the Wine prefix of {launcher}'s game.", launcher),
        Why::Folder => tr!("Its program is in the folder of {launcher}'s game.", launcher),
        Why::Title => tr!("Same name.").into(),
    }
}

/// One side of a pair: title, and where it is from with its play time.
fn side(title: &str, from: String, minutes: u64, accent: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_size(px(14.))
                .font_weight(theme::W_BOLD)
                .text_color(if accent { theme::accent() } else { theme::text_primary() })
                .truncate()
                .child(title.to_string()),
        )
        .child(div().text_size(px(12.5)).text_color(theme::text_muted()).child(format!("{from}  \u{b7}  {}", playtime_label(minutes))))
}

fn duplicate_row(svc: &Entity<LauncherImports>, dup: &Duplicate, busy: bool) -> Div {
    let id = dup.id();
    let button = |suffix: &str, kind: ButtonKind, label: &str, how: Resolve| {
        let (svc, dup) = (svc.clone(), dup.clone());
        button_sm(SharedString::from(format!("dup-{id}-{suffix}")), kind, label.to_string())
            .when(!busy, |b| b.on_click(move |_, _, cx| svc.update(cx, |s, cx| s.resolve(dup.clone(), how, cx))))
            .when(busy, |b| b.opacity(0.5))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(14.))
        .rounded(px(10.))
        .bg(theme::ink_a(0.04))
        .border_1()
        .border_color(theme::ink_a(0.08))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .child(side(&dup.mine.title, tr!("Added by you").into(), dup.mine.minutes, false))
                .child(icon(Icon::ChevronRight, px(18.), theme::text_dim()))
                .child(side(&dup.theirs.title, dup.launcher.name().to_string(), dup.theirs.minutes, true)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().text_size(px(12.5)).text_color(theme::text_dim()).child(why_text(dup)))
                .child(button("keep", ButtonKind::Ghost, tr!("Keep both"), Resolve::KeepBoth))
                .child(button("replace", ButtonKind::Glass, tr!("Replace"), Resolve::Replace))
                .child(button("merge", ButtonKind::Primary, if busy { tr!("Merging\u{2026}") } else { tr!("Merge") }, Resolve::Merge)),
        )
}

/// The games in the Collection twice, when there are any (or the outcome of
/// the last one settled).
fn duplicates_card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let s = d.launchers.svc.read(cx);
    if s.duplicates.is_empty() && s.resolved.is_none() {
        return None;
    }
    let svc = d.launchers.svc.clone();
    let n = s.duplicates.len();
    let rows: Vec<Div> = s.duplicates.iter().map(|dup| duplicate_row(&svc, dup, s.resolving.contains(&dup.id()))).collect();
    let resolved = s
        .resolved
        .clone()
        .map(|(text, error)| div().text_size(px(12.5)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text));
    let head = div()
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
                .child(icon(Icon::CallMerge, px(22.), theme::accent())),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_family(theme::font_heading())
                        .font_weight(theme::W_BOLD)
                        .text_size(px(16.))
                        .child(if n == 0 { tr!("Already in your Collection").to_string() } else { trn!(n, "{n} game is in your Collection twice", "{n} games are in your Collection twice") }),
                )
                .when(n > 0, |d| {
                    d.child(div().text_size(px(12.5)).line_height(px(18.)).text_color(theme::text_dim()).child(tr!(
                        "You added these here by hand, and their launcher has them too. Merge keeps the launcher's game, which it keeps starting with its own settings, and moves your play time, sessions, status, notes and journal to it. Replace removes the one you added, with its play time. Neither deletes any file."
                    )))
                })
                .children(resolved),
        );
    Some(card_box().py(px(16.)).gap(px(12.)).child(head).children(rows))
}

pub fn render(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let duplicates = duplicates_card(d, cx);
    let cards: Vec<Div> = Launcher::ALL.iter().map(|l| launcher_card(d, *l, cx)).collect();
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .children(duplicates)
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
    if !s.duplicates.is_empty() {
        rows.push(Row::header(trn!(s.duplicates.len(), "{n} game is in your Collection twice", "{n} games are in your Collection twice")));
        for dup in &s.duplicates {
            let busy = s.resolving.contains(&dup.id());
            let desc = format!("{} \u{2192} {} ({})", dup.mine.title, dup.theirs.title, dup.launcher.name());
            rows.push(
                Row::action(SharedString::from(format!("dup-merge|{}", dup.id())), tr!("Merge"))
                    .desc(desc)
                    .icon(Icon::CallMerge)
                    .enabled(!busy),
            );
            rows.push(Row::action(SharedString::from(format!("dup-replace|{}", dup.id())), tr!("Replace")).enabled(!busy));
            rows.push(Row::action(SharedString::from(format!("dup-keep|{}", dup.id())), tr!("Keep both")).enabled(!busy));
        }
    }
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
            if let Some((what, pair)) = id.split_once('|') {
                let how = match what {
                    "dup-merge" => Resolve::Merge,
                    "dup-replace" => Resolve::Replace,
                    _ => Resolve::KeepBoth,
                };
                let dup = svc.read(cx).duplicates.iter().find(|d| d.id() == pair).cloned();
                if let Some(dup) = dup {
                    svc.update(cx, |s, cx| s.resolve(dup, how, cx));
                }
            } else if let Some(l) = id.strip_suffix("-now").and_then(Launcher::from_source) {
                svc.update(cx, |s, cx| s.import(l, cx));
            }
        }
        _ => {}
    }
}
