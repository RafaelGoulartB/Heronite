//! The Steam page of the Stores: the library read from the Steam client
//! (`services::steam_library`). No login: the client is signed in already;
//! the page says whether it can be read, opens or restarts Steam when that
//! is what is missing, and reads it on request.
//!
//! The Steam account card ([`account`]) offers the optional sign-in with a
//! QR code, which reads the library from the account whenever the client
//! cannot be read; the library card points at it when the client is stuck.

mod account;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::steam_owned::SteamSource;
use heronite_i18n::{tr, trn};
use heronite_steam::ClientApiFlag;

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::{columns, status_pill, stores, Page, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::steam_library::{SteamLibrary, SteamLibraryEvent, SteamState};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, ButtonKind};
use crate::ui::icon;

pub struct SteamPage {
    svc: Entity<SteamLibrary>,
    console_list: crate::console::list::ListNav,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for SteamPage {}

const KEYWORDS: &str = "steam valve library import games biblioteca importar jogos";

/// "Read on 13 Oct, 14:02" for an RFC 3339 date.
fn read_at(updated_at: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(updated_at).ok()?.with_timezone(&chrono::Local);
    Some(tr!("Last read {when}", when = t.format("%Y-%m-%d %H:%M")))
}

impl SteamPage {
    pub fn new(_ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = crate::services::steam_library(cx);
        stores::watch(cx);
        let subs = vec![
            cx.observe(&svc, |_, _, cx| cx.notify()),
            cx.subscribe(&svc, |_, _, ev: &SteamLibraryEvent, cx| {
                let SteamLibraryEvent::Imported(report) = ev;
                if report.changed() {
                    cx.emit(PageEvent::LibraryChanged);
                }
            }),
        ];
        svc.update(cx, |s, cx| s.check(cx));
        Self { svc, console_list: Default::default(), _subs: subs }
    }

    /// Checks again whether Steam can be read (the page was shown again).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.check(cx));
    }

