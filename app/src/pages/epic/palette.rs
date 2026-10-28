//! Command palette commands of the Epic Games page (see `pages/commands.rs`).

use gpui_kit::*;
use heronite_i18n::tr;

use super::{login_helper, EpicPage};
use crate::icons::Icon;
use crate::pages::commands::{close_dialog, CommandCtx, PageCommand, PaletteCommands};

impl PaletteCommands for EpicPage {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let Some(svc) = crate::services::downloads_started(cx) else { return Vec::new() };
        let s = svc.read(cx);
        let account = s.account();
        if !account.logged_in {
            return vec![PageCommand::new("login", tr!("Sign in to Epic Games"), tr!("Opens the login window"), Icon::AccountCircle)
                .keywords(tr!("log in login account legendary"))];
        }
        let eos = s.eos();
        let mut v = vec![PageCommand::new(
            "logout",
            tr!("Sign out of Epic Games\u{2026}"),
            tr!("Signed in as {name}", name = account.name),
            Icon::Logout,
        )
        .keywords(tr!("log out logout account disconnect"))];
        if eos.installed {
            v.push(
                PageCommand::new(
                    "eos-check",
                    tr!("Check for EOS Overlay updates"),
                    tr!("Version {version}", version = eos.version),
                    Icon::Update,
                )
                .keywords(tr!("epic online services overlay"))
                .disabled(eos.checking.then_some(tr!("Checking already")))
                .stay(),
            );
            v.push(
                PageCommand::new(
                    "eos-remove",
                    tr!("Remove the EOS Overlay\u{2026}"),
                    tr!("Uninstall it from every prefix"),
                    Icon::DeleteOutline,
                )
                .keywords(tr!("epic online services overlay uninstall")),
            );
        } else {
            v.push(
                PageCommand::new("eos-install", tr!("Install the EOS Overlay"), tr!("Epic's in-game overlay"), Icon::Download)
                    .keywords(tr!("epic online services overlay"))
                    .stay(),
            );
        }
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        match id {
            // Without the helper the page shows the browser method.
            "login" if login_helper().is_some() => self.start_window_login(cx),
            "logout" => self.confirm_logout = true,
            "eos-check" => self.svc.update(cx, |s, cx| s.eos_check_updates(cx)),
            "eos-install" => self.svc.update(cx, |s, cx| s.eos_install(cx)),
            "eos-remove" => self.confirm_eos_remove = true,
            _ => {}
        }
        cx.notify();
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
