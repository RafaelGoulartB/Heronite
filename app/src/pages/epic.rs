//! Epic Games page: log in (embedded login window, or the browser + pasted
//! code), account, library refresh and importing an installed game.

use std::process::{Command, Stdio};

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::eos_overlay;
use heronite_epic::user::BROWSER_LOGIN_URL;

use super::{page_body, page_column, PageCtx, PageEvent};
use crate::dialogs::epic_import::EpicImportPicker;
use crate::dialogs::epic_install::{EpicInstallDialog, Mode};
use crate::dialogs::DialogEvent;
use crate::icons::Icon;
use crate::services::widgets::{fmt_percent, progress_bar};
use crate::services::{self, Downloads};
use crate::theme;
use crate::ui::controls::{action_button, button, ButtonKind};
use crate::ui::fields::{meta_text, new_input, panel, paragraph};
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
    std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.exists()))
}

impl EpicPage {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = services::downloads(cx);
        cx.observe(&svc, |_, _, cx| cx.notify()).detach();
        let code = new_input(window, cx, "", "Authorization code or the page's JSON");
        // Installed version and the latest one (Heroic asks when the settings open).
        svc.update(cx, |s, cx| s.eos_reload(true, cx));
        let mut this = Self {
            ctx,
            svc,
            code,
            confirm_logout: false,
            confirm_eos_remove: std::env::var("HERONITE_DEBUG_EPIC_PAGE").as_deref() == Ok("eos-remove"),
            window_open: false,
            notice: None,
            dialog: None,
            dialog_sub: None,
        };
        // Screenshots: `HERONITE_DEBUG_EPIC_PAGE=import|logout`.
        match std::env::var("HERONITE_DEBUG_EPIC_PAGE").as_deref() {
            Ok("import") => {
                let heroic = this.ctx.heroic.clone();
                let picker = cx.new(|cx| EpicImportPicker::new(heroic, window, cx));
                this.open_dialog(picker, window, cx);
            }
            Ok("logout") => this.confirm_logout = true,
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
                        let (heroic, app) = (this.ctx.heroic.clone(), this.ctx.app.clone());
                        let dlg = cx.new(|cx| EpicInstallDialog::new(heroic, app, game, Mode::Import, window, cx));
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
        let mut panel = panel().gap(px(12.)).p(px(18.)).child(paragraph(eos_main_text(eos.installed, working)));
        if eos.installed && !working {
            panel = panel
                .child(paragraph(format!("Current Version: {}", eos.version)))
                .child(paragraph(format!("Latest Version: {}", eos.latest)));
        }
        if let Some(b) = &busy {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w(px(360.))
                    .child(div().text_size(px(13.33)).text_color(white()).child(match b.percent {
                        Some(p) => format!("{} {}", b.label, fmt_percent(p as f64)),
                        None => format!("{}...", b.label),
                    }))
                    .child(progress_bar(b.percent, 5., theme::accent())),
            );
        }
        let mut actions = div().flex().flex_wrap().items_center().gap(px(12.));
        if let Some(b) = busy {
            if let Some(cancel) = b.cancel {
                actions = actions.child(button("eos-cancel", ButtonKind::Tertiary, "Cancel").on_click(move |_, _, _| cancel.request()));
            }
        } else if eos.installed {
            let outdated = !eos.latest.is_empty() && eos.latest != eos.version && !eos.checking;
            if outdated {
                actions = actions.child(button("eos-update", ButtonKind::Primary, "Update").on_click(cx.listener(|this, _, _, cx| {
                    this.svc.update(cx, |s, cx| s.eos_install(cx));
                })));
            } else {
                actions = actions.child(action_button(
                    "eos-check",
                    ButtonKind::Primary,
                    if eos.checking { "Checking for updates..." } else { "Check for updates" },
                    !eos.checking,
                    cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.eos_check_updates(cx))),
                ));
            }
            actions = actions.child(button("eos-remove", ButtonKind::Tertiary, "Uninstall").on_click(cx.listener(|this, _, _, cx| {
                this.confirm_eos_remove = !this.confirm_eos_remove;
                cx.notify();
            })));
        } else {
            actions = actions.child(button("eos-install", ButtonKind::Primary, "Install").on_click(cx.listener(|this, _, _, cx| {
                this.svc.update(cx, |s, cx| s.eos_install(cx));
            })));
        }
        panel = panel.child(actions);
        if self.confirm_eos_remove && eos.installed && !working {
            panel = panel.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .child(div().text_size(px(14.)).text_color(rgb(0xcaf3fd)).child("Are you sure you want to uninstall the EOS Overlay?"))
                    .child(button("eos-remove-no", ButtonKind::Outline, "No").on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_eos_remove = false;
                        cx.notify();
                    })))
                    .child(button("eos-remove-yes", ButtonKind::Tertiary, "Yes").on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_eos_remove = false;
                        this.svc.update(cx, |s, cx| s.eos_remove(cx));
                        cx.notify();
                    }))),
            );
        }
        panel.child(meta_text("Enable or disable the overlay for a game from the game's menu (right-click)."))
    }

    fn start_window_login(&mut self, cx: &mut Context<Self>) {
        let Some(helper) = login_helper() else {
            self.notice = Some((
                "The login window (heronite-epic-login) was not found next to the app. Use the browser method below.".into(),
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
                .background_spawn(async move { Command::new(helper).stdin(Stdio::null()).stderr(Stdio::null()).output() })
                .await;
            this.update(cx, |this, cx| {
                this.window_open = false;
                match out {
                    Ok(o) if o.status.success() => {
                        let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if code.is_empty() {
                            this.notice = Some(("The login window closed without a code.".into(), true));
                        } else {
                            this.svc.update(cx, |s, cx| s.login(code, cx));
                        }
                    }
                    Ok(_) => {} // window closed by the user
                    Err(err) => this.notice = Some((format!("Could not open the login window: {err}").into(), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_browser(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = Command::new("xdg-open").arg(BROWSER_LOGIN_URL).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            self.notice = Some((format!("Could not open the browser: {err}").into(), true));
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

/// Text under "EOS Overlay" (`getMainEosText`).
fn eos_main_text(installed: bool, working: bool) -> &'static str {
    match (installed, working) {
        (true, true) => "The EOS Overlay is being updated...",
        (true, false) => "The EOS Overlay is installed",
        (false, true) => "The EOS Overlay is being installed...",
        (false, false) => "The EOS Overlay is not installed",
    }
}

fn heading(text: &str) -> Div {
    div().font_family(theme::FONT_HEADING).text_size(px(18.)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child(text.to_string())
}

impl Render for EpicPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (account, stats, updates, refreshing, logging_in, last_error) = {
            let s = self.svc.read(cx);
            (s.account().clone(), s.stats().clone(), s.updates_count(), s.refreshing, s.logging_in, s.last_error.clone())
        };

        let mut column = page_column().max_w(px(760.)).gap(px(18.));
        if account.logged_in {
            let line = format!("{} games in your library  -  {} installed  -  {} update{} available", stats.owned, stats.installed, updates, if updates == 1 { "" } else { "s" });
            let mut actions = div().flex().flex_wrap().gap(px(12.)).child(action_button(
                "epic-refresh",
                ButtonKind::Primary,
                if refreshing { "Refreshing..." } else { "Refresh library" },
                !refreshing,
                cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.refresh_library(cx))),
            ));
            if updates > 0 {
                actions = actions.child(button("epic-update-all", ButtonKind::Secondary, format!("Update all ({updates})")).on_click(
                    cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.update_all(cx))),
                ));
            }
            actions = actions.child(button("epic-import", ButtonKind::Secondary, "Import game").on_click(cx.listener(|this, _, window, cx| {
                let heroic = this.ctx.heroic.clone();
                let picker = cx.new(|cx| EpicImportPicker::new(heroic, window, cx));
                this.open_dialog(picker, window, cx);
            })));
            actions = actions.child(button("epic-logout", ButtonKind::Outline, "Log out").on_click(cx.listener(|this, _, _, cx| {
                this.confirm_logout = !this.confirm_logout;
                cx.notify();
            })));
            let mut card = panel()
                .gap(px(14.))
                .p(px(18.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(icon(Icon::AccountCircle, px(46.), theme::accent()))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().font_family(theme::FONT_HEADING).text_size(px(22.)).font_weight(theme::W_BOLD).text_color(white()).child(account.name.clone()))
                                .child(div().text_size(px(13.33)).text_color(theme::success()).child("Logged in to Epic Games")),
                        ),
                )
                .child(paragraph(line))
                .child(actions);
            if self.confirm_logout {
                card = card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(div().text_size(px(14.)).text_color(rgb(0xcaf3fd)).child("Log out of Epic Games? Your installed games stay on disk."))
                        .child(button("epic-logout-no", ButtonKind::Outline, "Cancel").on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_logout = false;
                            cx.notify();
                        })))
                        .child(button("epic-logout-yes", ButtonKind::Tertiary, "Log out").on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_logout = false;
                            this.svc.update(cx, |s, cx| s.logout(cx));
                            cx.notify();
                        }))),
                );
            }
            column = column.child(heading("Epic Games")).child(card);
        } else {
            let mut card = panel().gap(px(14.)).p(px(18.)).child(paragraph(
                "Log in to your Epic Games account to see your library and install your games. Your login is handled by legendary, the same way Heroic does it.",
            ));
            card = card.child(div().flex().flex_wrap().items_center().gap(px(12.)).child(action_button(
                "epic-login-window",
                ButtonKind::Primary,
                if self.window_open { "Waiting for the login window..." } else if logging_in { "Logging in..." } else { "Log in with Epic Games" },
                !self.window_open && !logging_in,
                cx.listener(|this, _, _, cx| this.start_window_login(cx)),
            )));
            column = column.child(heading("Epic Games")).child(card);

            let can_submit = !logging_in && !self.code.read(cx).value().trim().is_empty();
            let browser = panel()
                .gap(px(12.))
                .p(px(18.))
                .child(div().text_size(px(16.)).font_weight(theme::W_BOLD).text_color(rgb(0xcaf3fd)).child("Or log in with your browser"))
                .child(paragraph("1. Open the login page, sign in to Epic Games and copy the authorization code it shows (the value of \"authorizationCode\", or select all and copy the whole page)."))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(12.))
                        .child(div().text_size(px(14.)).text_color(white()).child(BROWSER_LOGIN_URL))
                        .child(button("epic-open-browser", ButtonKind::Secondary, "Open login page in browser").on_click(cx.listener(|this, _, _, cx| this.open_browser(cx))))
                        .child(button("epic-copy-link", ButtonKind::Outline, "Copy link").on_click(cx.listener(|_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(BROWSER_LOGIN_URL.to_string()));
                        }))),
                )
                .child(paragraph("2. Paste it here:"))
                .child(crate::ui::fields::input_box(&self.code, false))
                .child(div().flex().child(action_button(
                    "epic-login-code",
                    ButtonKind::Primary,
                    if logging_in { "Logging in..." } else { "Log in" },
                    can_submit,
                    cx.listener(|this, _, _, cx| this.submit_code(cx)),
                )));
            column = column.child(browser);
        }
        column = column.child(heading("EOS Overlay")).child(self.eos_panel(cx));
        if let Some((text, error)) = self.notice.clone() {
            column = column.child(div().text_size(px(14.)).text_color(if error { theme::danger().into() } else { white() }).child(text));
        }
        if let Some(err) = last_error {
            column = column.child(div().text_size(px(13.33)).text_color(theme::danger()).child(err.lines().next().unwrap_or("").to_string()));
        }
        column = column.child(meta_text("Games are installed, updated and verified with legendary. Files are shared with Heroic and Heronite."));

        div()
            .size_full()
            .relative()
            .child(page_body("page-epic").child(column))
            .children(self.dialog.clone())
    }
}
