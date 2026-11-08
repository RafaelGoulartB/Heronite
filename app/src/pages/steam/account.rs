//! The Steam account card of the Steam page: the optional sign-in with a QR
//! code, the signed-in account, and the note on the library card that offers
//! it when Steam on this PC cannot be read.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::super::{caption_with, status_pill, tag_pill};
use super::SteamPage;
use crate::console::list::Row;
use crate::icons::Icon;
use crate::services::steam_library::QrState;
use crate::theme;
use crate::ui::controls::{action_button, icon_button_sm, ButtonKind};
use crate::ui::icon;
use crate::ui::qr::qr_code;

/// Pixels per module of the QR code (whole, so the modules stay sharp).
const QR_MODULE: f32 = 5.;

/// A paragraph of a card.
fn paragraph(text: impl Into<SharedString>) -> AnyElement {
    div().py(px(12.)).text_size(px(13.)).line_height(px(19.)).text_color(theme::text_muted()).child(text.into()).into_any_element()
}

/// The heading row of the card: icon, title, pill and a button.
fn heading(lead: Icon, title: impl Into<SharedString>, pill: Option<Div>, button: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .py(px(14.))
        .child(icon(lead, px(44.), theme::accent()))
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
                        .truncate()
                        .child(title.into()),
                )
                .children(pill.map(|p| div().flex().child(p))),
        )
        .child(button)
        .into_any_element()
}

/// A numbered step next to the QR code.
fn step(n: usize, text: &'static str) -> Div {
    div()
        .flex()
        .gap(px(10.))
        .text_size(px(13.))
        .line_height(px(19.))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .mt(px(-1.))
                .rounded_full()
                .bg(theme::ink_a(0.08))
                .text_size(px(12.))
                .font_weight(theme::W_BOLD)
                .child(n.to_string()),
        )
        .child(div().flex_1().min_w_0().child(text))
}

impl SteamPage {
    /// The account card (right column).
    pub(super) fn render_account(&mut self, cx: &mut Context<Self>) -> Div {
        let s = self.svc.read(cx);
        let (name, qr, busy, syncing, last, stuck) =
            (s.account.name.clone(), s.account.qr.clone(), s.account.busy, s.syncing, s.account.last.clone(), s.client_stuck());
        let mut column = div();
        if let Some(name) = name {
            let read = icon_button_sm(
                "steam-account-read",
                ButtonKind::Ghost,
                Icon::Refresh,
                if syncing { tr!("Reading\u{2026}") } else { tr!("Read library now") },
                !syncing,
                cx.listener(|this, _, _, cx| this.run_action("read", cx)),
            );
            // Not while a read runs: renewing the tokens would save them again.
            let sign_out = action_button(
                "steam-sign-out",
                ButtonKind::Glass,
                tr!("Sign out"),
                !syncing && !busy,
                cx.listener(|this, _, _, cx| this.run_action("sign-out", cx)),
            );
            column = column.child(caption_with(tr!("Steam account"), read)).child(crate::ui::controls::card([
                heading(Icon::AccountCircle, name, Some(status_pill(tr!("Signed in to Steam"), theme::success())), sign_out),
                paragraph(tr!(
                    "Your library is read from this account whenever Steam on this PC cannot be read, also while it is closed. Games you hid in Steam and games shared with you through Family Sharing only come from Steam on this PC."
                )),
            ]));
        } else {
            let rows = match &qr {
                Some(qr) => self.render_qr(qr, cx),
                None => self.render_signed_out(busy, stuck, cx),
            };
            column = column.child(caption_with(tr!("Steam account"), tag_pill(tr!("Optional")))).child(crate::ui::controls::card(rows));
        }
        if let Some(last) = last {
            let (text, error) = match last {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            column = column.child(
                div()
                    .mt(px(14.))
                    .mx(px(6.))
                    .text_size(px(13.))
                    .text_color(if error { theme::danger_text() } else { theme::text_primary() })
                    .child(text),
            );
        }
        column.mb(px(24.))
    }

    fn render_signed_out(&self, busy: bool, stuck: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let button = action_button(
            "steam-sign-in",
            ButtonKind::Primary,
            if busy { tr!("Starting\u{2026}") } else { tr!("Show QR code") },
            !busy,
            cx.listener(|this, _, _, cx| this.run_action("sign-in", cx)),
        );
        let mut rows = vec![heading(Icon::QrCode, tr!("Sign in with the Steam app"), None, button)];
        if stuck {
            rows.push(
                div()
                    .my(px(4.))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .bg(theme::accent_tint())
                    .border_1()
                    .border_color(theme::accent_tint_border())
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .child(tr!(
                        "Steam on this PC cannot be read right now. Sign in here and Heronite reads your library from your Steam account instead."
                    ))
                    .into_any_element(),
            );
        }
        rows.push(paragraph(tr!(
            "Optional. Scan a QR code with the Steam app on your phone, and Heronite reads your library, with its play time, from your Steam account whenever Steam on this PC cannot be read. Heronite never sees your password."
        )));
        rows
    }

    fn render_qr(&self, qr: &QrState, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let new_code = action_button(
            "steam-sign-in-again",
            ButtonKind::Primary,
            tr!("New code"),
            true,
            cx.listener(|this, _, _, cx| this.run_action("sign-in", cx)),
        );
        let cancel = action_button(
            "steam-sign-in-cancel",
            ButtonKind::Ghost,
            tr!("Cancel"),
            true,
            cx.listener(|this, _, _, cx| this.run_action("cancel-sign-in", cx)),
        );
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
            .child(div().text_size(px(13.)).text_color(color).child(status))
            .child(div().flex().gap(px(8.)).when(qr.ended, |d| d.child(new_code)).child(cancel));
        vec![div().flex().flex_wrap().items_start().gap(px(20.)).py(px(16.)).children(code).child(steps).into_any_element()]
    }

    /// The library card's note when Steam on this PC cannot be read: the
    /// account brings the library meanwhile, or can be signed in.
    pub(super) fn render_fallback(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.svc.read(cx);
        if !s.client_stuck() {
            return None;
        }
        if s.account.signed_in() {
            return Some(
                div()
                    .py(px(12.))
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(tr!("Meanwhile your library is read from your Steam account."))
                    .into_any_element(),
            );
        }
        // The QR code shows in the account card already.
        if s.account.qr.is_some() {
            return None;
        }
        let busy = s.account.busy;
        Some(
            crate::ui::controls::row(
                tr!("Cannot fix it now?"),
                Some(tr!("Sign in to your Steam account with a QR code and your library comes from it instead.")),
                action_button(
                    "steam-fallback-sign-in",
                    ButtonKind::Glass,
                    tr!("Use a QR code"),
                    !busy,
                    cx.listener(|this, _, _, cx| this.run_action("sign-in", cx)),
                ),
            )
            .into_any_element(),
        )
    }

    /// The account's rows in console mode. The QR code needs the desktop
    /// view; signed in, the account can be read and signed out here.
    pub(super) fn account_console_rows(&self, cx: &App) -> Vec<Row> {
        let s = self.svc.read(cx);
        let mut rows = vec![Row::header(tr!("Steam account"))];
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
