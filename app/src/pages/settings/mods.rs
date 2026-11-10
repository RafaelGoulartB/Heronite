//! Settings → Mods: the Nexus Mods account (the personal API key), who opens
//! the `nxm://` and `ror2mm://` links of the mod sites, and what applying
//! mods needs (fuse-overlayfs and FUSE). The mods themselves are managed per
//! game, from the game's Mods manager. The state lives in the mods service
//! (`services::mods`); the process work (xdg-mime) starts when the section is
//! shown, never when the page is created.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::tool::{Support, WindowsSupport};

use super::SettingsPage;
use crate::icons::Icon;
use crate::pages::{status_pill, tag_pill};
use crate::services::mods::{self, HandlerState, JobStage, ModsEvent};
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, icon_button_sm, row, ButtonKind};
use crate::ui::fields::{input_box, labeled, meta_text};
use crate::ui::icon;

/// Where Nexus Mods hands out the personal API key.
const API_KEYS_URL: &str = "https://www.nexusmods.com/settings/api-keys";

pub struct State {
    key: Entity<InputState>,
    /// The key was sent; the answer has not come yet.
    connecting: bool,
    /// The last thing the service said that is not about a game (connected, wrong key...).
    message: Option<(SharedString, bool)>,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsPage>) -> Self {
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let service = mods::service(cx);
        cx.subscribe(&service, |d: &mut SettingsPage, _, event: &ModsEvent, cx| {
            match event {
                ModsEvent::Account => d.mods.connecting = false,
                ModsEvent::Notice { game_key, message, error } if game_key.is_empty() => {
                    d.mods.message = Some((message.clone().into(), *error));
                }
                ModsEvent::Jobs | ModsEvent::Changed { .. } => {}
                _ => return,
            }
            cx.notify();
        })
        .detach();
        Self { key, connecting: false, message: None }
    }
}

/// Starts the work this section shows (it spawns processes: not at page creation).
pub fn shown(cx: &mut Context<SettingsPage>) {
    mods::service(cx).update(cx, |s, cx| {
        s.refresh_handlers(cx);
        s.refresh_support(cx);
    });
}

fn open_in_browser(url: &'static str, cx: &mut App) {
    cx.background_spawn(async move { crate::ui::open_external(url) }).detach();
}

fn connect(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let key = d.mods.key.read(cx).value().to_string();
    d.mods.connecting = true;
    d.mods.message = None;
    mods::service(cx).update(cx, |s, cx| s.set_api_key(key, cx));
    cx.notify();
}

fn disconnect(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.mods.message = None;
    mods::service(cx).update(cx, |s, cx| s.sign_out(cx));
    cx.notify();
}

/// What the service knows, copied out so the page can draw and listen.
struct Facts {
    account: Option<(String, bool)>,
    handlers: HandlerState,
    support: Option<Support>,
    /// Progress of the fuse-overlayfs download, when it runs (0..=100; `None` = unknown size).
    fetching: Option<Option<u32>>,
}

fn facts(cx: &App) -> Facts {
    let service = mods::started(cx);
    let Some(service) = service else {
        return Facts { account: None, handlers: HandlerState::Unknown, support: None, fetching: None };
    };
    let s = service.read(cx);
    Facts {
        account: s.account().map(|a| (a.account.name.clone(), a.account.is_premium)),
        handlers: s.handlers(),
        support: s.support().cloned(),
        fetching: s.jobs(None).find(|j| j.stage == JobStage::Tool).map(|j| j.progress.fraction().map(|f| (f * 100.0).round() as u32)),
    }
}

fn status_line(text: impl Into<SharedString>, ok: bool) -> Div {
    div().text_size(px(13.)).text_color(if ok { theme::text_primary() } else { theme::danger_text() }).max_w(px(420.)).child(text.into())
}

fn handlers_text(state: HandlerState) -> (&'static str, bool) {
    match state {
        HandlerState::Unknown => (tr!("Looking…"), true),
        HandlerState::Ours => (tr!("Heronite opens them"), true),
        HandlerState::Other => (tr!("Another app opens them"), false),
        HandlerState::None => (tr!("No app opens them"), true),
    }
}

/// `Some` when FUSE itself is the problem (fuse-overlayfs missing is not: it is downloaded).
fn fuse_problem(support: &Support) -> Option<String> {
    if support.fusermount.is_none() || !support.dev_fuse {
        support.problem()
    } else {
        None
    }
}

fn overlay_text(f: &Facts) -> (String, bool) {
    if let Some(progress) = f.fetching {
        return match progress {
            Some(p) => (tr!("Downloading… {percent}%", percent = p), true),
            None => (tr!("Downloading…").to_string(), true),
        };
    }
    match f.support.as_ref().map(|s| s.overlay.as_ref()) {
        None => (tr!("Looking…").to_string(), true),
        Some(Some(path)) => (tr!("Found at {path}", path = path.display()), true),
        Some(None) => (tr!("Not downloaded yet: it is fetched the first time a game with mods starts").to_string(), true),
    }
}

