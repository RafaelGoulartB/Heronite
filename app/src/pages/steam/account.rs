//! Method 2 of the Steam page: the Steam account, signed in with a QR code
//! (optional), which brings the library whenever the client on this PC
//! cannot be read.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::methods::{self, Method, Tone};
use super::SteamPage;
use crate::console::list::Row;
use crate::icons::Icon;
use crate::services::steam_library::QrState;
use crate::theme;
use crate::ui::controls::{action_button, ButtonKind};
use crate::ui::qr::qr_code;

/// Pixels per module of the QR code (whole, so the modules stay sharp).
const QR_MODULE: f32 = 5.;

/// A numbered step next to the QR code.
fn step(n: usize, text: &'static str) -> Div {
    div()
        .flex()
        .gap(px(10.))
        .text_size(px(13.))
        .line_height(px(19.))
        .text_color(theme::text_primary())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .bg(theme::ink_a(0.08))
                .text_size(px(12.))
                .font_weight(theme::W_BOLD)
                .child(n.to_string()),
        )
        .child(div().flex_1().min_w_0().child(text))
}

impl SteamPage {
    /// Method 2: the Steam account.
    pub(super) fn render_account(&mut self, cx: &mut Context<Self>) -> Div {
        let s = self.svc.read(cx);
        let (name, qr, busy, syncing, last, stuck) =
            (s.account.name.clone(), s.account.qr.clone(), s.account.busy, s.syncing, s.account.last.clone(), s.client_stuck());
        let status = match (&name, &qr) {
            (Some(name), _) => (tr!("Signed in as {name}", name = name).into(), Tone::Good),
            (None, Some(q)) if q.ended => (tr!("Code expired").into(), Tone::Warning),
            (None, Some(q)) if q.scanned => (tr!("Approve on your phone").into(), Tone::Accent),
            (None, Some(_)) => (tr!("Waiting for the scan").into(), Tone::Accent),
            (None, None) => (tr!("Not signed in").into(), Tone::Neutral),
        };
        let notice = match (name.is_some(), stuck) {
            (false, true) if qr.is_none() => Some((
                tr!("Method 1 cannot read your library right now. Sign in here and your games come from your account instead.").to_string(),
                Tone::Accent,
            )),
            (true, true) => {
                Some((tr!("Method 1 cannot read your library right now: it comes from your account meanwhile.").to_string(), Tone::Good))
            }
            (true, false) => {
                Some((tr!("Used whenever method 1 cannot read your library, also while Steam is closed.").to_string(), Tone::Neutral))
            }
            _ => None,
        };
        let mut actions = Vec::new();
        let mut body = Vec::new();
        if name.is_some() {
            // Not while a read runs: renewing the tokens would save them again.
            actions.push(
                action_button(
                    "steam-sign-out",
                    ButtonKind::Glass,
                    tr!("Sign out"),
                    !syncing && !busy,
                    cx.listener(|this, _, _, cx| this.run_action("sign-out", cx)),
                )
                .into_any_element(),
            );
        } else if let Some(qr) = &qr {
            body.push(Self::render_qr(qr));
            if qr.ended {
                actions.push(
                    action_button(
                        "steam-sign-in-again",
                        ButtonKind::Primary,
                        tr!("New code"),
                        true,
                        cx.listener(|this, _, _, cx| this.run_action("sign-in", cx)),
                    )
                    .into_any_element(),
                );
            }
            actions.push(
                action_button(
                    "steam-sign-in-cancel",
                    ButtonKind::Ghost,
                    tr!("Cancel"),
                    true,
                    cx.listener(|this, _, _, cx| this.run_action("cancel-sign-in", cx)),
                )
                .into_any_element(),
            );
        } else {
            actions.push(
                action_button(
                    "steam-sign-in",
                    ButtonKind::Primary,
                    if busy { tr!("Starting\u{2026}") } else { tr!("Show QR code") },
                    !busy,
                    cx.listener(|this, _, _, cx| this.run_action("sign-in", cx)),
                )
                .into_any_element(),
            );
        }
        methods::method_card(Method {
            eyebrow: methods::online_eyebrow(),
            icon: Icon::CloudSync,
            title: tr!("From your Steam account").into(),
            badge: (tr!("Sign in with a QR code").into(), Tone::Accent),
            status: Some(status),
            facts: vec![
                methods::brings(tr!("Works with Steam closed or blocked on this PC")),
                methods::brings(tr!("Brings play time; private profiles work too")),
                methods::needs(tr!("One sign-in with the Steam app on your phone. Heronite never sees your password")),
                methods::needs(tr!("Does not know Family Sharing games or the games you hid in Steam")),
            ],
            notice,
            body,
            footer: Vec::new(),
            actions,
            message: last.map(|l| match l {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            }),
        })
    }

    /// The QR code and how to scan it.
    fn render_qr(qr: &QrState) -> AnyElement {
        let (status, color) = if qr.ended {
            (tr!("This code expired. Show a new one to try again."), theme::warning())
        } else if qr.scanned {
            (tr!("Scanned. Approve the sign-in in the Steam app."), theme::accent())
        } else {
            (tr!("Waiting for the code to be scanned\u{2026}"), theme::text_muted())
        };
        let code = qr_code(qr.url(), QR_MODULE).map(|c| c.when(qr.ended || qr.scanned, |c| c.opacity(0.3)));
        let steps = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(200.))
            .gap(px(10.))
            .child(step(1, tr!("Open the Steam app on your phone and sign in to the account to read.")))
            .child(step(2, tr!("Open Steam Guard (the shield) and tap the QR code button.")))
            .child(step(3, tr!("Scan this code, then approve the sign-in.")))
            .child(div().text_size(px(13.)).text_color(color).child(status));
        div().flex().flex_wrap().items_start().gap(px(20.)).children(code).child(steps).into_any_element()
    }

    /// The account's rows in console mode. The QR code needs the desktop
    /// view; signed in, the account can be signed out here.
    pub(super) fn account_console_rows(&self, cx: &App) -> Vec<Row> {
        let s = self.svc.read(cx);
        let mut rows = vec![Row::header(format!("{} \u{b7} {}", methods::online_eyebrow(), tr!("Sign in with a QR code")))];
        match &s.account.name {
            Some(name) => {
                rows.push(Row::info("account", tr!("Signed in to Steam"), name.clone()).icon(Icon::AccountCircle));
                rows.push(Row::action("sign-out", tr!("Sign out")).icon(Icon::Logout).enabled(!s.syncing && !s.account.busy));
            }
            None => rows.push(
                Row::info("account", tr!("Not signed in"), tr!("Sign in with a QR code from the Steam page outside console mode."))
                    .icon(Icon::QrCode),
            ),
        }
        rows
    }
}
