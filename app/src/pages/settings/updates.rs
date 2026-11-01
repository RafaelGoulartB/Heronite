//! Settings → About → Updates: the state of `services::update`, its
//! switches and the notes of the release on offer.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{n_, tr};
use heronite_update::Unsupported;

use super::SettingsPage;
use crate::console::list::{Outcome, Row, Value};
use crate::services::update::{self, Phase, Updater};
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
            detail: checked.map(|a| tr!("Last checked {a}", a)).unwrap_or_else(|| tr!("Not checked yet").into()),
            action: Some((tr!("Check for updates"), ButtonKind::Glass, true, Action::Check)),
        },
        Phase::Checking => Status {
            title: tr!("Checking for updates\u{2026}").into(),
            detail: tr!("Asking GitHub for the latest release").into(),
            action: Some((tr!("Checking\u{2026}"), ButtonKind::Glass, false, Action::Check)),
        },
        Phase::UpToDate => Status {
            title: tr!("Heronite is up to date").into(),
            detail: tr!("Version {current} \u{b7} checked {when}", current, when = checked.unwrap_or_else(|| tr!("just now").into())),
            action: Some((tr!("Check again"), ButtonKind::Glass, true, Action::Check)),
        },
        Phase::Available => {
            let r = u.release();
            let mut detail = tr!("You have {current}", current);
            if let Some(r) = r.filter(|r| !r.date().is_empty()) {
                detail.push_str(&tr!(" \u{b7} released {date}", date = r.date()));
            }
            if let Some(p) = u.package() {
                detail.push_str(&tr!(" \u{b7} {package} package", package = p.label()));
            }
            if let Some(a) = u.download_asset().filter(|a| a.size > 0) {
                detail.push_str(&format!(" \u{b7} {}", update::size_text(a.size)));
            }
            let label = if u.can_install() { tr!("Update") } else { tr!("Download") };
            Status {
                title: tr!("Heronite {version} is available", version),
                detail,
                action: Some((label, ButtonKind::Primary, true, get(Action::Install))),
            }
        }
        Phase::Downloading(p) => Status {
            title: tr!("Downloading Heronite {version}", version),
            detail: update::progress_text(p),
            action: Some((tr!("Cancel"), ButtonKind::Ghost, true, Action::Cancel)),
        },
        Phase::Installing => Status {
            title: tr!("Installing Heronite {version}", version),
            detail: tr!("The package manager is installing it. Enter your password if you are asked.").into(),
            action: Some((tr!("Installing\u{2026}"), ButtonKind::Primary, false, Action::Install)),
        },
        // The Windows setup is downloaded and runs as the app quits.
        Phase::Ready if u.installs_on_quit() => {
            let blocker = crate::system::restart_blocker(cx);
            Status {
                title: tr!("Heronite {version} is ready to install", version),
                detail: match &blocker {
                    Some(why) => tr!("{why}: restart later, or it installs when you quit Heronite.", why),
                    None => tr!("Restart to install it now, or it installs when you quit Heronite.").into(),
                },
                action: Some((tr!("Restart now"), ButtonKind::Primary, blocker.is_none(), Action::Restart)),
            }
        }
        Phase::Ready => {
            let blocker = crate::system::restart_blocker(cx);
            Status {
                title: tr!("Heronite {version} is installed", version),
                detail: match &blocker {
                    Some(why) => tr!("{why}: restart later, or it starts the next time you open Heronite.", why),
                    None => tr!("Restart to use it now, or it starts the next time you open Heronite.").into(),
                },
                action: Some((tr!("Restart now"), ButtonKind::Primary, blocker.is_none(), Action::Restart)),
            }
        }
        Phase::CheckFailed(e) => Status {
            title: tr!("Could not check for updates").into(),
            detail: e.clone(),
            action: Some((tr!("Try again"), ButtonKind::Glass, true, Action::Check)),
        },
        Phase::InstallFailed(e) => Status {
            title: tr!("Could not update to {version}", version),
            detail: e.clone(),
            action: Some((tr!("Try again"), ButtonKind::Primary, true, get(Action::Install))),
        },
    }
}

