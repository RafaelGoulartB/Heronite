//! Epic Games page: log in (embedded login window, or the browser + pasted
//! code), account, library refresh and importing an installed game.

mod console;
mod palette;

use heronite_i18n::{tr, trc, trn};
use std::process::{Command, Stdio};

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::eos_overlay;
use heronite_epic::user::BROWSER_LOGIN_URL;

use super::{columns, content_width, page_close, sheet, status_pill, stores, Page, PageCtx, PageEvent};
use crate::dialogs::epic_import::EpicImportPicker;
use crate::dialogs::epic_install::{EpicInstallDialog, Mode};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::services::{self, Downloads};
use crate::theme;
use crate::ui::controls::{action_button, button, caption, card, row, ButtonKind};
use crate::ui::fields::{meta_text, new_input};
use crate::ui::icon;

pub struct EpicPage {
    ctx: PageCtx,
    svc: Entity<Downloads>,
    code: Entity<InputState>,
    confirm_logout: bool,
    /// "Are you sure you want to uninstall the EOS Overlay?" is showing.
    confirm_eos_remove: bool,
    /// The embedded login window is open.
    window_open: bool,
    notice: Option<(SharedString, bool)>,
    dialog: Option<AnyView>,
    dialog_sub: Option<Subscription>,
    /// Console mode: focused row and the open sheet / keyboard.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
}

impl EventEmitter<PageEvent> for EpicPage {}

/// `heronite-epic-login` next to this executable, else on `PATH`.
fn login_helper() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "heronite-epic-login.exe" } else { "heronite-epic-login" };
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.exists()))
}

impl EpicPage {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = services::downloads(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let code = new_input(window, cx, "", tr!("Authorization code or the page's JSON"));
        // Installed version and the latest one (Heroic asks when the settings open).
        svc.update(cx, |s, cx| s.eos_reload(true, cx));
        let debug = crate::env::value("HERONITE_DEBUG_EPIC_PAGE");
        let mut this = Self {
            ctx,
            svc,
            code,
            confirm_logout: false,
            confirm_eos_remove: debug.as_deref() == Some("eos-remove"),
            window_open: false,
            notice: None,
            dialog: None,
            dialog_sub: None,
            console_list: Default::default(),
            layer: Default::default(),
        };
        // Screenshots: `HERONITE_DEBUG_EPIC_PAGE=import|logout|eos-remove`.
        match debug.as_deref() {
            Some("import") => {
                let data_paths = this.ctx.data_paths.clone();
                let picker = cx.new(|cx| EpicImportPicker::new(data_paths, window, cx));
                this.open_dialog(picker, window, cx);
            }
            Some("logout") => this.confirm_logout = true,
            _ => {}
        }
        this
    }

    fn open_dialog<V>(&mut self, view: Entity<V>, window: &mut Window, cx: &mut Context<Self>)
    where
        V: Render + EventEmitter<DialogEvent>,
    {
        self.dialog_sub = Some(cx.subscribe_in(&view, window, |this, view, ev: &DialogEvent, window, cx| {
            if matches!(ev, DialogEvent::Close | DialogEvent::ChangedAndClose) {
                this.dialog = None;
                this.dialog_sub = None;
                // The import picker hands over the chosen game.
                if let Ok(picker) = view.clone().into_any().downcast::<EpicImportPicker>() {
                    if let Some(game) = picker.update(cx, |p, _| p.picked.take()) {
                        let (data_paths, app) = (this.ctx.data_paths.clone(), this.ctx.app.clone());
                        let dlg = cx.new(|cx| EpicInstallDialog::new(data_paths, app, game, Mode::Import, window, cx));
                        this.open_dialog(dlg, window, cx);
                    }
                }
            }
            cx.notify();
        }));
        self.dialog = Some(view.into());
        cx.notify();
    }

