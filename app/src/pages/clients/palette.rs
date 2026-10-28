//! Command palette commands of the client pages (see `pages/commands.rs`):
//! set the client up, open it, close it, refresh its games.

use gpui_kit::*;
use heronite_data::clients::Client;
use heronite_i18n::tr;

use super::{ClientKind, ClientPage};
use crate::icons::Icon;
use crate::pages::commands::{close_dialog, CommandCtx, PageCommand, PaletteCommands};
use crate::services::clients::{Activity, ClientState};

/// Extra search words (English and Portuguese).
fn keywords(client: Client) -> &'static str {
    match client {
        Client::BattleNet => "battle.net battlenet blizzard wow overwatch diablo loja entrar conta jogos",
        Client::Ea => "ea app origin electronic arts loja entrar conta jogos",
        Client::Ubisoft => "ubisoft connect uplay loja entrar conta jogos",
        Client::Rockstar => "rockstar games launcher social club gta red dead loja entrar conta jogos",
    }
}

/// Why `st` is busy, for a command that cannot run meanwhile.
fn busy_reason(st: &ClientState) -> Option<&'static str> {
    match st.activity {
        Activity::Idle => None,
        Activity::SettingUp(_) => Some(tr!("Setting up\u{2026}")),
        Activity::Opening => Some(tr!("Opening\u{2026}")),
        Activity::Closing => Some(tr!("Closing\u{2026}")),
        Activity::Adopting | Activity::Forgetting => Some(tr!("Working\u{2026}")),
    }
}

/// The commands of `client`; `st` is `None` before anything is known.
pub(super) fn commands(client: Client, st: Option<&ClientState>) -> Vec<PageCommand> {
    let name = client.name();
    let default = ClientState::default();
    let st = st.unwrap_or(&default);
    let first = || (!st.installed()).then(|| tr!("Set up {client} first", client = name));
    let busy = || busy_reason(st).map(str::to_string);
    vec![
        PageCommand::new(
            "setup",
            tr!("Set up {client}", client = name),
            tr!("Download its installer and install it with Wine"),
            Icon::Download,
        )
        .keywords(keywords(client))
        .disabled(
            busy()
                .or_else(|| st.installed().then(|| tr!("Already set up").to_string()))
                .or_else(|| st.status.is_none().then(|| tr!("Looking\u{2026}").to_string())),
        ),
        PageCommand::new("open", tr!("Open {client}", client = name), tr!("Sign in and install games"), Icon::OpenInNew)
            .keywords(keywords(client))
            .disabled(first().or_else(busy)),
        PageCommand::new("close", tr!("Close {client}", client = name), tr!("Closes it and what runs in its Wine prefix"), Icon::Stop)
            .keywords(keywords(client))
            .disabled(
                first()
                    .or_else(busy)
                    .or_else(|| (!st.running).then(|| tr!("{client} is not running", client = name)))
                    .or_else(|| st.game_running.then(|| tr!("A game is running: close it first.").to_string())),
            )
            .stay(),
        PageCommand::new(
            "sync",
            tr!("Refresh {client} games", client = name),
            tr!("Bring its installed games into the Collection"),
            Icon::Refresh,
        )
        .keywords(keywords(client))
        .disabled(first().or_else(busy).or_else(|| st.syncing.then(|| tr!("Refreshing\u{2026}").to_string())))
        .stay(),
    ]
}

impl<K: ClientKind> PaletteCommands for ClientPage<K> {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let svc = crate::services::clients_started(cx);
        let st = svc.as_ref().map(|s| s.read(cx).state(K::CLIENT));
        commands(K::CLIENT, st)
    }

    fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.run_action(id, window, cx);
    }

    fn page_modal(&self) -> bool {
        self.dialog.is_some()
    }

    fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let closed = close_dialog(&mut self.dialog, &mut self.dialog_sub, window, cx);
        cx.notify();
        closed
    }
}

#[cfg(test)]
mod tests {
    use heronite_clients::Status;
    use heronite_data::clients::Client;

    use super::{commands, ClientState, PageCommand};

    fn reason<'a>(cmds: &'a [PageCommand], id: &str) -> Option<&'a str> {
        cmds.iter().find(|c| c.id == id).and_then(|c| c.disabled.as_deref())
    }

    fn installed() -> ClientState {
        let mut st = ClientState::default();
        st.status = Some(Status { prefix: Some("/p".into()), exe: Some("/p/x.exe".into()), ..Default::default() });
        st
    }

    #[test]
    fn a_client_not_set_up_can_only_be_set_up() {
        let mut st = ClientState::default();
        st.status = Some(Status::default());
        let cmds = commands(Client::Ea, Some(&st));
        assert_eq!(reason(&cmds, "setup"), None);
        for id in ["open", "close", "sync"] {
            assert!(reason(&cmds, id).is_some_and(|r| r.contains("EA app")), "{id}");
        }
    }

    #[test]
    fn a_client_that_is_set_up_can_be_opened_and_refreshed() {
        let cmds = commands(Client::Ubisoft, Some(&installed()));
        assert!(reason(&cmds, "setup").is_some());
        assert_eq!(reason(&cmds, "open"), None);
        assert_eq!(reason(&cmds, "sync"), None);
        assert!(reason(&cmds, "close").is_some(), "it does not run");
    }

    #[test]
    fn a_running_client_closes_unless_a_game_runs() {
        let mut st = installed();
        st.running = true;
        assert_eq!(reason(&commands(Client::Rockstar, Some(&st)), "close"), None);
        st.game_running = true;
        assert!(reason(&commands(Client::Rockstar, Some(&st)), "close").is_some());
    }

    #[test]
    fn while_the_service_is_not_there_nothing_is_set_up_yet() {
        let cmds = commands(Client::BattleNet, None);
        assert!(reason(&cmds, "setup").is_some(), "not looked at yet");
        assert!(reason(&cmds, "open").is_some());
    }

    #[test]
    fn every_command_is_found_by_the_clients_brand() {
        for client in Client::ALL {
            assert!(commands(client, None).iter().all(|c| !c.keywords.is_empty()));
        }
    }
}