    fn read_now(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.sync(cx));
    }

    /// The status of the client: text, colour, whether it can be read.
    fn status(&self, cx: &App) -> (String, Rgba, bool) {
        match self.svc.read(cx).state() {
            SteamState::Checking => (tr!("Checking Steam\u{2026}").into(), theme::text_muted(), false),
            SteamState::Connected => (tr!("Steam is running").into(), theme::success(), true),
            // A read asked for while Steam does not answer fails at once: not "running".
            SteamState::Reading => (tr!("Reading\u{2026}").into(), theme::accent(), true),
            SteamState::Closed => (tr!("Steam is closed").into(), theme::text_muted(), false),
            SteamState::Opening => (tr!("Steam is starting").into(), theme::accent(), false),
            SteamState::Restarting => (tr!("Restarting Steam\u{2026}").into(), theme::accent(), false),
            SteamState::NotAnswering => (tr!("Steam is not answering").into(), theme::warning(), false),
            SteamState::NeedsRestart => (tr!("Restart Steam once").into(), theme::warning(), false),
            SteamState::Blocked => (tr!("Blocked by another tool").into(), theme::warning(), false),
            SteamState::PortTaken => (tr!("Port in use").into(), theme::warning(), false),
            SteamState::Failed => (tr!("Could not connect").into(), theme::danger_text(), false),
            SteamState::NotInstalled => (tr!("Steam not found").into(), theme::text_muted(), false),
        }
    }

    fn explanation(&self, cx: &App) -> Option<String> {
        let s = self.svc.read(cx);
        if s.state() == SteamState::PortTaken {
            return s.taken.as_ref().map(heronite_steam::port_taken_message);
        }
        let text = match s.state() {
            // Said only when the flag file is there: another tool may hold its place.
            SteamState::Closed => Some(match s.flag {
                Some(ClientApiFlag::On) => {
                    tr!("Open Steam and Heronite reads your library by itself. The access it needs is turned on already: no restart.")
                }
                Some(ClientApiFlag::Blocked) => tr!(
                    "Open Steam and Heronite reads your library by itself, if Steam lets it: another tool on this PC (Millennium, for example) decides whether Steam opens the access Heronite needs."
                ),
                _ => tr!("Open Steam and Heronite reads your library by itself."),
            }),
            SteamState::Blocked => Some(heronite_steam::blocked_message()),
            SteamState::Opening => Some(tr!("Steam is starting. If it asks, sign in: your library is read as soon as Steam is ready.")),
            SteamState::NotAnswering => Some(tr!(
                "Steam is open but does not let Heronite read its library yet. If it is updating or asking you to sign in, finish that: the library is read by itself. Otherwise restart Steam once. Read library now shows what went wrong."
            )),
            SteamState::NeedsRestart => Some(tr!(
                "Steam was open before Heronite could turn on the access it needs. Restart it once (games running from Steam close), or wait: your library comes in the next time Steam opens."
            )),
            SteamState::Failed => Some(tr!(
                "Steam restarted but did not turn on the access Heronite needs, so it is not asked again. Your installed Steam games still show in the Collection."
            )),
            SteamState::NotInstalled => {
                Some(tr!("Steam was not found on this PC. Install Steam and sign in, then open it: Heronite reads your library by itself."))
            }
            _ => None,
        };
        text.map(str::to_string)
    }

    /// The button the status calls for: open Steam, restart it, or read.
    /// Reading is offered whenever Steam may be running, so a client that
    /// does not answer says why instead of keeping the button off.
    fn main_action(&self, cx: &App) -> (&'static str, &'static str, bool) {
        let s = self.svc.read(cx);
        match s.state() {
            SteamState::Closed => ("open", tr!("Open Steam"), true),
            // Blocked: once the other tool lets the access open, a restart brings it.
            // PortTaken: once the other program let the port go, a restart opens it.
            SteamState::NeedsRestart | SteamState::NotAnswering | SteamState::Blocked | SteamState::PortTaken => {
                ("restart", tr!("Restart Steam"), heronite_steam::can_restart())
            }
            SteamState::Restarting => ("restart", tr!("Restarting\u{2026}"), false),
            SteamState::Reading => ("read", tr!("Reading\u{2026}"), false),
            state => ("read", tr!("Read library now"), matches!(state, SteamState::Connected | SteamState::Opening)),
        }
    }

    /// A read next to the restart, when Steam runs but does not answer:
    /// it shows why.
    fn second_action(&self, cx: &App) -> Option<(&'static str, &'static str)> {
        matches!(self.svc.read(cx).state(), SteamState::NotAnswering | SteamState::PortTaken).then(|| ("read", tr!("Read library now")))
    }

    fn run_action(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "open" => self.svc.update(cx, |s, cx| s.open(cx)),
            "restart" => self.svc.update(cx, |s, cx| s.restart(cx)),
            "read" => self.read_now(cx),
            "sign-in" => self.svc.update(cx, |s, cx| s.begin_sign_in(cx)),
            "cancel-sign-in" => self.svc.update(cx, |s, cx| s.cancel_sign_in(cx)),
            "sign-out" => self.svc.update(cx, |s, cx| s.sign_out(cx)),
            _ => {}
        }
    }

    fn summary_line(&self, cx: &App) -> String {
        let s = &self.svc.read(cx).summary;
        if s.updated_at.is_none() {
            return tr!("Not read yet: only your installed Steam games are in the Collection.").to_string();
        }
        let mut parts =
            vec![trn!(s.games, "{n} game in your library", "{n} games in your library"), super::store::installed_count(s.installed)];
        let missing = s.games.saturating_sub(s.installed);
        if missing > 0 {
            parts.push(super::store::not_installed_count(missing));
        }
        if let Some(at) = s.updated_at.as_deref().and_then(read_at) {
            parts.push(at);
        }
        if s.source == SteamSource::Account {
            parts.push(tr!("From your Steam account").to_string());
        }
        parts.join("  \u{b7}  ")
    }

    /// The outcome of the last read asked for here.
    fn result_line(&self, cx: &App) -> Option<(String, bool)> {
        let s = self.svc.read(cx);
        if let Some(e) = &s.error {
            return Some((e.clone(), true));
        }
        match s.last.as_ref()? {
            Ok(r) if r.added > 0 => Some((trn!(r.added, "Added {n} game to the Collection.", "Added {n} games to the Collection."), false)),
            Ok(_) => Some((tr!("The Collection already has every game of your library.").to_string(), false)),
            Err(e) => Some((e.clone(), true)),
        }
    }

    fn render_library(&mut self, cx: &mut Context<Self>) -> Div {
        let (status, color, _) = self.status(cx);
        let (action, label, enabled) = self.main_action(cx);
        let mut rows: Vec<AnyElement> = vec![div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(14.))
            .child(icon(Icon::SportsEsports, px(44.), theme::accent()))
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
                            .child(tr!("Steam library")),
                    )
                    .child(div().flex().child(status_pill(status, color))),
            )
            .when_some(self.second_action(cx), |row, (second, second_label)| {
                row.child(action_button(
                    SharedString::from(format!("steam-{second}")),
                    ButtonKind::Glass,
                    second_label,
                    true,
                    cx.listener(move |this, _, _, cx| this.run_action(second, cx)),
                ))
            })
            .child(action_button(
                SharedString::from(format!("steam-{action}")),
                ButtonKind::Primary,
                label,
                enabled,
                cx.listener(move |this, _, _, cx| this.run_action(action, cx)),
            ))
            .into_any_element()];
        rows.push(div().py(px(12.)).text_size(px(13.)).text_color(theme::text_muted()).child(self.summary_line(cx)).into_any_element());
        if let Some(text) = self.explanation(cx) {
            rows.push(div().py(px(12.)).text_size(px(13.)).line_height(px(19.)).child(text).into_any_element());
        }
        rows.extend(self.render_fallback(cx));
        let mut column = div().child(caption(tr!("Library"))).child(card(rows));
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
        column
    }

    fn render_about(&self) -> Div {
        let text = tr!(
            "Heronite reads your library from the Steam client, so there is no login: only games are added (no tools, soundtracks, demos or games you hid in Steam). A game you already have from another store or added yourself is not added twice: it gets Steam's install instead. Steam keeps installing, updating and launching its games. When Steam on this PC cannot be read, signing in to your Steam account with a QR code brings your library in anyway."
        );
        div().child(caption(tr!("About"))).child(card([div()
            .py(px(12.))
            .text_size(px(13.))
            .line_height(px(19.))
            .text_color(theme::text_muted())
            .child(text)
            .into_any_element()]))
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let (status, _, _) = self.status(cx);
        let (action, label, enabled) = self.main_action(cx);
        let mut rows = vec![
            Row::header(tr!("Library")),
            Row::info("status", "Steam", status).icon(Icon::SportsEsports),
            Row::action(action, label).desc(self.summary_line(cx)).icon(Icon::Refresh).enabled(enabled),
        ];
        if let Some((second, second_label)) = self.second_action(cx) {
            rows.push(Row::action(second, second_label).icon(Icon::Refresh));
        }
        if let Some(text) = self.explanation(cx) {
            rows.push(Row::info("why", "Steam", text));
        }
        if let Some((text, _)) = self.result_line(cx) {
            rows.push(Row::info("result", text, ""));
        }
        rows.extend(self.account_console_rows(cx));
        rows
    }

    fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let list = self.console_list.render("steam-console", &rows, true);
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
                    .child(stores::console_tabs(Page::Steam))
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

impl Render for SteamPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let wide = stores::content_width(window) >= 1000.;
        let side = div().flex().flex_col().child(self.render_account(cx)).child(self.render_about());
        let body = columns(wide, [self.render_library(cx), side]);
        div()
            .size_full()
            .relative()
            .child(stores::sheet("steam-page", Page::Steam, tr!("Your account and library."), cx, body))
            .into_any_element()
    }
}

impl PadTarget for SteamPage {
    fn on_pad(&mut self, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(next) = stores::switch(Page::Steam, pad) {
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

impl PaletteCommands for SteamPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let syncing = crate::services::steam_library_started(cx).is_some_and(|s| s.read(cx).syncing);
        vec![PageCommand::new("read", tr!("Read the Steam library"), tr!("Add the games of your Steam library"), Icon::Refresh)
            .keywords(KEYWORDS)
            .disabled(syncing.then(|| tr!("Reading\u{2026}")))
            .stay()]
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if id == "read" {
            self.read_now(cx);
        }
        cx.notify();
    }

    fn page_modal(&self) -> bool {
        false
    }
}
