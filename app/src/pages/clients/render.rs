//! Desktop layout of the client pages: the client, its account, the games
//! it installed and how it works.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_clients::ClientGame;
use heronite_i18n::{tr, trn};

use super::{logic, ClientKind, ClientPage};
use crate::icons::Icon;
use crate::pages::{caption_with, columns, content_width, empty_row, list_item, page_close, sheet, status_pill, stores};
use crate::services::clients::{Activity, ClientState};
use crate::services::widgets::{busy_bar, progress_bar};
use crate::theme;
use crate::ui::controls::{action_button, button, button_sm, caption, card, icon_button_sm, row, ButtonKind};
use crate::ui::icon;

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// The click handler of a button that runs `$action` of
/// [`ClientPage::run_action`].
macro_rules! act {
    ($cx:expr, $action:expr) => {{
        let action: &'static str = $action;
        $cx.listener(move |this, _, window, cx| this.run_action(action, window, cx))
    }};
}

/// A paragraph of a card.
fn paragraph(text: impl Into<SharedString>) -> AnyElement {
    div().py(px(12.)).text_size(px(13.)).line_height(px(19.)).text_color(theme::text_muted()).child(text.into()).into_any_element()
}

impl<K: ClientKind> ClientPage<K> {
    /// Id of an element, unique per client.
    pub(super) fn id(&self, name: &str) -> SharedString {
        SharedString::from(format!("{}-{name}", K::CLIENT.id()))
    }

