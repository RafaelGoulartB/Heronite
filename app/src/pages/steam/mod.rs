//! The Steam page of the Stores (`services::steam_library`): the library
//! read from the Steam client running here (signed in already), with no
//! login. The card says whether it can be read and opens or restarts Steam
//! when that is what is missing.
//!
//! Heronite no longer signs in to Steam: its sign-in with a QR code, as the
//! Steam mobile app, got accounts locked by Steam's security.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::steam_owned::SteamSource;
use heronite_i18n::{tr, trn};
use heronite_steam::ClientApiFlag;

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::methods::{self, Method, Tone};
use super::{stores, Page, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::steam_library::{SteamLibrary, SteamLibraryEvent, SteamState};
use crate::ui::controls::{action_button, icon_button_sm, ButtonKind};

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

    /// The status of the client.
    fn status(&self, cx: &App) -> (String, Tone) {
        let (text, tone) = match self.svc.read(cx).state() {
            SteamState::Checking => (tr!("Checking Steam\u{2026}"), Tone::Neutral),
            SteamState::Connected => (tr!("Steam is running"), Tone::Good),
            // A read asked for while Steam does not answer fails at once: not "running".
            SteamState::Reading => (tr!("Reading\u{2026}"), Tone::Accent),
            SteamState::Closed => (tr!("Steam is closed"), Tone::Neutral),
            SteamState::Opening => (tr!("Steam is starting"), Tone::Accent),
            SteamState::Restarting => (tr!("Restarting Steam\u{2026}"), Tone::Accent),
            SteamState::NotAnswering => (tr!("Steam is not answering"), Tone::Warning),
            SteamState::NeedsRestart => (tr!("Restart Steam once"), Tone::Warning),
            SteamState::Blocked => (tr!("Blocked by another tool"), Tone::Warning),
            SteamState::PortTaken => (tr!("Port in use"), Tone::Warning),
            SteamState::Failed => (tr!("Could not connect"), Tone::Warning),
            SteamState::NotInstalled => (tr!("Steam not found"), Tone::Neutral),
        };
        (text.to_string(), tone)
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
            Ok(r) => {
                let mut said: Vec<String> = Vec::new();
                if r.added > 0 {
                    said.push(trn!(r.added, "Added {n} game to the Collection.", "Added {n} games to the Collection.").to_string());
                }
                if r.hidden_added > 0 {
                    said.push(
                        trn!(
                            r.hidden_added,
                            "Added {n} game you hid in Steam, hidden here too",
                            "Added {n} games you hid in Steam, hidden here too"
                        )
                        .to_string(),
                    );
                }
                if said.is_empty() {
                    said.push(tr!("The Collection already has every game of your library.").to_string());
                }
                Some((said.join("  \u{b7}  "), false))
            }
            Err(e) => Some((e.clone(), true)),
        }
    }

    /// The Steam client on this PC.
    fn render_local(&mut self, cx: &mut Context<Self>) -> Div {
        let state = self.svc.read(cx).state();
        let stuck = self.svc.read(cx).client_stuck();
        let (action, label, enabled) = self.main_action(cx);
        // Reading is the strip's button; the card has the rest.
        let button = (action != "read").then(|| {
            action_button(
                SharedString::from(format!("steam-{action}")),
                ButtonKind::Primary,
                label,
                enabled,
                cx.listener(move |this, _, _, cx| this.run_action(action, cx)),
            )
            .into_any_element()
        });
        let notice =
            self.explanation(cx).map(|text| (text, if stuck || state == SteamState::Failed { Tone::Warning } else { Tone::Neutral }));
        methods::method_card(Method {
            eyebrow: tr!("Local"),
            icon: Icon::Computer,
            title: tr!("From Steam on this PC").into(),
            badge: methods::no_login(),
            status: Some(self.status(cx)).map(|(text, tone)| (text.into(), tone)),
            facts: vec![
                methods::brings(tr!("Reads the Steam app already open and signed in here")),
                methods::brings(tr!("Brings the games you hid in Steam, hidden here too, and Family Sharing games")),
                methods::needs(tr!("Needs Steam running on this PC")),
            ],
            notice,
            body: Vec::new(),
            footer: Vec::new(),
            actions: button.into_iter().collect(),
            message: None,
        })
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let (status, _) = self.status(cx);
        let (action, label, enabled) = self.main_action(cx);
        let syncing = self.svc.read(cx).syncing;
        let mut rows = vec![
            Row::header(tr!("Your library")),
            Row::action("read", if syncing { tr!("Reading\u{2026}") } else { tr!("Read library now") })
                .desc(self.summary_line(cx))
                .icon(Icon::Refresh)
                .enabled(!syncing),
            Row::header(format!("{} \u{b7} {}", tr!("Local"), tr!("No login needed"))),
            Row::info("status", tr!("From Steam on this PC"), status).icon(Icon::Computer),
        ];
        if action != "read" {
            rows.push(Row::action(action, label).icon(Icon::Refresh).enabled(enabled));
        }
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
        let wide = stores::content_width(window) >= 1000.;
        let syncing = self.svc.read(cx).syncing;
        let read = icon_button_sm(
            "steam-read",
            ButtonKind::Glass,
            Icon::Refresh,
            if syncing { tr!("Reading\u{2026}") } else { tr!("Read library now") },
            !syncing,
            cx.listener(|this, _, _, cx| this.run_action("read", cx)),
        );
        let strip = methods::library_strip(self.summary_line(cx), self.result_line(cx), read);
        let card = self.render_local(cx);
        let cards = div().w_full().when(wide, |d| d.max_w(px(640.))).child(card);
        let note = methods::footnote(tr!(
            "Only games come in: no tools, soundtracks or demos. A game you already have from another store or added yourself is not added twice: it gets Steam's install instead. Steam keeps installing, updating and launching its games."
        ));
        let body = methods::page(strip, cards, [note.into_any_element()]);
        let lead = tr!("Heronite reads your Steam library from the Steam app on this PC. It never asks you to sign in to Steam.");
        div().size_full().relative().child(stores::sheet("steam-page", Page::Steam, lead, cx, body)).into_any_element()
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