/// Why "Install updates automatically" is off for packages.
const PACKAGE_NO_AUTO: &str = n_!("Packages ask for your password, so Heronite installs them only when you click Update.");

/// Why this copy cannot install updates by itself, for the switches.
#[cfg(windows)]
fn unsupported_text(why: &Unsupported) -> String {
    match why {
        Unsupported::NotAppImage | Unsupported::Flatpak | Unsupported::Missing(_) => {
            tr!("Only a copy installed with Heronite's setup or unpacked from its portable zip updates itself.").into()
        }
        Unsupported::ReadOnly(dir) => {
            tr!("This copy's folder ({dir}) is not writable: download new versions by hand.", dir = dir.display())
        }
    }
}

/// Why this copy cannot install updates by itself, for the switches.
#[cfg(not(windows))]
fn unsupported_text(why: &Unsupported) -> String {
    match why {
        Unsupported::NotAppImage => tr!("Only the AppImage updates itself. Update this copy the way you installed it.").into(),
        Unsupported::Flatpak => tr!("Flatpak updates this copy: run flatpak update, or update it in your software center.").into(),
        Unsupported::Missing(_) => tr!("The AppImage file was moved or deleted while Heronite runs.").into(),
        Unsupported::ReadOnly(dir) => {
            tr!("The AppImage's folder ({dir}) is not writable: download new versions by hand.", dir = dir.display())
        }
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
        // A package whose install failed: the command that installs it by hand.
        let manual = u.manual_command().filter(|_| matches!(u.phase(), Phase::InstallFailed(_))).map(|cmd| {
            let copy = cmd.clone();
            row(
                tr!("Install it from a terminal"),
                Some(&cmd),
                button_sm("upd-copy", ButtonKind::Glass, tr!("Copy command"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
            )
            .into_any_element()
        });

        let auto_check = {
            let desc = tr!("At start and every 6 hours. Only GitHub's release page is contacted.");
            match &unsupported {
                Some(why) => {
                    row(tr!("Check for updates automatically"), Some(&unsupported_text(why)), toggle("upd-auto-check-sw", false, true))
                        .into_any_element()
                }
                None => {
                    let updater = updater.clone();
                    toggle_row("upd-auto-check", tr!("Check for updates automatically"), Some(desc), prefs.auto_check)
                        .on_click(move |_, _, cx| updater.update(cx, |u, cx| u.set_auto_check(!u.prefs().auto_check, cx)))
                        .into_any_element()
                }
            }
        };
        let auto_install = {
            let label = tr!("Install updates automatically");
            let desc = if u.installs_on_quit() {
                tr!("Download new versions in the background. They are checked against Heronite's signing key and install when you quit Heronite.")
            } else {
                tr!("Download new versions in the background. They are checked against Heronite's signing key and start the next time you open Heronite.")
            };
            if unsupported.is_some() {
                row(label, Some(desc), toggle("upd-auto-install-sw", false, true)).into_any_element()
            } else if !u.can_auto_install() {
                row(label, Some(tr(PACKAGE_NO_AUTO)), toggle("upd-auto-install-sw", false, true)).into_any_element()
            } else {
                let updater = updater.clone();
                toggle_row("upd-auto-install", label, Some(desc), prefs.auto_install)
                    .on_click(move |_, _, cx| updater.update(cx, |u, cx| u.set_auto_install(!u.prefs().auto_install, cx)))
                    .into_any_element()
            }
        };

        let rows = std::iter::once(main.into_any_element()).chain(manual).chain([auto_check, auto_install]);
        let mut col = div().flex().flex_col().child(caption(tr!("Updates"))).child(card(rows));
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
                    button_sm(
                        "upd-skip",
                        ButtonKind::Ghost,
                        if skipped { tr!("Don't skip this version") } else { tr!("Skip this version") },
                    )
                    .on_click(move |_, _, cx| skip_updater.update(cx, |u, cx| u.set_skipped(!skipped, cx))),
                )
                .child(button_sm("upd-page", ButtonKind::Glass, tr!("Release page")).on_click(move |_, _, _| update::open_page(&page)));
            let notes = if r.notes.trim().is_empty() {
                div().py(px(12.)).text_size(px(13.)).text_color(theme::text_dim()).child(tr!("This release has no notes."))
            } else {
                crate::ui::release_notes::list(&r.notes, true)
            };
            col = col.child(caption(tr!("What's new in {version}", version = r.version))).child(card_box().child(notes).child(buttons));
        }
        Some(col)
    }
}

