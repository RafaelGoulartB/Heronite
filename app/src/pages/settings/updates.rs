//! Settings → About → Updates: the state of `services::update`, its
//! switches and the notes of the release on offer.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_update::Unsupported;

use super::SettingsPage;
use crate::console::list::{Outcome, Row, Value};
use crate::services::update::{self, notes, Phase, Updater};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card, card_box, row, toggle, toggle_row, ButtonKind};

/// The main row: what is going on and the one action that fits.
struct Status {
    title: String,
    detail: String,
    action: Option<(&'static str, ButtonKind, bool, Action)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Check,
    Install,
    OpenPage,
    Cancel,
    Restart,
}

fn status(u: &Updater, cx: &App) -> Status {
    let current = u.current().to_string();
    let version = u.release().map(|r| r.version.clone()).unwrap_or_default();
    let checked = u.last_check().map(update::ago_text);
    let get = |what: Action| if u.can_install() { what } else { Action::OpenPage };
    match u.phase() {
        Phase::Idle => Status {
            title: format!("Heronite {current}"),
            detail: checked.map(|a| format!("Last checked {a}")).unwrap_or_else(|| "Not checked yet".into()),
            action: Some(("Check for updates", ButtonKind::Glass, true, Action::Check)),
        },
        Phase::Checking => Status {
            title: "Checking for updates\u{2026}".into(),
            detail: "Asking GitHub for the latest release".into(),
            action: Some(("Checking\u{2026}", ButtonKind::Glass, false, Action::Check)),
        },
        Phase::UpToDate => Status {
            title: "Heronite is up to date".into(),
            detail: format!("Version {current} \u{b7} checked {}", checked.unwrap_or_else(|| "just now".into())),
            action: Some(("Check again", ButtonKind::Glass, true, Action::Check)),
        },
        Phase::Available => {
            let r = u.release();
            let mut detail = format!("You have {current}");
            if let Some(r) = r.filter(|r| !r.date().is_empty()) {
                detail.push_str(&format!(" \u{b7} released {}", r.date()));
            }
            if let Some(a) = r.and_then(|r| r.appimage.as_ref()).filter(|a| a.size > 0) {
                detail.push_str(&format!(" \u{b7} {}", update::size_text(a.size)));
            }
            let label = if u.can_install() { "Update" } else { "Download" };
            Status {
                title: format!("Heronite {version} is available"),
                detail,
                action: Some((label, ButtonKind::Primary, true, get(Action::Install))),
            }
        }
        Phase::Downloading(p) => Status {
            title: format!("Downloading Heronite {version}"),
            detail: update::progress_text(p),
            action: Some(("Cancel", ButtonKind::Ghost, true, Action::Cancel)),
        },
        Phase::Ready => {
            let blocker = crate::system::restart_blocker(cx);
            Status {
                title: format!("Heronite {version} is installed"),
                detail: match &blocker {
                    Some(why) => format!("{why}: restart later, or it starts the next time you open Heronite."),
                    None => "Restart to use it now, or it starts the next time you open Heronite.".into(),
                },
                action: Some(("Restart now", ButtonKind::Primary, blocker.is_none(), Action::Restart)),
            }
        }
        Phase::CheckFailed(e) => Status {
            title: "Could not check for updates".into(),
            detail: e.clone(),
            action: Some(("Try again", ButtonKind::Glass, true, Action::Check)),
        },
        Phase::InstallFailed(e) => Status {
            title: format!("Could not update to {version}"),
            detail: e.clone(),
            action: Some(("Try again", ButtonKind::Primary, true, get(Action::Install))),
        },
    }
}

/// Why this copy cannot install updates by itself, for the switches.
fn unsupported_text(why: &Unsupported) -> String {
    match why {
        Unsupported::NotAppImage => "Only the AppImage updates itself. Update this copy the way you installed it.".into(),
        Unsupported::Missing(_) => "The AppImage file was moved or deleted while Heronite runs.".into(),
        Unsupported::ReadOnly(dir) => format!("The AppImage's folder ({}) is not writable: download new versions by hand.", dir.display()),
    }
}

fn run(updater: &Entity<Updater>, action: Action, cx: &mut App) {
    updater.update(cx, |u, cx| match action {
        Action::Check => u.check(true, cx),
        Action::Install => u.install(cx),
        Action::Cancel => u.cancel(cx),
        Action::Restart => u.restart(cx),
        Action::OpenPage => {
            let page = u.release().map(|r| r.page_url.clone()).filter(|p| !p.is_empty());
            update::open_page(page.as_deref().unwrap_or(heronite_update::RELEASES_PAGE));
        }
    });
}

/// The release notes, one element per line.
fn notes_list(markdown: &str) -> Div {
    let mut col = div().flex().flex_col().gap(px(4.)).py(px(12.));
    for note in notes::parse(markdown) {
        col = col.child(match note {
            notes::Note::Heading(t) => {
                div().pt(px(6.)).text_size(px(13.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(t).into_any_element()
            }
            notes::Note::Bullet(t) => div()
                .flex()
                .gap(px(8.))
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(theme::text_muted())
                .child(div().flex_none().text_color(theme::text_dim()).child("\u{2022}"))
                .child(div().flex_1().min_w_0().child(t))
                .into_any_element(),
            notes::Note::Text(t) => {
                div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(t).into_any_element()
            }
        });
    }
    col
}

impl SettingsPage {
    pub(super) fn updates_section(&self, cx: &App) -> Option<Div> {
        let updater = update::get(cx)?;
        let u = updater.read(cx);
        let s = status(u, cx);
        let prefs = u.prefs().clone();
        let unsupported = u.unsupported().cloned();
        let release = u.release().cloned();
        let skipped = u.skipped();
        let show_notes = release.is_some() && !matches!(u.phase(), Phase::UpToDate | Phase::Idle | Phase::Checking | Phase::CheckFailed(_));

        let control = s.action.map(|(label, kind, enabled, action)| {
            let updater = updater.clone();
            action_button("upd-action", kind, label, enabled, move |_, _, cx| run(&updater, action, cx))
        });
        let progress = match u.phase() {
            Phase::Downloading(p) => {
                Some(div().pb(px(12.)).child(crate::services::widgets::progress_bar(p.percent(), 4., theme::accent())))
            }
            _ => None,
        };
        let main = div().flex().flex_col().child(row(s.title, Some(&s.detail), div().children(control))).children(progress);

        let auto_check = {
            let desc = "At start and every 6 hours. Only GitHub's release page is contacted.";
            match &unsupported {
                Some(why) => row("Check for updates automatically", Some(&unsupported_text(why)), toggle("upd-auto-check-sw", false, true))
                    .into_any_element(),
                None => {
                    let updater = updater.clone();
                    toggle_row("upd-auto-check", "Check for updates automatically", Some(desc), prefs.auto_check)
                        .on_click(move |_, _, cx| updater.update(cx, |u, cx| u.set_auto_check(!u.prefs().auto_check, cx)))
                        .into_any_element()
                }
            }
        };
        let auto_install = {
            let label = "Install updates automatically";
            let desc = "Download new versions in the background. They are checked against Heronite's signing key and start the next time you open Heronite.";
            if unsupported.is_some() {
                row(label, Some(desc), toggle("upd-auto-install-sw", false, true)).into_any_element()
            } else {
                let updater = updater.clone();
                toggle_row("upd-auto-install", label, Some(desc), prefs.auto_install)
                    .on_click(move |_, _, cx| updater.update(cx, |u, cx| u.set_auto_install(!u.prefs().auto_install, cx)))
                    .into_any_element()
            }
        };

        let mut col = div().flex().flex_col().child(caption("Updates")).child(card([main.into_any_element(), auto_check, auto_install]));
        if let (true, Some(r)) = (show_notes, release) {
            let page = r.page_url.clone();
            let skip_updater = updater.clone();
            let buttons = div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .py(px(12.))
                .border_t_1()
                .border_color(theme::row_separator())
                .child(
                    button_sm("upd-skip", ButtonKind::Ghost, if skipped { "Don't skip this version" } else { "Skip this version" })
                        .on_click(move |_, _, cx| skip_updater.update(cx, |u, cx| u.set_skipped(!skipped, cx))),
                )
                .child(button_sm("upd-page", ButtonKind::Glass, "Release page").on_click(move |_, _, _| update::open_page(&page)));
            let notes = if r.notes.trim().is_empty() {
                div().py(px(12.)).text_size(px(13.)).text_color(theme::text_dim()).child("This release has no notes.")
            } else {
                notes_list(&r.notes)
            };
            col = col.child(caption(format!("What's new in {}", r.version))).child(card_box().child(notes).child(buttons));
        }
        Some(col)
    }
}

/// Console mode rows of the same card.
pub(super) fn console_rows(cx: &App) -> Vec<Row> {
    let Some(updater) = update::get(cx) else { return Vec::new() };
    let u = updater.read(cx);
    let s = status(u, cx);
    let mut rows = vec![Row::header("Updates")];
    rows.push(match s.action {
        Some((label, _, enabled, _)) => Row::action("upd:action", format!("{}: {label}", s.title)).desc(s.detail).enabled(enabled),
        None => Row::info("upd:status", s.title, s.detail),
    });
    let unsupported = u.unsupported().map(unsupported_text);
    let check = Row::toggle("upd:auto-check", "Check for updates automatically", u.prefs().auto_check && unsupported.is_none());
    rows.push(match &unsupported {
        Some(why) => check.desc(why.clone()).enabled(false),
        None => check.desc("At start and every 6 hours"),
    });
    rows.push(
        Row::toggle("upd:auto-install", "Install updates automatically", u.prefs().auto_install && unsupported.is_none())
            .desc("They start the next time you open Heronite")
            .enabled(unsupported.is_none()),
    );
    if let Some(r) = u.release().filter(|_| matches!(u.phase(), Phase::Available | Phase::InstallFailed(_))) {
        let skipped = u.skipped();
        rows.push(
            Row::action("upd:skip", if skipped { "Don't skip this version" } else { "Skip this version" })
                .desc(format!("Heronite {}", r.version)),
        );
    }
    rows
}

/// Handles an outcome of the rows above; `false` when it was not one of them.
pub(super) fn console_apply(outcome: &Outcome, cx: &mut App) -> bool {
    let Some(updater) = update::get(cx) else { return false };
    match outcome {
        Outcome::Activate(id) if id.as_ref() == "upd:action" => {
            let action = status(updater.read(cx), cx).action;
            if let Some((_, _, true, action)) = action {
                run(&updater, action, cx);
            }
        }
        Outcome::Activate(id) if id.as_ref() == "upd:skip" => updater.update(cx, |u, cx| u.set_skipped(!u.skipped(), cx)),
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "upd:auto-check" => updater.update(cx, |u, cx| u.set_auto_check(*on, cx)),
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "upd:auto-install" => {
            updater.update(cx, |u, cx| u.set_auto_install(*on, cx))
        }
        _ => return false,
    }
    true
}

/// Command palette: check, update, restart (shown on this section).
pub(super) fn palette_commands(cx: &App) -> Vec<crate::pages::commands::PageCommand> {
    use crate::icons::Icon;
    use crate::pages::commands::PageCommand;
    let Some(updater) = update::get(cx) else { return Vec::new() };
    let u = updater.read(cx);
    let busy = match u.phase() {
        Phase::Checking => Some("Checking now"),
        Phase::Downloading(_) => Some("An update is downloading"),
        Phase::Ready => Some("An update is installed: restart to use it"),
        _ => None,
    };
    let mut v = vec![PageCommand::new("upd:check", "Check for updates", format!("Heronite {}", u.current()), Icon::Update)
        .keywords("update upgrade new version release")
        .disabled(busy)];
    let version = u.release().map(|r| r.version.clone()).unwrap_or_default();
    match u.phase() {
        Phase::Available | Phase::InstallFailed(_) if u.can_install() => v.push(
            PageCommand::new("upd:install", format!("Update to Heronite {version}"), "Download, verify and install it", Icon::Download)
                .keywords("update upgrade new version install"),
        ),
        Phase::Ready => v.push(
            PageCommand::new("upd:restart", format!("Restart into Heronite {version}"), "The update is installed", Icon::Update)
                .keywords("update upgrade restart relaunch")
                .disabled(crate::system::restart_blocker(cx)),
        ),
        _ => {}
    }
    v
}

/// Runs a command of [`palette_commands`]; `false` for other ids.
pub(super) fn run_palette(id: &str, cx: &mut App) -> bool {
    let Some(updater) = update::get(cx) else { return false };
    let action = match id {
        "upd:check" => Action::Check,
        "upd:install" => Action::Install,
        "upd:restart" => Action::Restart,
        _ => return false,
    };
    run(&updater, action, cx);
    true
}