fn fuse_text(f: &Facts) -> (String, bool) {
    match f.support.as_ref() {
        None => (tr!("Looking…").to_string(), true),
        Some(s) => match fuse_problem(s) {
            Some(problem) => (problem, false),
            None => (tr!("Ready").to_string(), true),
        },
    }
}

fn account_card(d: &SettingsPage, f: &Facts, cx: &mut Context<SettingsPage>) -> Div {
    let Some((name, premium)) = f.account.clone() else {
        let connecting = d.mods.connecting;
        return card_box()
            .py(px(12.))
            .gap(px(12.))
            .child(meta_text(tr!(
                "Nexus Mods asks for your personal API key to download mods and look for their updates. It stays on this computer."
            )))
            .child(labeled(tr!("Personal API key"), input_box(&d.mods.key, false)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(action_button(
                        "mods-connect",
                        ButtonKind::Primary,
                        if connecting { tr!("Connecting…") } else { tr!("Connect") },
                        !connecting,
                        cx.listener(|d, _, _, cx| connect(d, cx)),
                    ))
                    .child(icon_button_sm(
                        "mods-get-key",
                        ButtonKind::Glass,
                        Icon::OpenInNew,
                        tr!("Get your API key"),
                        true,
                        |_, _, cx| open_in_browser(API_KEYS_URL, cx),
                    )),
            );
    };
    let (pill, detail) = if premium {
        (status_pill(tr!("Premium"), theme::success()), tr!("Mods install straight from Heronite."))
    } else {
        (tag_pill(tr!("Free")), tr!("Free accounts download from the \"Mod manager download\" button on a mod's page on the site."))
    };
    card([row(
        name,
        Some(detail),
        div().flex().items_center().gap(px(10.)).child(pill).child(action_button(
            "mods-disconnect",
            ButtonKind::Glass,
            tr!("Disconnect"),
            true,
            cx.listener(|d, _, _, cx| disconnect(d, cx)),
        )),
    )
    .into_any_element()])
}

fn links_card(f: &Facts, cx: &mut Context<SettingsPage>) -> Div {
    let (text, ok) = handlers_text(f.handlers);
    let mut control = div().flex().items_center().gap(px(10.));
    if f.handlers == HandlerState::Ours {
        control = control.child(icon(Icon::Check, px(16.), theme::success()));
    }
    control = control.child(status_line(text, ok));
    if f.handlers != HandlerState::Ours && f.handlers != HandlerState::Unknown {
        control = control.child(action_button(
            "mods-register",
            ButtonKind::Primary,
            tr!("Open them with Heronite"),
            true,
            cx.listener(|_, _, _, cx| mods::service(cx).update(cx, |s, cx| s.register_handlers(cx))),
        ));
    }
    card([row(
        "nxm:// · ror2mm://",
        Some(tr!("The \"Mod manager download\" button of Nexus Mods and \"Install with mod manager\" of Thunderstore start the download in Heronite.")),
        control,
    )
    .into_any_element()])
}

/// Windows: how the files go in, and what the system offers beyond that
/// (shown, never turned on from here).
fn windows_card(w: WindowsSupport) -> Div {
    let yes_no = |on: bool| if on { tr!("On") } else { tr!("Off") };
    card([
        row(
            tr!("Hard links and copies"),
            Some(tr!(
                "While the game runs, the mods' files are in its folder: linked when they are on the same drive as the mods, copied otherwise. The game's own files a mod replaces are set aside and put back when it ends, or at the next start after a crash."
            )),
            status_line(tr!("Ready"), true),
        )
        .into_any_element(),
        row(tr!("Projected File System"), Some(tr!("An optional Windows feature. Not used yet.")), status_line(yes_no(w.projfs), true))
            .into_any_element(),
        row(tr!("Symbolic links"), Some(tr!("Need Developer Mode or an administrator. Not used.")), status_line(yes_no(w.symlinks), true))
            .into_any_element(),
    ])
}

fn apply_card(f: &Facts, cx: &mut Context<SettingsPage>) -> Div {
    if let Some(w) = f.support.as_ref().and_then(|s| s.windows) {
        return windows_card(w);
    }
    let (overlay, overlay_ok) = overlay_text(f);
    let (fuse, fuse_ok) = fuse_text(f);
    let missing = f.support.as_ref().is_some_and(|s| s.overlay.is_none()) && f.fetching.is_none();
    let mut overlay_control = div().flex().items_center().gap(px(10.)).child(status_line(overlay, overlay_ok));
    if missing {
        overlay_control = overlay_control.child(action_button(
            "mods-fetch-tool",
            ButtonKind::Glass,
            tr!("Download now"),
            true,
            cx.listener(|_, _, _, cx| mods::service(cx).update(cx, |s, cx| s.fetch_tool(cx))),
        ));
    }
    card([
        row("fuse-overlayfs", Some(tr!("Lays the mods over the game's folder while it runs.")), overlay_control).into_any_element(),
        row("FUSE", None, status_line(fuse, fuse_ok)).into_any_element(),
    ])
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let f = facts(cx);
    let message = d.mods.message.clone().map(|(text, error)| {
        div().px(px(6.)).pt(px(8.)).text_size(px(13.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
    });
    let notes = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(6.))
        .child(meta_text(if cfg!(windows) {
            tr!(
                "Mods apply only when the game is started from Heronite. Their files are put into the game's folder while it runs and taken out when it ends; what the game writes there goes to the mods' overwrite folder."
            )
        } else {
            tr!(
                "Mods apply only when the game is started from Heronite. The game's own folder is never changed: the mods are laid over it while the game runs and taken away when it ends."
            )
        }))
        .child(meta_text(tr!(
            "Nexus Mods shows its \"Mod manager download\" button only when your preferences on the site allow it."
        )));
    // The feature's own caveat first: what follows only matters if mods suit the game.
    let experimental = div()
        .flex()
        .items_start()
        .gap(px(10.))
        .px(px(6.))
        .pb(px(4.))
        .child(div().pt(px(1.)).child(crate::dialogs::mods::experimental_pill()))
        .child(div().flex_1().min_w_0().child(meta_text(tr!(
            "Mods are new in Heronite. They work for games whose mods are files in the game's folder; games that keep mods elsewhere, or that need their own tools, are not covered. Each game's Mods says what to expect for it."
        ))));
    div()
        .flex()
        .flex_col()
        .child(experimental)
        .child(caption(tr!("Nexus Mods account")))
        .child(account_card(d, &f, cx))
        .children(message)
        .child(caption(tr!("Links from the sites")))
        .child(links_card(&f, cx))
        .child(caption(tr!("Applying mods")))
        .child(apply_card(&f, cx))
        .child(div().h(px(12.)))
        .child(notes)
        .into_any_element()
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let f = facts(cx);
    let mut rows = vec![Row::header(tr!("Nexus Mods account"))];
    match &f.account {
        Some((name, premium)) => {
            rows.push(Row::info("account", tr!("Connected as"), name.clone()));
            rows.push(Row::info("plan", tr!("Account"), if *premium { tr!("Premium") } else { tr!("Free") }));
            rows.push(Row::action("disconnect", tr!("Disconnect")));
        }
        None => {
            let key = d.mods.key.read(cx).value().to_string();
            rows.push(
                Row::text("api-key", tr!("Personal API key"), "\u{2022}".repeat(key.chars().count().min(24)))
                    .desc(tr!("Needed to download mods and look for updates")),
            );
            rows.push(
                Row::action("connect", if d.mods.connecting { tr!("Connecting…") } else { tr!("Connect") }).enabled(!d.mods.connecting),
            );
        }
    }
    if let Some((text, _)) = &d.mods.message {
        rows.push(Row::info("message", tr!("Last result"), text.clone()));
    }
    let (links, _) = handlers_text(f.handlers);
    rows.push(Row::header(tr!("Links from the sites")));
    rows.push(Row::info("links", "nxm:// · ror2mm://", links));
    if f.handlers != HandlerState::Ours && f.handlers != HandlerState::Unknown {
        rows.push(Row::action("register", tr!("Open them with Heronite")));
    }
    rows.push(Row::header(tr!("Applying mods")));
    if let Some(w) = f.support.as_ref().and_then(|s| s.windows) {
        rows.push(Row::info("method", tr!("Hard links and copies"), tr!("Ready")));
        rows.push(Row::info("projfs", tr!("Projected File System"), if w.projfs { tr!("On") } else { tr!("Off") }));
        return rows;
    }
    let (overlay, _) = overlay_text(&f);
    let (fuse, _) = fuse_text(&f);
    rows.push(Row::info("overlay", "fuse-overlayfs", overlay));
    rows.push(Row::info("fuse", "FUSE", fuse));
    if f.support.as_ref().is_some_and(|s| s.overlay.is_none()) && f.fetching.is_none() {
        rows.push(Row::action("fetch-tool", tr!("Download now")));
    }
    rows
}

pub(super) fn console_apply(
    d: &mut SettingsPage,
    outcome: crate::console::list::Outcome,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) {
    use crate::console::list::Outcome;
    let Outcome::Activate(id) = outcome else { return };
    match id.as_ref() {
        "api-key" => d.console_osk(tr!("Nexus Mods API key"), "", window, cx, |d, text, window, cx| {
            d.mods.key.update(cx, |s, cx| s.set_value(text.trim().to_string(), window, cx));
        }),
        "connect" => connect(d, cx),
        "disconnect" => disconnect(d, cx),
        "register" => mods::service(cx).update(cx, |s, cx| s.register_handlers(cx)),
        "fetch-tool" => mods::service(cx).update(cx, |s, cx| s.fetch_tool(cx)),
        _ => {}
    }
}
