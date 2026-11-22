//! The EA page's "Your EA library" card: the EA account (optional), signed
//! in on EA's page in the login window (`heronite-epic-login --store ea`),
//! which brings the games the account owns that are not installed
//! (`services::clients::EaAccount`).

use std::process::{Command, Stdio};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{logic, ClientKind, ClientPage};
use crate::icons::Icon;
use crate::pages::store::{login_helper, no_webview_notice, LOGIN_NO_WEBVIEW};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, icon_button_sm, ButtonKind};
use crate::ui::icon;

impl<K: ClientKind> ClientPage<K> {
    /// The EA account reads or signs in (the EA page only).
    pub(super) fn ea_busy(&self, cx: &App) -> bool {
        K::CLIENT == heronite_data::clients::Client::Ea && self.svc.read(cx).ea.busy()
    }

    /// Signs in in the login window, which prints the EA session's cookies.
    pub(super) fn start_ea_sign_in(&mut self, cx: &mut Context<Self>) {
        if self.ea_window {
            return;
        }
        let Some(helper) = login_helper() else {
            self.ea_notice = Some(tr!("The login window (heronite-epic-login) was not found next to the app.").into());
            cx.notify();
            return;
        };
        self.ea_window = true;
        self.ea_notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    let mut cmd = Command::new(helper);
                    cmd.arg("--store").arg("ea").stdin(Stdio::null());
                    heronite_data::proc::guard(&mut cmd).output()
                })
                .await;
            this.update(cx, |this, cx| {
                this.ea_window = false;
                match out {
                    Ok(o) if o.status.success() => {
                        let output = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        this.svc.update(cx, |s, cx| s.ea_signed_in(output, cx));
                    }
                    Ok(o) if o.status.code() == Some(LOGIN_NO_WEBVIEW) => this.ea_notice = Some(no_webview_notice().to_string()),
                    // The user closed the window.
                    Ok(o) if o.status.code() == Some(4) => {}
                    Ok(o) => {
                        log::warn!("the EA sign-in window stopped ({}): {}", o.status, String::from_utf8_lossy(&o.stderr).trim());
                        this.ea_notice = Some(tr!("The login window stopped unexpectedly. Try again.").into());
                    }
                    Err(err) => this.ea_notice = Some(tr!("Could not open the login window: {err}", err)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_ea_library(&self, cx: &mut Context<Self>) -> Div {
        let svc = self.svc.read(cx);
        let (ea, ea_app_busy) = (&svc.ea, svc.ea_app_leaving());
        let (name, signing_in, reading, owned, last) = (ea.name.clone(), ea.signing_in, ea.reading, ea.owned.clone(), ea.last.clone());
        let busy = self.ea_window || signing_in || ea_app_busy;
        // "Connected", not "signed in": the Account card above is the EA
        // app's own sign-in, a separate one.
        let (title, tone) = match &name {
            Some(name) if !name.is_empty() => (tr!("EA account connected: {name}", name = name), theme::accent()),
            Some(_) => (tr!("Connected to your EA account").to_string(), theme::accent()),
            None if self.ea_window => (tr!("Waiting for the login window").to_string(), theme::text_dim()),
            None if signing_in => (tr!("Connecting\u{2026}").to_string(), theme::text_dim()),
            None => (tr!("EA account not connected").to_string(), theme::text_dim()),
        };
        let hint = match (&name, &owned) {
            (Some(_), Some(last)) => {
                let read = logic::synced_text(&last.at, chrono::Local::now()).unwrap_or_default();
                Some(format!("{} \u{00b7} {read}", trn!(last.games, "{n} game on the account", "{n} games on the account")))
            }
            (Some(_), None) => None,
            (None, _) => Some(
                tr!("Optional, apart from the EA app's own sign-in above. Connect your EA account to also see the games you own that are not installed; they install through the EA app.")
                    .to_string(),
            ),
        };
        let actions = match &name {
            None => div().child(icon_button_sm(
                self.id("ea-sign-in"),
                ButtonKind::Primary,
                Icon::AccountCircle,
                tr!("Connect EA account"),
                !busy,
                cx.listener(|this, _, _, cx| this.start_ea_sign_in(cx)),
            )),
            Some(_) => div()
                .flex()
                .gap(px(6.))
                .child(icon_button_sm(
                    self.id("ea-read"),
                    ButtonKind::Glass,
                    Icon::Refresh,
                    if reading { tr!("Reading\u{2026}") } else { tr!("Read library now") },
                    !reading && !ea_app_busy,
                    cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.read_ea(true, cx))),
                ))
                .child(action_button(
                    self.id("ea-sign-out"),
                    ButtonKind::Ghost,
                    tr!("Disconnect"),
                    !reading,
                    cx.listener(|this, _, _, cx| this.svc.update(cx, |s, cx| s.ea_sign_out(cx))),
                )),
        };
        let line = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .child(icon(Icon::CloudSync, px(36.), tone))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(200.))
                    .gap(px(3.))
                    .child(div().text_size(px(16.)).font_weight(theme::W_500).text_color(theme::text_primary()).child(title))
                    .children(hint.map(|h| div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(h))),
            )
            .child(actions);
        let mut rows = vec![line.into_any_element()];
        // What connecting does, before the user does it.
        if name.is_none() {
            rows.push(
                div()
                    .pb(px(12.))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(theme::text_dim())
                    .child(tr!(
                        "Heronite signs in on EA's own page and only reads the list of your games: it never sees your password, and never buys, installs or changes anything in your account. If EA changes its service, your installed games keep showing as they do now."
                    ))
                    .into_any_element(),
            );
        }
        let message = self.ea_notice.clone().map(|t| (t, true)).or_else(|| last.map(|r| r.map_or_else(|e| (e, true), |t| (t, false))));
        if let Some((text, error)) = message {
            rows.push(
                div()
                    .pb(px(12.))
                    .text_size(px(13.))
                    .text_color(if error { theme::danger_text() } else { theme::text_primary() })
                    .child(text)
                    .into_any_element(),
            );
        }
        div().child(caption(tr!("Your EA library"))).child(card(rows))
    }
}