/// Console mode rows of the same card.
pub(super) fn console_rows(cx: &App) -> Vec<Row> {
    let Some(updater) = update::get(cx) else { return Vec::new() };
    let u = updater.read(cx);
    let s = status(u, cx);
    let mut rows = vec![Row::header(tr!("Updates"))];
    rows.push(match s.action {
        Some((label, _, enabled, _)) => Row::action("upd:action", format!("{}: {label}", s.title)).desc(s.detail).enabled(enabled),
        None => Row::info("upd:status", s.title, s.detail),
    });
    let unsupported = u.unsupported().map(unsupported_text);
    let check = Row::toggle("upd:auto-check", tr!("Check for updates automatically"), u.prefs().auto_check && unsupported.is_none());
    rows.push(match &unsupported {
        Some(why) => check.desc(why.clone()).enabled(false),
        None => check.desc(tr!("At start and every 6 hours")),
    });
    let auto = unsupported.is_none() && u.can_auto_install();
    rows.push(
        Row::toggle("upd:auto-install", tr!("Install updates automatically"), u.prefs().auto_install && auto)
            .desc(if u.package().is_some() {
                tr(PACKAGE_NO_AUTO)
            } else if u.installs_on_quit() {
                tr!("They install when you quit Heronite")
            } else {
                tr!("They start the next time you open Heronite")
            })
            .enabled(auto),
    );
    if let Some(cmd) = u.manual_command().filter(|_| matches!(u.phase(), Phase::InstallFailed(_))) {
        rows.push(Row::info("upd:manual", tr!("Install it from a terminal"), cmd));
    }
    if let Some(r) = u.release().filter(|_| matches!(u.phase(), Phase::Available | Phase::InstallFailed(_))) {
        let skipped = u.skipped();
        rows.push(
            Row::action("upd:skip", if skipped { tr!("Don't skip this version") } else { tr!("Skip this version") })
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
        Phase::Checking => Some(tr!("Checking now")),
        Phase::Downloading(_) | Phase::Installing => Some(tr!("An update is installing")),
        Phase::Ready if u.installs_on_quit() => Some(tr!("An update is ready: restart to install it")),
        Phase::Ready => Some(tr!("An update is installed: restart to use it")),
        _ => None,
    };
    let mut v = vec![PageCommand::new("upd:check", tr!("Check for updates"), format!("Heronite {}", u.current()), Icon::Update)
        .keywords("update upgrade new version release atualizar atualizacao atualização nova versão")
        .disabled(busy)];
    let version = u.release().map(|r| r.version.clone()).unwrap_or_default();
    match u.phase() {
        Phase::Available | Phase::InstallFailed(_) if u.can_install() => v.push(
            PageCommand::new(
                "upd:install",
                tr!("Update to Heronite {version}", version),
                tr!("Download, verify and install it"),
                Icon::Download,
            )
            .keywords("update upgrade new version install atualizar atualização nova versão instalar"),
        ),
        Phase::Ready => v.push(
            PageCommand::new(
                "upd:restart",
                tr!("Restart into Heronite {version}", version),
                if u.installs_on_quit() { tr!("The update installs as Heronite restarts") } else { tr!("The update is installed") },
                Icon::Update,
            )
            .keywords("update upgrade restart relaunch atualizar reiniciar")
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
