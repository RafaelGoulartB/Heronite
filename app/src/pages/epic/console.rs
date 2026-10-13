//! Epic Games in console mode: the account, the library actions and the EOS
//! Overlay as console rows; logging out and uninstalling the overlay ask
//! first. Logging in: the Epic window (typing the password needs a
//! keyboard), or the browser page plus the authorization code typed on the
//! on-screen keyboard.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_epic::eos_overlay;
use heronite_i18n::{tr, trn};

use super::{installed_count, EpicPage};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::osk::{Osk, OskEvent};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;

impl EpicPage {
    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let s = self.svc.read(cx);
        let (account, stats, updates, refreshing, logging_in) =
            (s.account().clone(), s.stats().clone(), s.epic_updates_count(), s.refreshing, s.logging_in);
        let (eos, busy) = (s.eos().clone(), s.busy_of(eos_overlay::APP_NAME).cloned());
        let mut rows = vec![Row::header(tr!("Account"))];
        if account.logged_in {
            rows.push(Row::info("account", account.name.clone(), tr!("Signed in")).icon(Icon::AccountCircle));
            let line = format!(
                "{} \u{b7} {} \u{b7} {}",
                trn!(stats.owned, "{n} game", "{n} games"),
                installed_count(stats.installed),
                trn!(updates, "{n} update", "{n} updates"),
            );
            rows.push(
                Row::action("refresh", if refreshing { tr!("Refreshing the library\u{2026}") } else { tr!("Refresh the library") })
                    .desc(line)
                    .icon(Icon::Refresh)
                    .enabled(!refreshing),
            );
            if updates > 0 {
                rows.push(Row::action("update-all", tr!("Update all ({updates})", updates)).icon(Icon::Update));
            }
            rows.push(
                Row::action("import", tr!("Import an installed game"))
                    .desc(tr!("Add a game already on this PC without downloading it again"))
                    .icon(Icon::FolderOpen),
            );
            rows.push(Row::action("logout", tr!("Log out\u{2026}")).icon(Icon::Logout).danger());
        } else {
            rows.push(
                Row::action(
                    "login-window",
                    if self.window_open {
                        tr!("Waiting for the login window\u{2026}")
                    } else if logging_in {
                        tr!("Logging in\u{2026}")
                    } else {
                        tr!("Log in with Epic Games")
                    },
                )
                .desc(tr!("Opens Epic's login window (typing the password needs a keyboard)"))
                .icon(Icon::AccountCircle)
                .enabled(!self.window_open && !logging_in),
            );
            rows.push(Row::action("browser", tr!("Open the login page in the browser")).icon(Icon::OpenInNew));
            rows.push(
                Row::action("code", tr!("Enter the authorization code"))
                    .desc(tr!("Shown on the login page after signing in"))
                    .icon(Icon::Edit)
                    .enabled(!logging_in),
            );
        }
        if let Some((text, _)) = &self.notice {
            rows.push(Row::info("notice", text.clone(), ""));
        }
        rows.push(Row::header("EOS Overlay"));
        match &busy {
            Some(b) => {
                let state = match b.percent {
                    Some(p) => format!("{} {p:.0}%", b.label),
                    None => format!("{}\u{2026}", b.label),
                };
                rows.push(Row::info("eos-state", "EOS Overlay", state).icon(Icon::Downloading));
                if b.cancel.is_some() {
                    rows.push(Row::action("eos-cancel", tr!("Cancel")).icon(Icon::Close));
                }
            }
            None if eos.installed => {
                rows.push(
                    Row::info(
                        "eos-state",
                        tr!("Installed"),
                        tr!("{version}  \u{b7}  latest {latest}", version = eos.version, latest = eos.latest),
                    )
                    .icon(Icon::Check),
                );
                let outdated = !eos.latest.is_empty() && eos.latest != eos.version && !eos.checking;
                if outdated {
                    rows.push(Row::action("eos-update", tr!("Update")).icon(Icon::Update));
                } else {
                    rows.push(
                        Row::action("eos-check", if eos.checking { tr!("Checking for updates\u{2026}") } else { tr!("Check for updates") })
                            .icon(Icon::Refresh)
                            .enabled(!eos.checking),
                    );
                }
                rows.push(Row::action("eos-remove", tr!("Uninstall\u{2026}")).icon(Icon::FaTrash).danger());
            }
            None => rows.push(
                Row::action("eos-install", tr!("Install the EOS Overlay"))
                    .desc(tr!("Epic's in-game overlay for friends and achievements"))
                    .icon(Icon::Download),
            ),
        }
        rows
    }

    fn confirm(
        &mut self,
        title: &str,
        message: &str,
        yes: &str,
        cx: &mut Context<Self>,
        on_yes: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let rows = vec![Row::action("no", tr!("Cancel")), Row::action("yes", yes.to_string()).danger()];
        let sheet = cx.new(|_| Sheet::new(title.to_string(), Some(message.to_string()), rows));
        let sub = cx.subscribe(&sheet, move |this, _, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            this.layer.close();
            if id.as_ref() == "yes" {
                on_yes(this, cx);
            }
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }

    fn console_activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "refresh" => self.svc.update(cx, |s, cx| s.refresh_library(cx)),
            "update-all" => self.svc.update(cx, |s, cx| s.update_all_epic(cx)),
            "import" => {
                let data_paths = self.ctx.data_paths.clone();
                let picker = cx.new(|cx| super::EpicImportPicker::new(data_paths, window, cx));
                self.open_dialog(picker, window, cx);
            }
            "logout" => {
                self.confirm(tr!("Log out of Epic Games?"), tr!("Your installed games stay on disk."), tr!("Log out"), cx, |this, cx| {
                    this.svc.update(cx, |s, cx| s.logout(cx));
                })
            }
            "login-window" => self.start_window_login(cx),
            "browser" => self.open_browser(cx),
            "code" => {
                let osk = cx.new(|_| Osk::new(tr!("Authorization code"), ""));
                let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
                    if let OskEvent::Done(text) = ev {
                        let text = text.trim().to_string();
                        if !text.is_empty() {
                            this.svc.update(cx, |s, cx| s.login(text, cx));
                        }
                    }
                    this.layer.close();
                    cx.notify();
                });
                self.layer.open(&osk, sub);
            }
            "eos-cancel" => {
                if let Some(cancel) = self.svc.read(cx).busy_of(eos_overlay::APP_NAME).and_then(|b| b.cancel.clone()) {
                    cancel.request();
                }
            }
            "eos-update" | "eos-install" => self.svc.update(cx, |s, cx| s.eos_install(cx)),
            "eos-check" => self.svc.update(cx, |s, cx| s.eos_check_updates(cx)),
            "eos-remove" => self.confirm(
                tr!("Uninstall the EOS Overlay?"),
                tr!("Games that use it lose the overlay until you install it again."),
                tr!("Uninstall"),
                cx,
                |this, cx| {
                    this.svc.update(cx, |s, cx| s.eos_remove(cx));
                },
            ),
            _ => {}
        }
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let error = self.svc.read(cx).last_error.clone().and_then(|e| e.lines().next().map(str::to_owned));
        let list = self.console_list.render("console-epic", &rows, !self.layer.is_open() && self.dialog.is_none());
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(u(76.))
                    .px(u(46.))
                    .child(kit::wordmark("EPIC GAMES"))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap(u(10.))
                    .px(u(46.))
                    .pb(u(16.))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(list.flex_1().min_h_0()))
                    .children(error.map(|e| kit::body_text(e).text_color(crate::theme::danger_text()))),
            )
            .children(self.layer.view())
            .children(self.dialog.clone())
            .into_any_element()
    }
}

impl PadTarget for EpicPage {
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if let Some(view) = self.dialog.clone() {
            let used = crate::console::registry::hook_for(&view).is_some_and(|h| h.pad(pad, window, cx));
            if !used && pad == Pad::B {
                self.dialog = None;
                self.dialog_sub = None;
            }
            cx.notify();
            return true;
        }
        let rows = self.console_rows(cx);
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.console_activate(&id, window, cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        if let Some(view) = &self.dialog {
            return crate::console::registry::hook_for(view).map(|h| h.hints(cx)).unwrap_or_else(|| vec![hint(Btn::B, tr!("Close"))]);
        }
        vec![hint(Btn::A, tr!("Select"))]
    }
}