    fn render_client(&mut self, st: &ClientState, cx: &mut Context<Self>) -> Div {
        let client = K::CLIENT;
        let pill = logic::pill(st);
        let main = logic::main_action(client, st);
        let main_kind = if main.id == "cancel" { ButtonKind::Glass } else { ButtonKind::Primary };
        let mut rows: Vec<AnyElement> = vec![div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .child(icon(K::PAGE.icon(), px(44.), theme::accent()))
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
                            .child(client.name()),
                    )
                    .child(div().flex().child(status_pill(pill.text(), pill.color()))),
            )
            .child(action_button(self.id("main"), main_kind, main.label.clone(), main.enabled, act!(cx, main.id)))
            .into_any_element()];
        if let Some(why) = main.why {
            rows.push(div().pb(px(12.)).text_size(px(12.5)).text_color(theme::text_muted()).child(why).into_any_element());
        }
        if let Activity::SettingUp(progress) = &st.activity {
            let bar = match logic::step_percent(progress) {
                Some(p) => progress_bar(Some(p), 6., theme::accent()).into_any_element(),
                None => busy_bar(self.id("busy"), 6.),
            };
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .py(px(12.))
                    .child(div().text_size(px(13.)).child(logic::step_text(client, progress, st.cancelling())))
                    .child(bar)
                    .into_any_element(),
            );
        } else if !st.installed() {
            rows.push(paragraph(tr!(
                "Heronite downloads the official installer and installs {client} in a Wine prefix of its own, with your default Proton. It takes about 5 minutes.",
                client = client.name()
            )));
        }
        let idle = !st.busy();
        match st.prefix().filter(|_| st.installed()) {
            Some(prefix) => {
                let shown = logic::tilde(prefix, &home());
                rows.push(
                    row(
                        tr!("Wine prefix"),
                        Some(&shown),
                        icon_button_sm(
                            self.id("folder"),
                            ButtonKind::Glass,
                            Icon::FolderOpen,
                            tr!("Open folder"),
                            true,
                            act!(cx, "folder"),
                        ),
                    )
                    .into_any_element(),
                );
                rows.push(
                    row(
                        tr!("Wine"),
                        Some(self.wine.as_deref().unwrap_or("\u{2026}")),
                        icon_button_sm(
                            self.id("settings"),
                            ButtonKind::Glass,
                            Icon::Settings,
                            tr!("Wine settings"),
                            idle,
                            act!(cx, "settings"),
                        ),
                    )
                    .into_any_element(),
                );
                rows.push(self.render_forget(st, cx));
            }
            None if !st.busy() && st.status.is_some() => rows.push(
                row(
                    tr!("Already installed?"),
                    Some(&tr!("Use {client} from a Wine prefix you already have.", client = client.name())),
                    action_button(self.id("adopt"), ButtonKind::Glass, tr!("Use an existing install"), true, act!(cx, "adopt")),
                )
                .into_any_element(),
            ),
            None => {}
        }
        let mut column = div().child(caption(tr!("Client"))).child(card(rows));
        if let Some(notice) = &st.notice {
            column = column.child(
                div()
                    .mt(px(14.))
                    .mx(px(6.))
                    .text_size(px(13.))
                    .text_color(if notice.error { theme::danger_text() } else { theme::text_primary() })
                    .child(notice.text.clone()),
            );
        }
        column
    }

    /// "Remove from Heronite", and its question once asked.
    fn render_forget(&mut self, st: &ClientState, cx: &mut Context<Self>) -> AnyElement {
        let client = K::CLIENT;
        if !self.confirm_forget {
            // The label lines up with the rows above (the button's padding is 12).
            return div()
                .flex()
                .py(px(10.))
                .ml(px(-12.))
                .child(
                    button_sm(self.id("forget"), ButtonKind::Ghost, tr!("Remove from Heronite"))
                        .when(!st.busy(), |b| b.on_click(act!(cx, "forget"))),
                )
                .into_any_element();
        }
        let path = st.prefix().map(|p| logic::tilde(p, &home())).unwrap_or_default();
        let mut text = div().flex().flex_col().gap(px(4.)).flex_1().min_w(px(240.)).text_size(px(13.)).child(tr!(
            "Remove {client} from Heronite? Its Wine prefix and the games installed in it stay on disk at {path}.",
            client = client.name(),
            path = path
        ));
        if st.in_collection > 0 {
            text = text.child(div().text_color(theme::text_muted()).child(trn!(
                st.in_collection,
                "Its {n} game leaves your Collection.",
                "Its {n} games leave your Collection."
            )));
        }
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .py(px(12.))
            .child(text)
            .child(button(self.id("forget-no"), ButtonKind::Ghost, tr!("Cancel")).on_click(act!(cx, "forget-no")))
            .child(button(self.id("forget-yes"), ButtonKind::Danger, tr!("Remove")).on_click(act!(cx, "forget-yes")))
            .into_any_element()
    }

    fn render_account(&self, st: &ClientState) -> Div {
        let (title, hint) = logic::account_lines(K::CLIENT, &st.account());
        let signed_in = matches!(st.account(), heronite_clients::Account::SignedIn(_));
        let line = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .child(icon(Icon::AccountCircle, px(36.), if signed_in { theme::accent() } else { theme::text_dim() }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(div().text_size(px(16.)).font_weight(theme::W_500).text_color(theme::text_primary()).child(title))
                    .children(hint.map(|h| div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(h))),
            );
        div().child(caption(tr!("Account"))).child(card([line.into_any_element()]))
    }

    fn render_game(&self, st: &ClientState, game: &ClientGame, cx: &mut Context<Self>) -> AnyElement {
        let in_collection = st.in_collection_game(&heronite_clients::collection::app_name(K::CLIENT, &game.id));
        let folder = game.folder.as_ref().map(|f| logic::tilde(f, &home()));
        let content = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(game.title.clone()))
            .children(folder.map(|f| div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(f)));
        let id = game.id.clone();
        let show = in_collection.then(|| {
            button_sm(self.id(&format!("show-{}", game.id)), ButtonKind::Glass, tr!("Show"))
                .on_click(cx.listener(move |this, _, _, cx| this.show_game(&id, cx)))
        });
        list_item(Some(icon(Icon::SportsEsports, px(26.), theme::text_muted()).into_any_element()), content, div().children(show))
            .into_any_element()
    }

    fn render_games(&mut self, st: &ClientState, cx: &mut Context<Self>) -> Div {
        let client = K::CLIENT;
        let games = st.status.as_ref().map(|s| s.games.clone()).unwrap_or_default();
        let refresh = icon_button_sm(
            self.id("sync"),
            ButtonKind::Ghost,
            Icon::Refresh,
            if st.syncing { tr!("Refreshing\u{2026}") } else { tr!("Refresh") },
            !st.syncing && !st.busy(),
            act!(cx, "sync"),
        );
        let mut rows: Vec<AnyElement> = Vec::new();
        if games.is_empty() {
            rows.push(
                empty_row(tr!(
                    "No games yet. Install them in {client}: they come into your Collection by themselves.",
                    client = client.name()
                ))
                .into_any_element(),
            );
        } else {
            rows.extend(games.iter().map(|g| self.render_game(st, g, cx)));
        }
        if let Some(text) = st.last.as_ref().and_then(|l| logic::synced_text(&l.at, chrono::Local::now())) {
            rows.push(div().py(px(10.)).text_size(px(12.)).text_color(theme::text_dim()).child(text).into_any_element());
        }
        div().child(caption_with(tr!("Installed games"), refresh)).child(card(rows))
    }

    fn render_about(&self) -> Div {
        let client = K::CLIENT;
        div().child(caption(tr!("About"))).child(card([
            paragraph(tr!(
                "{client} runs under Wine in a prefix of its own. You sign in, install and update games in it; Heronite starts them through it and counts the play time for the game, not for {client}.",
                client = client.name()
            )),
            paragraph(logic::client_note(client)),
        ]))
    }
}

impl<K: ClientKind> Render for ClientPage<K> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        let st = self.svc.read(cx).state(K::CLIENT).clone();
        // The client and its account on the left, what it installed and how
        // it works on the right.
        let mut left = self.render_client(&st, cx);
        let mut right = div();
        if st.installed() {
            left = left.child(self.render_account(&st));
            right = right.child(self.render_games(&st, cx));
        }
        right = right.child(self.render_about());
        let wide = content_width(window) >= 1000.;
        let close = page_close(cx);
        let body = div().flex().flex_col().child(stores::tabs(K::PAGE, cx)).child(columns(wide, [left, right]));
        div()
            .size_full()
            .relative()
            .child(sheet(self.id("page"), stores::title(), Some(tr!("Your account and library.").into()), None, 1320., window, close, body))
            .children(self.dialog.clone())
            .into_any_element()
    }
}
