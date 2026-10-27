//! What a client page says and offers for the state of its client: the
//! pill, the main button, the setup step, the account lines, the dates.
//! Pure, so the desktop page, the console and the palette agree.

use std::path::Path;

use gpui_kit::*;
use heronite_clients::setup::Step;
use heronite_clients::Account;
use heronite_data::clients::Client;
use heronite_i18n::tr;

use crate::services::clients::{Activity, ClientState, Progress};
use crate::services::proton_setup::Setup as WineSetup;
use crate::theme;

/// The pill beside the client's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pill {
    NotSetUp,
    SettingUp,
    Ready,
    Running,
}

pub(super) fn pill(st: &ClientState) -> Pill {
    if matches!(st.activity, Activity::SettingUp(_)) {
        Pill::SettingUp
    } else if !st.installed() {
        Pill::NotSetUp
    } else if st.running {
        Pill::Running
    } else {
        Pill::Ready
    }
}

impl Pill {
    pub(super) fn text(self) -> &'static str {
        match self {
            Pill::NotSetUp => tr!("Not set up"),
            Pill::SettingUp => tr!("Setting up\u{2026}"),
            Pill::Ready => tr!("Ready"),
            Pill::Running => tr!("Running"),
        }
    }

    pub(super) fn color(self) -> Rgba {
        match self {
            Pill::NotSetUp => theme::text_muted(),
            Pill::SettingUp | Pill::Running => theme::accent(),
            Pill::Ready => theme::success(),
        }
    }
}

/// The client's main button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Main {
    /// `setup`, `cancel`, `open`, `close` (or `none` while it works), the id
    /// `ClientPage::run_action` takes.
    pub id: &'static str,
    pub label: String,
    pub enabled: bool,
    /// Why it is disabled, when the user can do something about it.
    pub why: Option<&'static str>,
}

pub(super) fn main_action(client: Client, st: &ClientState) -> Main {
    let name = client.name();
    let main = |id, label: String, enabled| Main { id, label, enabled, why: None };
    match &st.activity {
        Activity::SettingUp(_) => main("cancel", tr!("Cancel").to_string(), !st.cancelling()),
        Activity::Opening => main("none", tr!("Opening\u{2026}").to_string(), false),
        Activity::Closing => main("none", tr!("Closing\u{2026}").to_string(), false),
        Activity::Adopting | Activity::Forgetting => main("none", tr!("Working\u{2026}").to_string(), false),
        Activity::Idle if !st.installed() => main("setup", tr!("Set up {client}", client = name), st.status.is_some()),
        Activity::Idle if st.running => {
            let mut m = main("close", tr!("Close {client}", client = name), !st.game_running);
            m.why = st.game_running.then(|| tr!("A game is running: close it first."));
            m
        }
        Activity::Idle => main("open", tr!("Open {client}", client = name), true),
    }
}

/// What the setup is doing, for the line under the client's name.
pub(super) fn step_text(client: Client, progress: &Progress, cancelling: bool) -> String {
    if cancelling {
        return tr!("Cancelling\u{2026}").to_string();
    }
    let percent = |p: f64| p.clamp(0., 100.).round() as u32;
    match progress {
        Progress::Wine(WineSetup::Downloading { percentage, .. }) => {
            tr!("Downloading Proton\u{2026} {percent}%", percent = percent(*percentage))
        }
        Progress::Wine(WineSetup::Unzipping { .. }) => tr!("Unpacking Proton\u{2026}").to_string(),
        Progress::Wine(_) => tr!("Looking for a Proton\u{2026}").to_string(),
        Progress::Step(Step::Downloading(Some(p))) => tr!("Downloading the installer\u{2026} {percent}%", percent = percent(*p)),
        Progress::Step(Step::Downloading(None)) => tr!("Downloading the installer\u{2026}").to_string(),
        Progress::Step(Step::CreatingPrefix) => tr!("Creating the Wine prefix\u{2026}").to_string(),
        Progress::Step(Step::Installing) => tr!("Installing {client}\u{2026} this can take a few minutes", client = client.name()),
    }
}

/// How far along the setup is, `0..=100`; `None` when it gives no number.
pub(super) fn step_percent(progress: &Progress) -> Option<f32> {
    match progress {
        Progress::Wine(WineSetup::Downloading { percentage, .. }) => Some(*percentage as f32),
        Progress::Step(Step::Downloading(Some(p))) => Some(*p as f32),
        _ => None,
    }
}

/// The account card's title and the line under it.
pub(super) fn account_lines(client: Client, account: &Account) -> (String, Option<String>) {
    match account {
        Account::SignedIn(Some(name)) => (tr!("Signed in as {name}", name = name), None),
        Account::SignedIn(None) => (tr!("Signed in").to_string(), None),
        Account::SignedOut => (
            tr!("Not signed in").to_string(),
            Some(tr!("Open {client} and sign in there: Heronite never sees your password.", client = client.name())),
        ),
        Account::Unknown => (tr!("Sign-in not seen yet").to_string(), Some(tr!("Open {client} to sign in.", client = client.name()))),
    }
}

/// "Last synced just now", "... 5 min ago" or the date, for an RFC 3339 time.
pub(super) fn synced_text(at: &str, now: chrono::DateTime<chrono::Local>) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&chrono::Local);
    let minutes = now.signed_duration_since(t).num_minutes();
    let when = if minutes < 1 {
        tr!("just now").to_string()
    } else if minutes < 60 {
        tr!("{n} min ago", n = minutes)
    } else {
        t.format("%Y-%m-%d %H:%M").to_string()
    };
    Some(tr!("Last synced {when}", when = when))
}