    /// The EOS Overlay: install / update / uninstall (the per-game switch is in
    /// the game's context menu, like Heroic's game menu).
    fn eos_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let (eos, busy) = {
            let s = self.svc.read(cx);
            (s.eos().clone(), s.busy_of(eos_overlay::APP_NAME).cloned())
        };
        let working = busy.is_some();
        let desc = (eos.installed && !working)
            .then(|| tr!("Current version {version}  \u{b7}  Latest {latest}", version = eos.version, latest = eos.latest));
        let mut actions = div().flex().flex_wrap().items_center().justify_end().gap(px(8.));
        if let Some(b) = &busy {
            if let Some(cancel) = b.cancel.clone() {
                actions = actions.child(button("eos-cancel", ButtonKind::Glass, tr!("Cancel")).on_click(move |_, _, _| cancel.request()));
            }
        } else if eos.installed {
            let outdated = !eos.latest.is_empty() && eos.latest != eos.version && !eos.checking;
            if outdated {
                actions =
                    actions.child(button("eos-update", ButtonKind::Primary, tr!("Update")).on_click(cx.listener(|this, _, _, cx| {
                        this.svc.update(cx, |s, cx| s.eos_install(cx));
                    })));
            } else {
                actions = actions.child(action_button(
                    "eos-check",
                    ButtonKind::Glass,
                    if eos.checking { tr!("Checking for updates...") } else { tr!("Check for updates") },
                    !eos.checking,
                    cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.eos_check_updates(cx))),
                ));
            }
            actions = actions.child(button("eos-remove", ButtonKind::Danger, tr!("Uninstall")).on_click(cx.listener(|this, _, _, cx| {
                this.confirm_eos_remove = !this.confirm_eos_remove;
                cx.notify();
            })));
        } else {
            actions = actions.child(button("eos-install", ButtonKind::Primary, tr!("Install")).on_click(cx.listener(|this, _, _, cx| {
                this.svc.update(cx, |s, cx| s.eos_install(cx));
            })));
        }
        let mut rows = vec![row(eos_main_text(eos.installed, working), desc.as_deref(), actions).into_any_element()];
        if let Some(b) = &busy {
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .py(px(12.))
                    .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(match b.percent {
                        Some(p) => format!("{} {}", b.label, fmt_percent(p as f64)),
                        None => format!("{}...", b.label),
                    }))
                    .child(progress_bar(b.percent, 6., theme::accent()))
                    .into_any_element(),
            );
        }
        if self.confirm_eos_remove && eos.installed && !working {
            rows.push(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .py(px(12.))
                    .child(
                        div().flex_1().min_w(px(240.)).text_size(px(13.)).child(tr!("Are you sure you want to uninstall the EOS Overlay?")),
                    )
                    .child(button("eos-remove-no", ButtonKind::Ghost, tr!("No")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_eos_remove = false;
                        cx.notify();
                    })))
                    .child(button("eos-remove-yes", ButtonKind::Danger, tr!("Yes, uninstall")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_eos_remove = false;
                        this.svc.update(cx, |s, cx| s.eos_remove(cx));
                        cx.notify();
                    })))
                    .into_any_element(),
            );
        }
        div().flex().flex_col().child(card(rows)).child(
            div()
                .mt(px(8.))
                .mx(px(6.))
                .child(meta_text(tr!("Enable or disable the overlay for a game from the game's menu (right-click)."))),
        )
    }

    fn start_window_login(&mut self, cx: &mut Context<Self>) {
        let Some(helper) = login_helper() else {
            self.notice = Some((
                tr!("The login window (heronite-epic-login) was not found next to the app. Use the browser method below.").into(),
                true,
            ));
            cx.notify();
            return;
        };
        self.window_open = true;
        self.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    // Guarded helper: closes with the app instead of being orphaned.
                    let mut cmd = Command::new(helper);
                    cmd.stdin(Stdio::null());
                    heronite_data::proc::guard(&mut cmd).output()
                })
                .await;
            this.update(cx, |this, cx| {
                this.window_open = false;
                match out {
                    Ok(o) if o.status.success() => {
                        let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if code.is_empty() {
                            this.notice = Some((tr!("The login window closed without a code.").into(), true));
                        } else {
                            this.svc.update(cx, |s, cx| s.login(code, cx));
                        }
                    }
                    // 127: the loader could not start it (WebKitGTK 4.1 is missing).
                    Ok(o) if o.status.code() == Some(127) => {
                        this.notice = Some((
                            tr!("The login window needs WebKitGTK 4.1, which is not installed. Use the browser method below.").into(),
                            true,
                        ))
                    }
                    // `CLOSED` of the helper: the user closed the window.
                    Ok(o) if o.status.code() == Some(4) => {}
                    Ok(o) => {
                        log::warn!("the login window stopped ({}): {}", o.status, String::from_utf8_lossy(&o.stderr).trim());
                        this.notice = Some((tr!("The login window stopped unexpectedly. Use the browser method below.").into(), true));
                    }
                    Err(err) => this.notice = Some((tr!("Could not open the login window: {err}", err).into(), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_browser(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = Command::new("xdg-open").arg(BROWSER_LOGIN_URL).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            self.notice = Some((tr!("Could not open the browser: {err}", err).into(), true));
            cx.notify();
        }
    }

    fn submit_code(&mut self, cx: &mut Context<Self>) {
        let text = self.code.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.notice = None;
        self.svc.update(cx, |s, cx| s.login(text, cx));
    }
}

/// "1 installed" / "3 installed" (Portuguese inflects it).
fn installed_count(n: usize) -> String {
    if n == 1 {
        trc!("one", "{n} installed", n)
    } else {
        trc!("other", "{n} installed", n)
    }
}

/// Text under "EOS Overlay" (`getMainEosText`).
fn eos_main_text(installed: bool, working: bool) -> &'static str {
    match (installed, working) {
        (true, true) => tr!("The EOS Overlay is being updated..."),
        (true, false) => tr!("The EOS Overlay is installed"),
        (false, true) => tr!("The EOS Overlay is being installed..."),
        (false, false) => tr!("The EOS Overlay is not installed"),
    }
}

impl Render for EpicPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let (account, stats, updates, refreshing, logging_in, last_error) = {
            let s = self.svc.read(cx);
            (s.account().clone(), s.stats().clone(), s.epic_updates_count(), s.refreshing, s.logging_in, s.last_error.clone())
        };

        let mut column = div().flex().flex_col().w_full();
        if account.logged_in {
            let mut rows: Vec<AnyElement> = Vec::new();
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .py(px(14.))
                    .child(icon(Icon::AccountCircle, px(44.), theme::accent()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(4.))
                            .child(
                                div()
                                    .font_family(theme::font_heading())
                                    .text_size(px(20.))
                                    .font_weight(theme::W_BOLD)
                                    .text_color(theme::text_primary())
                                    .child(account.name.clone()),
                            )
                            .child(div().flex().child(status_pill(tr!("Logged in to Epic Games"), theme::success()))),
                    )
                    .child(button("epic-logout", ButtonKind::Glass, tr!("Log out")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_logout = !this.confirm_logout;
                        cx.notify();
                    })))
                    .into_any_element(),
            );
            if self.confirm_logout {
                rows.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .py(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(240.))
                                .text_size(px(13.))
                                .child(tr!("Log out of Epic Games? Your installed games stay on disk.")),
                        )
                        .child(button("epic-logout-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_logout = false;
                            cx.notify();
                        })))
                        .child(button("epic-logout-yes", ButtonKind::Danger, tr!("Log out")).on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_logout = false;
                            this.svc.update(cx, |s, cx| s.logout(cx));
                            cx.notify();
                        })))
                        .into_any_element(),
                );
            }
            let line = format!(
                "{}  \u{b7}  {}  \u{b7}  {}",
                trn!(stats.owned, "{n} game in your library", "{n} games in your library"),
                installed_count(stats.installed),
                trn!(updates, "{n} update available", "{n} updates available"),
            );
            let mut lib_actions = div().flex().items_center().gap(px(8.));
            if updates > 0 {
                lib_actions = lib_actions.child(
                    button("epic-update-all", ButtonKind::Glass, tr!("Update all ({updates})", updates))
                        .on_click(cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.update_all_epic(cx)))),
                );
            }
            lib_actions = lib_actions.child(action_button(
                "epic-refresh",
                ButtonKind::Primary,
                if refreshing { tr!("Refreshing...") } else { tr!("Refresh library") },
                !refreshing,
                cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.refresh_library(cx))),
            ));
            rows.push(row(tr!("Library"), Some(&line), lib_actions).into_any_element());
            rows.push(
                row(
                    tr!("Import an installed game"),
                    Some(tr!("Add a game that is already on this PC without downloading it again.")),
                    button("epic-import", ButtonKind::Glass, tr!("Import game")).on_click(cx.listener(|this, _, window, cx| {
                        let data_paths = this.ctx.data_paths.clone();
                        let picker = cx.new(|cx| EpicImportPicker::new(data_paths, window, cx));
                        this.open_dialog(picker, window, cx);
                    })),
                )
                .into_any_element(),
            );
            column = column.child(caption(tr!("Account"))).child(card(rows));
        } else {
            let login = row(
                tr!("Log in with Epic Games"),
                Some(tr!("Your login is handled by legendary.")),
                action_button(
                    "epic-login-window",
                    ButtonKind::Primary,
                    if self.window_open {
                        tr!("Waiting for the login window...")
                    } else if logging_in {
                        tr!("Logging in...")
                    } else if account.expired {
                        tr!("Log in again")
                    } else {
                        tr!("Log in")
                    },
                    !self.window_open && !logging_in,
                    cx.listener(|this, _, _, cx| this.start_window_login(cx)),
                ),
            );
            column = column.child(caption(tr!("Account"))).child(card([login.into_any_element()]));

            let can_submit = !logging_in && !self.code.read(cx).value().trim().is_empty();
            let step1 = row(
                tr!("1. Open the login page"),
                Some(tr!("Sign in to Epic Games and copy the authorization code it shows (the value of \"authorizationCode\", or select all and copy the whole page).")),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(button("epic-open-browser", ButtonKind::Glass, tr!("Open in browser")).on_click(cx.listener(|this, _, _, cx| this.open_browser(cx))))
                    .child(button("epic-copy-link", ButtonKind::Ghost, tr!("Copy link")).on_click(cx.listener(|_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(BROWSER_LOGIN_URL.to_string()));
                    }))),
            );
            let step2 =
                div().flex().flex_col().gap(px(10.)).py(px(12.)).child(div().text_size(px(14.)).child(tr!("2. Paste the code"))).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().flex_1().min_w_0().child(crate::ui::fields::input_box(&self.code, false)))
                        .child(action_button(
                            "epic-login-code",
                            ButtonKind::Primary,
                            if logging_in { tr!("Logging in...") } else { tr!("Log in") },
                            can_submit,
                            cx.listener(|this, _, _, cx| this.submit_code(cx)),
                        )),
                );
            column = column
                .child(caption(tr!("Or log in with your browser")))
                .child(card([step1.into_any_element(), step2.into_any_element()]))
                .child(div().mt(px(8.)).mx(px(6.)).child(meta_text(BROWSER_LOGIN_URL)));
        }
        if let Some((text, error)) = self.notice.clone() {
            column = column.child(
                div()
                    .mt(px(14.))
                    .mx(px(6.))
                    .text_size(px(13.))
                    .text_color(if error { theme::danger_text() } else { theme::text_primary() })
                    .child(text),
            );
        }
        if let Some(err) = last_error {
            column = column.child(
                div()
                    .mt(px(10.))
                    .mx(px(6.))
                    .text_size(px(12.))
                    .text_color(theme::danger_text())
                    .child(err.lines().next().unwrap_or("").to_string()),
            );
        }
        let eos = div()
            .child(caption("EOS Overlay"))
            .child(self.eos_panel(cx))
            .child(div().mt(px(18.)).mx(px(6.)).child(meta_text(tr!("Games are installed, updated and verified with legendary."))));
        // Wide: the account on the left, the EOS Overlay on the right.
        let wide = content_width(window) >= 1000.;

        let close = page_close(cx);
        div()
            .size_full()
            .relative()
            .child(sheet(
                "page-epic",
                stores::title(),
                Some(tr!("Your account, library and the EOS Overlay.").into()),
                None,
                1320.,
                window,
                close,
                div().flex().flex_col().child(stores::tabs(Page::Epic, cx)).child(columns(wide, [column, eos])),
            ))
            .children(self.dialog.clone())
            .into_any_element()
    }
}
