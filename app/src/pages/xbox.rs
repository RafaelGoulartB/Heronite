//! The Xbox page of the Stores (Windows): the Microsoft Store and PC Game
//! Pass games the Xbox app installed on this PC (`services::xbox_library`).
//! No login: Heronite reads which of the user's packages are games and adds
//! them to the Collection; the Xbox app keeps installing, updating and
//! signing in to them.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_xbox::XboxGame;

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::{caption_with, columns, empty_row, list_item, status_pill, stores, Page, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::xbox_library::{open_xbox_app, XboxLibrary, XboxLibraryEvent};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card, icon_button_sm, ButtonKind};
use crate::ui::icon;

pub struct XboxPage {
    svc: Entity<XboxLibrary>,
    console_list: crate::console::list::ListNav,
    /// Opening the Xbox app failed.
    error: Option<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for XboxPage {}

const KEYWORDS: &str = "xbox microsoft store game pass gamepass pc windows loja jogos tienda juegos spiele";

/// "Last read 2026-11-03 14:02" for an RFC 3339 date.
fn read_at(at: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&chrono::Local);
    Some(tr!("Last read {when}", when = t.format("%Y-%m-%d %H:%M")))
}

/// A paragraph of a card.
fn paragraph(text: impl Into<SharedString>) -> AnyElement {
    div().py(px(12.)).text_size(px(13.)).line_height(px(19.)).text_color(theme::text_muted()).child(text.into()).into_any_element()
}

impl XboxPage {
    pub fn new(_ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = crate::services::xbox_library(cx);
        stores::watch(cx);
        let subs = vec![
            cx.observe(&svc, |_, _, cx| cx.notify()),
            cx.subscribe(&svc, |_, _, ev: &XboxLibraryEvent, cx| {
                let XboxLibraryEvent::Synced(report) = ev;
                if report.added + report.updated > 0 {
                    cx.emit(PageEvent::LibraryChanged);
                }
            }),
        ];
        svc.update(cx, |s, cx| s.sync_quietly(cx));
        Self { svc, console_list: Default::default(), error: None, _subs: subs }
    }