/// `path` with the home folder written `~`.
pub(super) fn tilde(path: &Path, home: &str) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.is_empty() && home != "/" => {
            if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.to_string_lossy())
            }
        }
        _ => path.to_string_lossy().into_owned(),
    }
}

/// The Collection's key of a game the client installed (a sideload entry).
pub(super) fn collection_key(client: Client, game_id: &str) -> String {
    format!("sideload_{}", heronite_clients::collection::app_name(client, game_id))
}

/// The line about the client's anti-cheat and the like, in the About card.
pub(super) fn client_note(client: Client) -> &'static str {
    match client {
        Client::BattleNet => tr!("Call of Duty uses the Ricochet anti-cheat, which does not run on Linux."),
        Client::Ea => {
            tr!("Battlefield 6, Battlefield 2042, Apex Legends and EA SPORTS FC use anti-cheats that block Linux.")
        }
        Client::Ubisoft => tr!("Rainbow Six Siege blocks Linux. Some games need another Proton: change it in Wine settings."),
        Client::Rockstar => tr!("GTA Online and Red Dead Online are blocked by BattlEye on Linux. The story modes work."),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use std::path::Path;

    use heronite_clients::setup::Step;
    use heronite_clients::{Account, Status};
    use heronite_data::clients::Client;
    use heronite_i18n::tr;

    use super::{
        account_lines, main_action, pill, step_percent, step_text, synced_text, tilde, Activity, ClientState, Pill, Progress, WineSetup,
    };

    fn installed() -> ClientState {
        let mut st = ClientState::default();
        st.status = Some(Status { prefix: Some("/p".into()), exe: Some("/p/x.exe".into()), ..Default::default() });
        st
    }

    #[test]
    fn a_client_goes_from_not_set_up_to_ready_to_running() {
        let mut st = ClientState::default();
        assert_eq!(pill(&st), Pill::NotSetUp);
        st.status = Some(Status::default());
        let m = main_action(Client::Ea, &st);
        assert_eq!((m.id, m.enabled), ("setup", true));
        let mut st = installed();
        assert_eq!(pill(&st), Pill::Ready);
        assert_eq!(main_action(Client::Ea, &st).id, "open");
        st.running = true;
        assert_eq!(pill(&st), Pill::Running);
        assert_eq!(main_action(Client::Ea, &st).id, "close");
    }

    #[test]
    fn nothing_is_offered_before_the_first_look() {
        let m = main_action(Client::Ubisoft, &ClientState::default());
        assert_eq!((m.id, m.enabled), ("setup", false));
    }

    #[test]
    fn a_running_game_keeps_the_client_open_and_says_why() {
        let mut st = installed();
        st.running = true;
        st.game_running = true;
        let m = main_action(Client::BattleNet, &st);
        assert_eq!((m.id, m.enabled), ("close", false));
        assert!(m.why.is_some());
    }

    #[test]
    fn a_setup_offers_cancel_until_it_is_asked_to_stop() {
        let mut st = ClientState::default();
        st.activity = Activity::SettingUp(Progress::Step(Step::Installing));
        let m = main_action(Client::Rockstar, &st);
        assert_eq!((m.id, m.enabled), ("cancel", true));
        assert_eq!(pill(&st), Pill::SettingUp);
        let mut busy = installed();
        busy.activity = Activity::Closing;
        assert!(!main_action(Client::Rockstar, &busy).enabled);
    }

    #[test]
    fn the_setup_text_follows_the_step() {
        let ea = Client::Ea;
        let say = |p: Progress| step_text(ea, &p, false);
        assert!(say(Progress::Step(Step::Downloading(Some(41.6)))).contains("42"));
        assert!(!say(Progress::Step(Step::Downloading(None))).contains('%'));
        assert!(say(Progress::Step(Step::Installing)).contains("EA app"));
        assert_eq!(step_percent(&Progress::Step(Step::Downloading(Some(10.)))), Some(10.));
        assert_eq!(step_percent(&Progress::Step(Step::Installing)), None);
        let wine = Progress::Wine(WineSetup::Downloading { version: "GE".into(), percentage: 7. });
        assert!(say(wine.clone()).contains('7'));
        assert_eq!(step_percent(&wine), Some(7.));
        assert_eq!(step_text(ea, &wine, true), tr!("Cancelling\u{2026}"));
    }

    #[test]
    fn the_account_lines_say_what_to_do() {
        let (title, hint) = account_lines(Client::Ea, &Account::SignedIn(Some("ana".into())));
        assert!(title.contains("ana") && hint.is_none());
        assert!(account_lines(Client::Ea, &Account::SignedOut).1.unwrap().contains("EA app"));
        assert!(account_lines(Client::Ea, &Account::Unknown).1.unwrap().contains("EA app"));
    }

    #[test]
    fn the_sync_time_is_relative_for_an_hour_and_a_date_after() {
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap();
        let at = |minutes: i64| (now - chrono::Duration::minutes(minutes)).to_rfc3339();
        assert!(synced_text(&at(0), now).unwrap().contains(tr!("just now")));
        assert!(synced_text(&at(5), now).unwrap().contains('5'));
        assert!(synced_text(&at(120), now).unwrap().contains("2026-10-26 10:00"));
        assert_eq!(synced_text("yesterday", now), None);
    }

    #[test]
    fn paths_under_home_are_written_with_a_tilde() {
        assert_eq!(tilde(Path::new("/home/ana/Games/x"), "/home/ana"), "~/Games/x");
        assert_eq!(tilde(Path::new("/home/ana"), "/home/ana"), "~");
        assert_eq!(tilde(Path::new("/mnt/x"), "/home/ana"), "/mnt/x");
        assert_eq!(tilde(Path::new("/mnt/x"), ""), "/mnt/x");
    }
}
