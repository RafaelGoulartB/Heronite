//! GOG / Amazon Games in console mode: the account and the library actions as
//! console rows; logging out asks first. Logging in: the store's window
//! (typing the password needs a keyboard), or the browser page plus the code
//! typed on the on-screen keyboard.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{installed_count, StoreKind, StorePage, Then};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::osk::{Osk, OskEvent};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::pages::{stores, PageEvent};

impl<K: StoreKind> StorePage<K> {
    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let store = self.store();
        let state = self.svc.read(cx).store(store).clone();
        let updates = state.updates.len();
        let mut rows = vec![Row::header(tr!("Account"))];
        if state.account.logged_in {
            rows.push(Row::info("account", state.account.name.clone(), tr!("Signed in")).icon(Icon::AccountCircle));
            let line = format!(
                "{} \u{b7} {} \u{b7} {}",
                trn!(state.owned, "{n} game", "{n} games"),
                installed_count(state.installed),
                trn!(updates, "{n} update", "{n} updates"),
            );
            rows.push(
                Row::action("refresh", if state.refreshing { tr!("Refreshing the library\u{2026}") } else { tr!("Refresh the library") })
                    .desc(line)
                    .icon(Icon::Refresh)
                    .enabled(!state.refreshing),
            );
            if updates > 0 {
                rows.push(Row::action("update-all", tr!("Update all ({updates})", updates)).icon(Icon::Update));
            }
            rows.push(
                Row::action("import", tr!("Import an installed game"))
                    .desc(tr!("Add a game already on this PC without downloading it again"))
                    .icon(Icon::FolderOpen),
            );
            if self.amazon_app {
                let busy = self.svc.read(cx).amazon_app_importing();
                rows.push(
                    Row::action("amazon-app", tr!("Add the games of the Amazon Games app"))
                        .desc(tr!("Add the games the Amazon Games app installed on this PC, without downloading them again."))
                        .icon(Icon::FolderOpen)
                        .enabled(!busy),
                );
            }
            rows.push(Row::action("logout", tr!("Log out\u{2026}")).icon(Icon::Logout).danger());
        } else {
            let busy = self.window_open || self.fetching_url || state.logging_in;
            rows.push(
                Row::action(
                    "login-window",
                    if self.window_open {
                        tr!("Waiting for the login window\u{2026}").to_string()
                    } else if state.logging_in {
                        tr!("Logging in\u{2026}").to_string()
                    } else {
                        tr!("Log in with {store}", store = store.name())
                    },
                )
                .desc(tr!("Opens the {store} login window (typing the password needs a keyboard)", store = store.name()))
                .icon(Icon::AccountCircle)
                .enabled(!busy),
            );
            rows.push(Row::action("browser", tr!("Open the login page in the browser")).icon(Icon::OpenInNew).enabled(!self.fetching_url));
            rows.push(
                Row::action("code", tr!("Enter the code"))
                    .desc(tr!("The address of the page you land on after signing in, or its code"))
                    .icon(Icon::Edit)
                    .enabled(!state.logging_in),
            );
        }
        if let Some((text, _)) = &self.notice {
            rows.push(Row::info("notice", text.clone(), ""));
        }
        rows
    }

    fn confirm_logout_sheet(&mut self, cx: &mut Context<Self>) {
        let rows = vec![Row::action("no", tr!("Cancel")), Row::action("yes", tr!("Log out")).danger()];
        let title = tr!("Log out of {store}?", store = self.store().name());
        let sheet = cx.new(|_| Sheet::new(title, Some(tr!("Your installed games stay on disk.").to_string()), rows));
        let sub = cx.subscribe(&sheet, |this, _, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            this.layer.close();
            if id.as_ref() == "yes" {
                let store = this.store();
                this.svc.update(cx, |s, cx| s.store_logout(store, cx));
            }
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }

    fn console_activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store();
        match id {
            "refresh" => self.svc.update(cx, |s, cx| s.store_refresh(store, false, false, cx)),
            "update-all" => self.svc.update(cx, |s, cx| s.store_update_all(store, cx)),
            "import" => self.open_import(window, cx),
            "amazon-app" => self.import_app_games(cx),
            "logout" => self.confirm_logout_sheet(cx),
            "login-window" => self.start_window_login(cx),
            "browser" => self.with_login_url(Then::Browser, cx),
            "code" => {
                let osk = cx.new(|_| Osk::new(tr!("Code or page address"), ""));
                let sub = cx.subscribe(&osk, |this, _, ev: &OskEvent, cx| {
                    if let OskEvent::Done(text) = ev {
                        this.submit_code(text.trim().to_string(), cx);
                    }
                    this.layer.close();
                    cx.notify();
                });
                self.layer.open(&osk, sub);
            }
            _ => {}
        }
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let error = self.svc.read(cx).store(self.store()).last_error.clone().and_then(|e| e.lines().next().map(str::to_owned));
        let list = self.console_list.render(self.id("console"), &rows, !self.layer.is_open() && self.dialog.is_none());
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
                    .gap(u(36.))
                    .child(kit::wordmark(&stores::title().to_uppercase()))
                    .child(stores::console_tabs(self.page()))
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

impl<K: StoreKind> PadTarget for StorePage<K> {
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
        if let Some(next) = stores::switch(self.page(), pad) {
            cx.emit(PageEvent::Navigate(next));
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