    /// Reads the games again (the page was shown again).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.sync_quietly(cx));
    }

    fn xbox_app(&self, cx: &App) -> Option<bool> {
        self.svc.read(cx).found.as_ref().map(|f| f.xbox_app)
    }

    fn run_action(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "read" => self.svc.update(cx, |s, cx| s.sync(cx)),
            "open" => {
                let installed = self.xbox_app(cx).unwrap_or(true);
                self.error = open_xbox_app(installed).err().map(|e| tr!("Could not open the Xbox app: {error}", error = e));
            }
            _ => {}
        }
        cx.notify();
    }

    fn show_game(&mut self, game: &XboxGame, cx: &mut Context<Self>) {
        cx.emit(PageEvent::ShowGame(format!("sideload_{}", heronite_xbox::collection::app_name(&game.family))));
    }

    /// The status of the Xbox app: text and colour.
    fn status(&self, cx: &App) -> (String, Rgba) {
        let s = self.svc.read(cx);
        match (&s.found, s.syncing) {
            (None, _) => (tr!("Looking for games\u{2026}").into(), theme::text_muted()),
            (Some(f), _) if f.xbox_app => (tr!("Xbox app found").into(), theme::success()),
            (Some(_), _) => (tr!("Xbox app not found").into(), theme::text_muted()),
        }
    }

    /// "3 games installed · Last read 2026-11-03 14:02".
    fn summary_line(&self, cx: &App) -> String {
        let s = self.svc.read(cx);
        let Some(found) = &s.found else { return tr!("Not read yet").to_string() };
        let mut parts = vec![trn!(found.games.len(), "{n} game installed", "{n} games installed")];
        if let Some(at) = found.last.as_ref().and_then(|l| read_at(&l.at)) {
            parts.push(at);
        }
        parts.join("  \u{b7}  ")
    }

    fn explanation(&self, cx: &App) -> Option<&'static str> {
        match self.xbox_app(cx) {
            Some(false) => Some(tr!(
                "The Xbox app was not found on this PC. Get it from the Microsoft Store to install PC Game Pass games; the games you installed from the Microsoft Store show up here anyway."
            )),
            _ => None,
        }
    }

    /// The outcome of the last read asked for here, or of opening the app.
    fn result_line(&self, cx: &App) -> Option<(String, bool)> {
        if let Some(e) = &self.error {
            return Some((e.clone(), true));
        }
        match self.svc.read(cx).last.as_ref()? {
            Ok(r) if r.added > 0 => Some((trn!(r.added, "Added {n} game to the Collection.", "Added {n} games to the Collection."), false)),
            Ok(_) => Some((tr!("The Collection already has every game installed from the Xbox app.").to_string(), false)),
            Err(e) => Some((e.clone(), true)),
        }
    }

    fn open_label(&self, cx: &App) -> &'static str {
        match self.xbox_app(cx) {
            Some(false) => tr!("Get the Xbox app"),
            _ => tr!("Open the Xbox app"),
        }
    }

    fn render_library(&mut self, cx: &mut Context<Self>) -> Div {
        let (status, color) = self.status(cx);
        let syncing = self.svc.read(cx).syncing;
        let mut rows: Vec<AnyElement> = vec![div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .child(icon(Icon::ShoppingBag, px(44.), theme::accent()))
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
                            .child(tr!("Microsoft Store and PC Game Pass")),
                    )
                    .child(div().flex().child(status_pill(status, color))),
            )
            .child(action_button(
                "xbox-open",
                ButtonKind::Glass,
                self.open_label(cx),
                true,
                cx.listener(|this, _, _, cx| this.run_action("open", cx)),
            ))
            .into_any_element()];
        rows.push(div().py(px(12.)).text_size(px(13.)).text_color(theme::text_muted()).child(self.summary_line(cx)).into_any_element());
        if let Some(text) = self.explanation(cx) {
            rows.push(paragraph(text));
        }
        let read = icon_button_sm(
            "xbox-read",
            ButtonKind::Ghost,
            Icon::Refresh,
            if syncing { tr!("Reading\u{2026}") } else { tr!("Read games now") },
            !syncing,
            cx.listener(|this, _, _, cx| this.run_action("read", cx)),
        );
        let mut column = div().child(caption_with(tr!("Library"), read)).child(card(rows));
        if let Some((text, error)) = self.result_line(cx) {
            column = column.child(
                div()
                    .mt(px(14.))
                    .mx(px(6.))
                    .text_size(px(13.))
                    .text_color(if error { theme::danger_text() } else { theme::text_primary() })
                    .child(text),
            );
        }
        column.child(self.render_games(cx))
    }

    fn render_game(&self, game: &XboxGame, cx: &mut Context<Self>) -> AnyElement {
        let content = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(game.title.clone()))
            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(game.folder.to_string_lossy().into_owned()));
        let shown = game.clone();
        let show = button_sm(SharedString::from(format!("xbox-show-{}", game.family)), ButtonKind::Glass, tr!("Show"))
            .on_click(cx.listener(move |this, _, _, cx| this.show_game(&shown, cx)));
        list_item(Some(icon(Icon::SportsEsports, px(26.), theme::text_muted()).into_any_element()), content, show).into_any_element()
    }

    fn render_games(&mut self, cx: &mut Context<Self>) -> Div {
        let games = self.svc.read(cx).found.as_ref().map(|f| f.games.clone()).unwrap_or_default();
        let rows: Vec<AnyElement> = if games.is_empty() {
            vec![empty_row(tr!(
                "No games yet. Install them in the Xbox app or the Microsoft Store: they come into your Collection by themselves."
            ))
            .into_any_element()]
        } else {
            games.iter().map(|g| self.render_game(g, cx)).collect()
        };
        div().child(caption(tr!("Installed games"))).child(card(rows))
    }

    fn render_about(&self) -> Div {
        div().child(caption(tr!("About"))).child(card([
            paragraph(tr!(
                "Heronite finds the games the Xbox app and the Microsoft Store installed on this PC, bought or from PC Game Pass, and adds them to your Collection by itself. There is no login: the Xbox app keeps installing, updating and signing in to them, and Heronite starts them and counts their play time."
            )),
            paragraph(tr!(
                "A game is found when it is made for Xbox on PC, as almost every game of the Xbox app is. Apps that are not games are left out."
            )),
        ]))
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let (status, _) = self.status(cx);
        let syncing = self.svc.read(cx).syncing;
        let mut rows = vec![
            Row::header(tr!("Library")),
            Row::info("status", "Xbox", status).icon(Icon::ShoppingBag),
            Row::action("read", if syncing { tr!("Reading\u{2026}") } else { tr!("Read games now") })
                .desc(self.summary_line(cx))
                .icon(Icon::Refresh)
                .enabled(!syncing),
            Row::action("open", self.open_label(cx)).icon(Icon::OpenInNew),
        ];
        if let Some(text) = self.explanation(cx) {
            rows.push(Row::info("why", "Xbox", text));
        }
        if let Some((text, _)) = self.result_line(cx) {
            rows.push(Row::info("result", text, ""));
        }
        rows
    }

    fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let list = self.console_list.render("xbox-console", &rows, true);
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
                    .child(stores::console_tabs(Page::Xbox))
                    .child(div().flex_1())
                    .child(kit::clock()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(u(46.))
                    .pb(u(16.))
                    .child(div().w(u(900.)).max_w_full().flex().flex_col().flex_1().min_h_0().child(list.flex_1().min_h_0())),
            )
            .into_any_element()
    }
}

impl Render for XboxPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let wide = stores::content_width(window) >= 1000.;
        let body = columns(wide, [self.render_library(cx), self.render_about()]);
        div()
            .size_full()
            .relative()
            .child(stores::sheet("xbox-page", Page::Xbox, tr!("Your Microsoft Store and PC Game Pass games."), cx, body))
            .into_any_element()
    }
}

impl PadTarget for XboxPage {
    fn on_pad(&mut self, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(next) = stores::switch(Page::Xbox, pad) {
            cx.emit(PageEvent::Navigate(next));
            return true;
        }
        let rows = self.console_rows(cx);
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.run_action(id.as_ref(), cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, _cx: &App) -> Vec<Hint> {
        vec![hint(Btn::A, tr!("Select"))]
    }
}

impl PaletteCommands for XboxPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let syncing = crate::services::xbox_library_started(cx).is_some_and(|s| s.read(cx).syncing);
        vec![
            PageCommand::new(
                "read",
                tr!("Read the Xbox games"),
                tr!("Add the Microsoft Store and PC Game Pass games installed on this PC"),
                Icon::Refresh,
            )
            .keywords(KEYWORDS)
            .disabled(syncing.then(|| tr!("Reading\u{2026}")))
            .stay(),
            PageCommand::new("open", tr!("Open the Xbox app"), tr!("Install PC Game Pass and Microsoft Store games"), Icon::OpenInNew)
                .keywords(KEYWORDS)
                .stay(),
        ]
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, cx);
    }

    fn page_modal(&self) -> bool {
        false
    }
}
