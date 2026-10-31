//! Desktop layout of the GOG / Amazon Games pages.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{installed_count, StoreKind, StorePage, Then};
use crate::icons::Icon;
use crate::pages::{columns, status_pill, stores};
use crate::services::downloads::{Store, StoreState};
use crate::theme;
use crate::ui::controls::{action_button, button, caption, card, row, ButtonKind};
use crate::ui::fields::{input_box, meta_text};
use crate::ui::icon;

impl<K: StoreKind> StorePage<K> {
    /// Id of an element, unique per store.
    pub(super) fn id(&self, name: &str) -> SharedString {
        SharedString::from(format!("{}-{name}", self.store().runner()))
    }

    fn render_account(&mut self, state: &StoreState, cx: &mut Context<Self>) -> Div {
        let store = self.store();
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
                                .child(state.account.name.clone()),
                        )
                        .child(div().flex().child(status_pill(tr!("Logged in to {store}", store = store.name()), theme::success()))),
                )
                .child(button(self.id("logout"), ButtonKind::Glass, tr!("Log out")).on_click(cx.listener(|this, _, _, cx| {
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
                            .child(tr!("Log out of {store}? Your installed games stay on disk.", store = store.name())),
                    )
                    .child(button(self.id("logout-no"), ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_logout = false;
                        cx.notify();
                    })))
                    .child(button(self.id("logout-yes"), ButtonKind::Danger, tr!("Log out")).on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_logout = false;
                        let store = this.store();
                        this.svc.update(cx, |s, cx| s.store_logout(store, cx));
                        cx.notify();
                    })))
                    .into_any_element(),
            );
        }
        let updates = state.updates.len();
        let line = format!(
            "{}  \u{b7}  {}  \u{b7}  {}",
            trn!(state.owned, "{n} game in your library", "{n} games in your library"),
            installed_count(state.installed),
            trn!(updates, "{n} update available", "{n} updates available"),
        );
        let mut lib_actions = div().flex().items_center().gap(px(8.));
        if updates > 0 {
            lib_actions =
                lib_actions.child(button(self.id("update-all"), ButtonKind::Glass, tr!("Update all ({updates})", updates)).on_click(
                    cx.listener(|this, _, _, cx| {
                        let store = this.store();
                        this.svc.update(cx, |s, cx| s.store_update_all(store, cx));
                    }),
                ));
        }
        lib_actions = lib_actions.child(action_button(
            self.id("refresh"),
            ButtonKind::Primary,
            if state.refreshing { tr!("Refreshing...") } else { tr!("Refresh library") },
            !state.refreshing,
            cx.listener(|this, _, _, cx| {
                let store = this.store();
                this.svc.update(cx, |s, cx| s.store_refresh(store, false, false, cx));
            }),
        ));
        rows.push(row(tr!("Library"), Some(&line), lib_actions).into_any_element());
        rows.push(
            row(
                tr!("Import an installed game"),
                Some(tr!("Add a game that is already on this PC without downloading it again.")),
                button(self.id("import"), ButtonKind::Glass, tr!("Import game"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx))),
            )
            .into_any_element(),
        );
        if self.amazon_app {
            let busy = self.svc.read(cx).amazon_app_importing();
            rows.push(
                row(
                    tr!("Games of the Amazon Games app"),
                    Some(tr!("Add the games the Amazon Games app installed on this PC, without downloading them again.")),
                    action_button(
                        self.id("amazon-app"),
                        ButtonKind::Glass,
                        if busy { tr!("Adding...") } else { tr!("Add games") },
                        !busy,
                        cx.listener(|this, _, _, cx| this.import_app_games(cx)),
                    ),
                )
                .into_any_element(),
            );
        }
        div().child(caption(tr!("Account"))).child(card(rows))
    }

    fn render_login(&mut self, state: &StoreState, cx: &mut Context<Self>) -> Div {
        let store = self.store();
        let busy = self.window_open || self.fetching_url || state.logging_in;
        let login = row(
            tr!("Log in with {store}", store = store.name()),
            Some(&tr!("Opens a window where you sign in to {store}.", store = store.name())),
            action_button(
                self.id("login-window"),
                ButtonKind::Primary,
                if self.window_open {
                    tr!("Waiting for the login window...")
                } else if state.logging_in {
                    tr!("Logging in...")
                } else if state.account.expired {
                    tr!("Log in again")
                } else {
                    tr!("Log in")
                },
                !busy,
                cx.listener(|this, _, _, cx| this.start_window_login(cx)),
            ),
        );
        let mut column = div().child(caption(tr!("Account"))).child(card([login.into_any_element()]));

        let hint = match store {
            Store::Gog => tr!("Sign in to GOG, then copy the address of the page you land on (it contains code=...)."),
            Store::Amazon => tr!("Sign in to Amazon, then copy the address of the amazon.com page you land on."),
        };
        let step1 = row(
            tr!("1. Open the login page"),
            Some(hint),
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(action_button(
                    self.id("open-browser"),
                    ButtonKind::Glass,
                    if self.fetching_url { tr!("Loading...") } else { tr!("Open in browser") },
                    !self.fetching_url,
                    cx.listener(|this, _, _, cx| this.with_login_url(Then::Browser, cx)),
                ))
                .child(
                    button(self.id("copy-link"), ButtonKind::Ghost, tr!("Copy link"))
                        .on_click(cx.listener(|this, _, _, cx| this.with_login_url(Then::CopyLink, cx))),
                ),
        );
        let can_submit = !state.logging_in && self.can_submit() && !self.code.read(cx).value().trim().is_empty();
        let step2 = div().flex().flex_col().gap(px(10.)).py(px(12.)).child(div().text_size(px(14.)).child(tr!("2. Paste the code"))).child(
            div().flex().items_center().gap(px(10.)).child(div().flex_1().min_w_0().child(input_box(&self.code, false))).child(
                action_button(
                    self.id("login-code"),
                    ButtonKind::Primary,
                    if state.logging_in { tr!("Logging in...") } else { tr!("Log in") },
                    can_submit,
                    cx.listener(|this, _, _, cx| {
                        let text = this.code.read(cx).value().to_string();
                        this.submit_code(text, cx);
                    }),
                ),
            ),
        );
        column =
            column.child(caption(tr!("Or log in with your browser"))).child(card([step1.into_any_element(), step2.into_any_element()]));
        let shown = match store {
            Store::Gog => Some(heronite_gog::user::LOGIN_URL.to_string()),
            Store::Amazon => self.login_url.clone(),
        };
        column.children(shown.map(|url| div().mt(px(8.)).mx(px(6.)).child(meta_text(url))))
    }

    /// The "About" card of the right column.
    fn render_about(&self) -> Div {
        let text = match self.store() {
            Store::Gog => {
                tr!("Games are downloaded and verified with gogdl. After you log in, your whole GOG library shows in your Collection, and you install games from there.")
            }
            Store::Amazon => {
                tr!("Games are downloaded and verified with nile. After you log in, your whole Amazon Games library, including the Prime Gaming games you claimed, shows in your Collection, and you install games from there.")
            }
        };
        div().child(caption(tr!("About"))).child(card([div()
            .py(px(12.))
            .text_size(px(13.))
            .line_height(px(19.))
            .text_color(theme::text_muted())
            .child(text)
            .into_any_element()]))
    }
}

impl<K: StoreKind> Render for StorePage<K> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let state = self.svc.read(cx).store(self.store()).clone();
        let mut column = if state.account.logged_in { self.render_account(&state, cx) } else { self.render_login(&state, cx) };
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
        if let Some(err) = state.last_error {
            column = column.child(
                div()
                    .mt(px(10.))
                    .mx(px(6.))
                    .text_size(px(12.))
                    .text_color(theme::danger_text())
                    .child(err.lines().next().unwrap_or("").to_string()),
            );
        }
        // Wide: the account on the left, the About card on the right.
        let wide = stores::content_width(window) >= 1000.;
        let body = columns(wide, [column, self.render_about()]);
        div()
            .size_full()
            .relative()
            .child(stores::sheet(self.id("page"), self.page(), tr!("Your account and library."), cx, body))
            .children(self.dialog.clone())
            .into_any_element()
    }
}
