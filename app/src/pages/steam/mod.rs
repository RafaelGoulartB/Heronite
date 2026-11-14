//! The Steam page of the Stores (`services::steam_library`): the library
//! read from the Steam client running here (signed in already), with no
//! login. The card says whether it can be read and opens or restarts Steam
//! when that is what is missing.
//!
//! Heronite no longer signs in to Steam: its sign-in with a QR code, as the
//! Steam mobile app, got accounts locked by Steam's security.
//!
//! A switch turns reading Steam off for the users who add their games by
//! themselves; "Remove Steam games" takes out the ones Heronite added (asked
//! first, in the card).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::steam_owned::SteamSource;
use heronite_i18n::{tr, trn};
use heronite_steam::ClientApiFlag;

use super::commands::{CommandCtx, PageCommand, PaletteCommands};
use super::methods::{self, Method, Tone};
use super::{stores, Page, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::sheet::{Sheet, SheetEvent};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::steam_library::{SteamLibrary, SteamLibraryEvent, SteamState};
use crate::ui::controls::{action_button, button, icon_button_sm, toggle_row, ButtonKind};

pub struct SteamPage {
    svc: Entity<SteamLibrary>,
    console_list: crate::console::list::ListNav,
    /// "Remove Steam games" is asked about, in the card.
    confirm_remove: bool,
    /// The console's confirmation.
    layer: crate::console::Layer,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PageEvent> for SteamPage {}

const KEYWORDS: &str = "steam valve library import games biblioteca importar jogos";

/// "Read on 13 Oct, 14:02" for an RFC 3339 date.
fn read_at(updated_at: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(updated_at).ok()?.with_timezone(&chrono::Local);
    Some(tr!("Last read {when}", when = t.format("%Y-%m-%d %H:%M")))
}

/// The confirmation of "Remove Steam games": `n` games, and whether
/// reading Steam is on (removing then turns it off too).
fn remove_message(n: usize, reading: bool) -> String {
    let mut said = vec![
        trn!(n, "Remove {n} Steam game from the Collection?", "Remove {n} Steam games from the Collection?").to_string(),
        tr!("Nothing is uninstalled and Steam is not touched. Playtime, status and Collection images for them are deleted.").to_string(),
        tr!("Games you added yourself or got from other stores stay. If you own one on Steam too, it only loses its link to Steam's copy.")
            .to_string(),
    ];
    if reading {
        said.push(tr!("Reading your Steam library turns off, so they do not come back. You can turn it on again.").to_string());
    }
    said.join(" ")
}

impl SteamPage {
    pub fn new(_ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = crate::services::steam_library(cx);
        stores::watch(cx);
        let subs = vec![
            cx.observe(&svc, |_, _, cx| cx.notify()),
            cx.subscribe(&svc, |_, _, ev: &SteamLibraryEvent, cx| match ev {
                SteamLibraryEvent::Imported(report) if report.changed() => cx.emit(PageEvent::LibraryChanged),
                SteamLibraryEvent::Imported(_) => {}
                SteamLibraryEvent::Removed => cx.emit(PageEvent::LibraryChanged),
            }),
        ];
        svc.update(cx, |s, cx| {
            s.check(cx);
            s.reload(cx);
        });
        let mut this = Self { svc, console_list: Default::default(), confirm_remove: false, layer: Default::default(), _subs: subs };
        // `HERONITE_DEBUG_STEAM_REMOVE=1` shows the confirmation of "Remove Steam games" (for screenshots).
        if crate::env::flag("HERONITE_DEBUG_STEAM_REMOVE") {
            this.confirm_remove = true;
            if crate::console::active() {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                    this.update(cx, |this, cx| this.console_confirm(cx)).ok();
                })
                .detach();
            }
        }
        this
    }

    /// Checks again whether Steam can be read (the page was shown again).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| {
            s.check(cx);
            s.reload(cx);
        });
    }

    fn set_reading(&mut self, on: bool, cx: &mut Context<Self>) {
        self.confirm_remove = false;
        self.svc.update(cx, |s, cx| s.set_enabled(on, cx));
    }

    /// Whether "Remove Steam games" can be asked now.
    fn can_remove(&self, cx: &App) -> bool {
        let s = self.svc.read(cx);
        s.removable > 0 && !s.syncing && !s.removing
    }

    fn remove_games(&mut self, cx: &mut Context<Self>) {
        self.confirm_remove = false;
        self.svc.update(cx, |s, cx| s.remove_games(cx));
    }

    /// What the confirmation says.
    fn remove_message(&self, cx: &App) -> String {
        let s = self.svc.read(cx);
        remove_message(s.removable, s.enabled())
    }

    fn read_now(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.sync(cx));
    }

    /// The status of the client.
    fn status(&self, cx: &App) -> (String, Tone) {
        let (text, tone) = match self.svc.read(cx).state() {
            SteamState::Off => (tr!("Off"), Tone::Neutral),
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
            SteamState::Off => {
                let mut said = tr!("Reading is off: Heronite adds no Steam game by itself. Turn it on to bring your library in again.").to_string();
                if s.removable > 0 {
                    said.push(' ');
                    said.push_str(&trn!(
                        s.removable,
                        "{n} Steam game added before is still in your Collection.",
                        "{n} Steam games added before are still in your Collection."
                    ));
                }
                return Some(said);
            }
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
        if !self.svc.read(cx).enabled() && s.updated_at.is_none() {
            return tr!("Reading is off: only the games you add yourself, and what other stores bring, are in the Collection.").to_string();
        }
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
        match &s.removal {
            Some(Ok(n)) => {
                return Some((
                    trn!(*n, "Removed {n} Steam game from the Collection.", "Removed {n} Steam games from the Collection.").to_string(),
                    false,
                ))
            }
            Some(Err(e)) => return Some((tr!("Could not remove every Steam game: {err}", err = e.as_str()), true)),
            None => {}
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
        let reading = self.svc.read(cx).enabled();
        let mut actions: Vec<AnyElement> = button.into_iter().collect();
        if self.can_remove(cx) && !self.confirm_remove {
            actions.push(
                action_button(
                    "steam-remove",
                    ButtonKind::Glass,
                    tr!("Remove Steam games"),
                    true,
                    cx.listener(|this, _, _, cx| {
                        this.confirm_remove = true;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            );
        }
        let mut body = vec![toggle_row(
            "steam-reading",
            tr!("Read my Steam library"),
            Some(tr!("Heronite adds your Steam games to the Collection. Turn it off to add only the games you choose yourself.")),
            reading,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.set_reading(!reading, cx)))
        .into_any_element()];
        if self.confirm_remove && self.can_remove(cx) {
            body.push(self.render_confirm(cx));
        }
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
            body,
            footer: Vec::new(),
            actions,
            message: None,
        })
    }

    /// The question under "Remove Steam games": text, Cancel, and the danger button.
    fn render_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let n = self.svc.read(cx).removable;
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(div().w_full().text_size(px(13.)).line_height(px(19.)).child(self.remove_message(cx)))
            .child(div().flex_1())
            .child(button("steam-remove-no", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(|this, _, _, cx| {
                this.confirm_remove = false;
                cx.notify();
            })))
            .child(
                button("steam-remove-yes", ButtonKind::Danger, trn!(n, "Remove {n} game", "Remove {n} games"))
                    .on_click(cx.listener(|this, _, _, cx| this.remove_games(cx))),
            )
            .into_any_element()
    }

    fn console_rows(&self, cx: &App) -> Vec<Row> {
        let (status, _) = self.status(cx);
        let (action, label, enabled) = self.main_action(cx);
        let (syncing, reading, removing) = {
            let s = self.svc.read(cx);
            (s.syncing, s.enabled(), s.removing)
        };
        let mut rows = vec![
            Row::header(tr!("Your library")),
            Row::action("read", if syncing { tr!("Reading\u{2026}") } else { tr!("Read library now") })
                .desc(self.summary_line(cx))
                .icon(Icon::Refresh)
                .enabled(!syncing && reading),
            Row::toggle("reading", tr!("Read my Steam library"), reading)
                .desc(tr!("Heronite adds your Steam games to the Collection. Turn it off to add only the games you choose yourself.")),
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
        if self.can_remove(cx) || removing {
            rows.push(
                Row::action("remove", if removing { tr!("Removing\u{2026}") } else { tr!("Remove Steam games") })
                    .icon(Icon::FaTrash)
                    .danger()
                    .enabled(!removing),
            );
        }
        rows
    }

    /// The console's confirmation: a sheet over the page.
    fn console_confirm(&mut self, cx: &mut Context<Self>) {
        let n = self.svc.read(cx).removable;
        let rows = vec![Row::action("no", tr!("Cancel")), Row::action("yes", trn!(n, "Remove {n} game", "Remove {n} games")).danger()];
        let message = self.remove_message(cx);
        let sheet = cx.new(|_| Sheet::new(tr!("Remove Steam games").to_string(), Some(message), rows));
        let sub = cx.subscribe(&sheet, move |this, _, ev: &SheetEvent, cx| {
            let SheetEvent::Activate(id) = ev;
            this.layer.close();
            if id.as_ref() == "yes" {
                this.remove_games(cx);
            }
            cx.notify();
        });
        self.layer.open(&sheet, sub);
    }

    fn render_console(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows(cx);
        let list = self.console_list.render("steam-console", &rows, !self.layer.is_open());
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
            .children(self.layer.view())
            .into_any_element()
    }
}

impl Render for SteamPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(cx);
        }
        let wide = stores::content_width(window) >= 1000.;
        let (syncing, reading) = {
            let s = self.svc.read(cx);
            (s.syncing, s.enabled())
        };
        let read = icon_button_sm(
            "steam-read",
            ButtonKind::Glass,
            Icon::Refresh,
            if syncing { tr!("Reading\u{2026}") } else { tr!("Read library now") },
            !syncing && reading,
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
    fn on_pad(&mut self, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.layer.pad(pad, window, cx) {
            cx.notify();
            return true;
        }
        if let Some(next) = stores::switch(Page::Steam, pad) {
            cx.emit(PageEvent::Navigate(next));
            return true;
        }
        let rows = self.console_rows(cx);
        let used = match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) if id.as_ref() == "remove" => {
                self.console_confirm(cx);
                true
            }
            Outcome::Activate(id) => {
                self.run_action(id.as_ref(), cx);
                true
            }
            Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "reading" => {
                self.set_reading(on, cx);
                true
            }
            Outcome::Ignored => false,
            _ => true,
        };
        cx.notify();
        used
    }

    fn hints(&self, cx: &App) -> Vec<Hint> {
        if let Some(h) = self.layer.hints(cx) {
            return h;
        }
        vec![hint(Btn::A, tr!("Select"))]
    }
}

impl PaletteCommands for SteamPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let svc = crate::services::steam_library_started(cx);
        let syncing = svc.as_ref().is_some_and(|s| s.read(cx).syncing);
        let off = svc.as_ref().is_some_and(|s| !s.read(cx).enabled());
        let why = if off { Some(tr!("Reading Steam is off")) } else { syncing.then(|| tr!("Reading\u{2026}")) };
        vec![PageCommand::new("read", tr!("Read the Steam library"), tr!("Add the games of your Steam library"), Icon::Refresh)
            .keywords(KEYWORDS)
            .disabled(why)
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

#[cfg(test)]
mod tests {
    use super::remove_message;

    #[test]
    fn the_confirmation_says_what_stays_and_that_reading_turns_off() {
        let on = remove_message(3, true);
        assert!(on.starts_with("Remove 3 Steam games from the Collection?"));
        assert!(on.contains("Nothing is uninstalled"));
        assert!(on.contains("turns off"));
        let off = remove_message(1, false);
        assert!(off.starts_with("Remove 1 Steam game from the Collection?"));
        assert!(!off.contains("turns off"));
    }
}
