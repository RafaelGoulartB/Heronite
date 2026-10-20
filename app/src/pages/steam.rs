//! The Steam tab of the Stores: the library read from the Steam client
//! (`services::steam_library`). No login: the client is signed in already;
//! the page says whether it can be read, opens or restarts Steam when that
//! is what is missing, and reads it on request.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::{columns, content_width, page_close, sheet, status_pill, stores, Page, PageCtx, PageEvent};
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
        let subs = vec![
            cx.observe(&svc, |_, _, cx| cx.notify()),
            cx.subscribe(&svc, |_, _, ev: &SteamLibraryEvent, cx| {
                let SteamLibraryEvent::Imported(report) = ev;
                if report.added > 0 || report.playtime > 0 {
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
            SteamState::Connected | SteamState::Reading => (tr!("Steam is running").into(), theme::success(), true),
            SteamState::Closed => (tr!("Steam is closed").into(), theme::text_muted(), false),
            SteamState::Opening => (tr!("Steam is starting").into(), theme::accent(), false),
            SteamState::Restarting => (tr!("Restarting Steam\u{2026}").into(), theme::accent(), false),
            SteamState::NeedsRestart => (tr!("Restart Steam once").into(), theme::warning(), false),
            SteamState::Failed => (tr!("Could not connect").into(), theme::danger_text(), false),
            SteamState::NotInstalled => (tr!("Steam not found").into(), theme::text_muted(), false),
        }
    }

    fn explanation(&self, cx: &App) -> Option<&'static str> {
        match self.svc.read(cx).state() {
            SteamState::Closed => Some(tr!("Open Steam and Heronite reads your library by itself. The access it needs is turned on already: no restart.")),
            SteamState::Opening => Some(tr!("Steam is starting. If it asks, sign in: your library is read as soon as Steam is ready.")),
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
        }
    }

    /// The button the status calls for: open Steam, restart it, or read.
    fn main_action(&self, cx: &App) -> (&'static str, &'static str, bool) {
        let s = self.svc.read(cx);
        match s.state() {
            SteamState::Closed => ("open", tr!("Open Steam"), true),
            SteamState::NeedsRestart => ("restart", tr!("Restart Steam"), heronite_steam::can_restart()),
            SteamState::Restarting => ("restart", tr!("Restarting\u{2026}"), false),
            SteamState::Reading => ("read", tr!("Reading\u{2026}"), false),
            state => ("read", tr!("Read library now"), state == SteamState::Connected),
        }
    }

    fn run_action(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "open" => self.svc.update(cx, |s, cx| s.open(cx)),
            "restart" => self.svc.update(cx, |s, cx| s.restart(cx)),
            "read" => self.read_now(cx),
            _ => {}
        }
    }

    fn summary_line(&self, cx: &App) -> String {
        let s = &self.svc.read(cx).summary;
        let mut parts =
            vec![trn!(s.games, "{n} game in your library", "{n} games in your library"), super::store::installed_count(s.installed)];
        if let Some(at) = s.updated_at.as_deref().and_then(read_at) {
            parts.push(at);
        }
        if s.updated_at.is_none() {
            return tr!("Not read yet").to_string();
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
            "Heronite reads your library from the Steam client, so there is no login: only games are added (no tools, soundtracks, demos or games you hid in Steam). A game you already have from another store or added yourself is not added twice: it gets Steam's install instead. Steam keeps installing, updating and launching its games."
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
        if let Some(text) = self.explanation(cx) {
            rows.push(Row::info("why", "Steam", text));
        }
        if let Some((text, _)) = self.result_line(cx) {
            rows.push(Row::info("result", text, ""));
        }
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
        let wide = content_width(window) >= 1000.;
        let close = page_close(cx);
        let body = div()
            .flex()
            .flex_col()
            .child(stores::tabs(Page::Steam, cx))
            .child(columns(wide, [self.render_library(cx), self.render_about()]));
        div()
            .size_full()
            .relative()
            .child(sheet("steam-page", stores::title(), Some(tr!("Your account and library.").into()), None, 1320., window, close, body))
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
