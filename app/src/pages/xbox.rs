//! The Xbox page of the Stores (Windows), laid out as the two import
//! methods ([`super::methods`]):
//!
//! 1. Local, no login: the Microsoft Store and PC Game Pass games the Xbox
//!    app installed on this PC (`services::xbox_library`). Heronite reads
//!    which of the user's packages are games and adds them to the
//!    Collection; the Xbox app keeps installing, updating and signing in to
//!    them.
//! 2. Online, optional: the Xbox account (Windows), which brings the play
//!    time Xbox counted, the last play, achievements, art and the games not
//!    installed here.

use std::process::{Command, Stdio};

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_xbox::XboxGame;

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::methods::{self, Method, Tone};
use super::{empty_row, list_item, stores, Page, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::xbox_library::{open_xbox_app, XboxLibrary, XboxLibraryEvent};
use crate::theme;
use crate::ui::controls::{action_button, button_sm, caption, card, icon_button_sm, ButtonKind};
use crate::ui::fields::{input_box, new_input};
use crate::ui::icon;

pub struct XboxPage {
    svc: Entity<XboxLibrary>,
    console_list: crate::console::list::ListNav,
    /// Opening the Xbox app failed.
    error: Option<String>,
    /// The address the account's sign-in ended on, pasted.
    code: Entity<InputState>,
    /// The login window is open.
    window_open: bool,
    /// A notice about the sign-in window.
    notice: Option<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for XboxPage {}

const KEYWORDS: &str = "xbox microsoft store game pass gamepass pc windows loja jogos tienda juegos spiele";

/// "Last read 2026-11-03 14:02" for an RFC 3339 date.
fn read_at(at: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&chrono::Local);
    Some(tr!("Last read {when}", when = t.format("%Y-%m-%d %H:%M")))
}

impl XboxPage {
    pub fn new(_ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let code = new_input(window, cx, "", tr!("Paste the address of the page you land on"));
        Self { svc, console_list: Default::default(), error: None, code, window_open: false, notice: None, _subs: subs }
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
            "account-read" => self.svc.update(cx, |s, cx| s.sync_account(true, cx)),
            "sign-in" => self.start_window_sign_in(cx),
            "sign-out" => self.svc.update(cx, |s, cx| s.sign_out(cx)),
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

    /// The status of the Xbox app.
    fn status(&self, cx: &App) -> (String, Tone) {
        match &self.svc.read(cx).found {
            None => (tr!("Looking for games\u{2026}").into(), Tone::Accent),
            Some(f) if f.xbox_app => (tr!("Xbox app found").into(), Tone::Good),
            Some(_) => (tr!("Xbox app not found").into(), Tone::Neutral),
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

    /// Method 1: the games the Xbox app installed on this PC.
    fn render_local(&mut self, cx: &mut Context<Self>) -> Div {
        let open = action_button(
            "xbox-open",
            ButtonKind::Glass,
            self.open_label(cx),
            true,
            cx.listener(|this, _, _, cx| this.run_action("open", cx)),
        );
        let (status, tone) = self.status(cx);
        methods::method_card(Method {
            eyebrow: methods::local_eyebrow(),
            icon: Icon::Computer,
            title: tr!("From the Xbox app on this PC").into(),
            badge: methods::no_login(),
            status: Some((status.into(), tone)),
            facts: vec![
                methods::brings(tr!("Finds the games the Xbox app and the Microsoft Store installed here, bought or from PC Game Pass")),
                methods::brings(tr!("The Xbox app keeps installing, updating and signing in to them")),
                methods::needs(tr!("Only the games installed on this PC; apps that are not games are left out")),
            ],
            notice: self.explanation(cx).map(|text| (text.to_string(), Tone::Neutral)),
            body: Vec::new(),
            footer: Vec::new(),
            actions: vec![open.into_any_element()],
            message: self.error.clone().map(|e| (e, true)),
        })
    }

    /// Signs in in the login window (`heronite-epic-login --store xbox`),
    /// which prints the address the sign-in ends on.
    fn start_window_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(helper) = super::store::login_helper() else {
            self.notice =
                Some(tr!("The login window (heronite-epic-login) was not found next to the app. Use the browser method below.").into());
            cx.notify();
            return;
        };
        let Some(url) = self.svc.update(cx, |s, cx| s.begin_sign_in(cx)) else { return };
        self.window_open = true;
        self.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    let mut cmd = Command::new(helper);
                    cmd.arg("--store").arg("xbox").arg("--url").arg(url).stdin(Stdio::null());
                    heronite_data::proc::guard(&mut cmd).output()
                })
                .await;
            this.update(cx, |this, cx| {
                this.window_open = false;
                match out {
                    Ok(o) if o.status.success() => {
                        let address = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        this.svc.update(cx, |s, cx| s.finish_sign_in(address, cx));
                    }
                    Ok(o) if o.status.code() == Some(super::store::LOGIN_NO_WEBVIEW) => {
                        this.notice = Some(super::store::no_webview_notice().to_string());
                    }
                    // The user closed the window.
                    Ok(o) if o.status.code() == Some(4) => {}
                    Ok(o) => {
                        log::warn!("the Xbox sign-in window stopped ({}): {}", o.status, String::from_utf8_lossy(&o.stderr).trim());
                        this.notice = Some(tr!("The login window stopped unexpectedly. Use the browser method below.").into());
                    }
                    Err(err) => this.notice = Some(tr!("Could not open the login window: {err}", err)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_sign_in_in_browser(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.svc.update(cx, |s, cx| s.begin_sign_in(cx)) else { return };
        if let Err(err) = heronite_platform::shell::open(&url) {
            self.notice = Some(tr!("Could not open the browser: {err}", err));
        }
        cx.notify();
    }

    fn submit_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.code.read(cx).value().trim().to_string();
        if !text.is_empty() {
            self.notice = None;
            // The code inside is used once: it does not stay on screen.
            self.code.update(cx, |s, cx| s.set_value("", window, cx));
            self.svc.update(cx, |s, cx| s.finish_sign_in(text, cx));
        }
    }

    /// Method 2: the Xbox account. Builds without one (other systems) say
    /// it is for Windows.
    fn render_account(&mut self, cx: &mut Context<Self>) -> Div {
        let s = self.svc.read(cx);
        let (available, gamertag, syncing, signing_in, last) =
            (s.account.available(), s.account.gamertag.clone(), s.account.syncing, s.account.signing_in, s.account.last.clone());
        let busy = self.window_open || signing_in;
        let status = match &gamertag {
            _ if !available => (tr!("Windows only").into(), Tone::Neutral),
            Some(name) if !name.is_empty() => (tr!("Signed in as {name}", name = name).into(), Tone::Good),
            Some(_) => (tr!("Signed in to Xbox").into(), Tone::Good),
            None if self.window_open => (tr!("Waiting for the login window").into(), Tone::Accent),
            None if signing_in => (tr!("Signing in\u{2026}").into(), Tone::Accent),
            None => (tr!("Not signed in").into(), Tone::Neutral),
        };
        let mut actions = Vec::new();
        let mut footer = Vec::new();
        if !available {
        } else if gamertag.is_some() {
            // Not while the account is read: a renewal under way would save it again.
            actions.push(
                action_button(
                    "xbox-sign-out",
                    ButtonKind::Glass,
                    tr!("Sign out"),
                    !syncing,
                    cx.listener(|this, _, _, cx| this.run_action("sign-out", cx)),
                )
                .into_any_element(),
            );
            actions.push(
                icon_button_sm(
                    "xbox-account-read",
                    ButtonKind::Ghost,
                    Icon::Refresh,
                    if syncing { tr!("Reading\u{2026}") } else { tr!("Read account now") },
                    !syncing,
                    cx.listener(|this, _, _, cx| this.run_action("account-read", cx)),
                )
                .into_any_element(),
            );
        } else {
            actions.push(
                action_button(
                    "xbox-sign-in",
                    ButtonKind::Primary,
                    if self.window_open {
                        tr!("Waiting for the login window...")
                    } else if signing_in {
                        tr!("Signing in\u{2026}")
                    } else {
                        tr!("Sign in")
                    },
                    !busy,
                    cx.listener(|this, _, _, cx| this.run_action("sign-in", cx)),
                )
                .into_any_element(),
            );
            footer.push(self.render_browser_sign_in(busy, cx));
        }
        let notice = match (available, gamertag.is_some()) {
            (false, _) => Some((tr!("The Xbox account can be added on Windows.").to_string(), Tone::Neutral)),
            (true, false) => Some((tr!("Opens Microsoft's sign-in page. Heronite never sees your password.").to_string(), Tone::Accent)),
            (true, true) => None,
        };
        let message = self.notice.clone().map(|n| (n, true)).or(last.map(|l| match l {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        }));
        methods::method_card(Method {
            eyebrow: methods::online_eyebrow(),
            icon: Icon::CloudSync,
            title: tr!("From your Xbox account").into(),
            badge: (tr!("Sign in with Microsoft").into(), Tone::Accent),
            status: Some(status),
            facts: vec![
                methods::brings(tr!("The play time Xbox counted, also outside Heronite and on console")),
                methods::brings(tr!("Achievements, art and when you last played")),
                methods::brings(tr!("The PC games you played that are not installed here")),
                methods::needs(tr!("One sign-in with your Microsoft account")),
            ],
            notice,
            body: Vec::new(),
            footer,
            actions,
            message,
        })
    }

    /// The sign-in through the browser, for when the login window cannot
    /// open: open the page, then paste the address it ends on.
    fn render_browser_sign_in(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let can_submit = !busy && !self.code.read(cx).value().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .pt(px(14.))
            .border_t_1()
            .border_color(theme::row_separator())
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(tr!("Or sign in with your browser")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(theme::text_muted())
                            .child(tr!("Sign in, then copy the address of the blank page you land on (it contains code=...).")),
                    )
                    .child(action_button(
                        "xbox-sign-in-browser",
                        ButtonKind::Glass,
                        tr!("Open in browser"),
                        !busy,
                        cx.listener(|this, _, _, cx| this.open_sign_in_in_browser(cx)),
                    )),
            )
            .child(div().flex().items_center().gap(px(10.)).child(div().flex_1().min_w_0().child(input_box(&self.code, false))).child(
                action_button(
                    "xbox-sign-in-code",
                    ButtonKind::Primary,
                    tr!("Sign in"),
                    can_submit,
                    cx.listener(|this, _, window, cx| this.submit_address(window, cx)),
                ),
            ))
            .into_any_element()
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

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let (status, _) = self.status(cx);
        let syncing = self.svc.read(cx).syncing;
        let mut rows = vec![
            Row::header(tr!("Your library")),
            Row::action("read", if syncing { tr!("Reading\u{2026}") } else { tr!("Read games now") })
                .desc(self.summary_line(cx))
                .icon(Icon::Refresh)
                .enabled(!syncing),
            Row::header(format!("{} \u{b7} {}", methods::local_eyebrow(), tr!("No login needed"))),
            Row::info("status", tr!("From the Xbox app on this PC"), status).icon(Icon::Computer),
            Row::action("open", self.open_label(cx)).icon(Icon::OpenInNew),
        ];
        if let Some(text) = self.explanation(cx) {
            rows.push(Row::info("why", "Xbox", text));
        }
        if let Some((text, _)) = self.result_line(cx) {
            rows.push(Row::info("result", text, ""));
        }
        let account = &self.svc.read(cx).account;
        if account.available() {
            rows.push(Row::header(format!("{} \u{b7} {}", methods::online_eyebrow(), tr!("Sign in with Microsoft"))));
            match &account.gamertag {
                Some(name) => {
                    rows.push(Row::info("account", tr!("Signed in to Xbox"), name.clone()).icon(Icon::AccountCircle));
                    rows.push(
                        Row::action("account-read", if account.syncing { tr!("Reading\u{2026}") } else { tr!("Read account now") })
                            .icon(Icon::Refresh)
                            .enabled(!account.syncing),
                    );
                    rows.push(Row::action("sign-out", tr!("Sign out")).icon(Icon::Logout).enabled(!account.syncing));
                }
                None => rows.push(Row::action("sign-in", tr!("Sign in")).icon(Icon::AccountCircle).enabled(!self.window_open)),
            }
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
        let syncing = self.svc.read(cx).syncing;
        let read = icon_button_sm(
            "xbox-read",
            ButtonKind::Glass,
            Icon::Refresh,
            if syncing { tr!("Reading\u{2026}") } else { tr!("Read games now") },
            !syncing,
            cx.listener(|this, _, _, cx| this.run_action("read", cx)),
        );
        let result = self.result_line(cx).filter(|_| self.error.is_none());
        let strip = methods::library_strip(self.summary_line(cx), result, read);
        let cards = methods::methods(wide, self.render_local(cx), self.render_account(cx));
        let note = methods::footnote(tr!("A game is found when it is made for Xbox on PC, as almost every game of the Xbox app is."));
        let body = methods::page(strip, cards, [self.render_games(cx).into_any_element(), note.into_any_element()]);
        div()
            .size_full()
            .relative()
            .child(stores::sheet(
                "xbox-page",
                Page::Xbox,
                tr!("Two ways to bring your Microsoft Store and PC Game Pass games in. Your account is optional."),
                cx,
                body,
            ))
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
