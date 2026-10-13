//! Command palette commands of the GOG / Amazon Games pages (see
//! `pages/commands.rs`): log in / out, refresh, update all, import.

use std::ops::Not;

use gpui_kit::*;
use heronite_i18n::{tr, trn};

use super::{login_helper, StoreKind, StorePage};
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};
use crate::services::downloads::Store;

/// Extra search words (English and Portuguese).
fn keywords(store: Store) -> &'static str {
    match store {
        Store::Gog => "gog galaxy gogdl account login library conta entrar biblioteca",
        Store::Amazon => "amazon prime gaming nile account login library conta entrar biblioteca",
    }
}

impl<K: StoreKind> PaletteCommands for StorePage<K> {
    fn palette_commands(_page: Option<&Self>, _ctx: &CommandCtx, cx: &App) -> Vec<PageCommand> {
        let store = K::STORE;
        let name = store.name();
        let Some(svc) = crate::services::downloads_started(cx) else { return Vec::new() };
        let state = svc.read(cx).store(store);
        let logged_in = state.account.logged_in;
        let first = || logged_in.not().then(|| tr!("Log in to {store} first", store = name));
        let updates = state.updates.len();
        let name_of = state.account.name.clone();
        vec![
            PageCommand::new("login", tr!("Log in to {store}", store = name), tr!("Opens the login window"), Icon::AccountCircle)
                .keywords(keywords(store))
                .disabled(if logged_in { Some(tr!("Already logged in")) } else { state.logging_in.then(|| tr!("Logging in...")) }),
            PageCommand::new(
                "refresh",
                tr!("Refresh the {store} library", store = name),
                tr!("Look for new games and updates"),
                Icon::Refresh,
            )
            .keywords(keywords(store))
            .disabled(first().or_else(|| state.refreshing.then(|| tr!("Refreshing...").to_string())))
            .stay(),
            PageCommand::new(
                "update-all",
                tr!("Update all {store} games", store = name),
                trn!(updates, "{n} update available", "{n} updates available"),
                Icon::Update,
            )
            .keywords(keywords(store))
            .disabled(first().or_else(|| (updates == 0).then(|| tr!("No updates available").to_string())))
            .stay(),
            PageCommand::new("import", tr!("Import a {store} game", store = name), tr!("Use a copy already on this PC"), Icon::FolderOpen)
                .keywords(keywords(store))
                .disabled(first()),
            PageCommand::new(
                "logout",
                tr!("Log out of {store}\u{2026}", store = name),
                if logged_in { tr!("Signed in as {name}", name = name_of) } else { tr!("Not logged in").to_string() },
                Icon::Logout,
            )
            .keywords(keywords(store))
            .disabled(logged_in.not().then(|| tr!("Not logged in"))),
        ]
    }

    fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store();
        match id {
            // Without the helper the page shows the browser method.
            "login" if login_helper().is_some() => self.start_window_login(cx),
            "logout" => self.confirm_logout = true,
            "refresh" => self.svc.update(cx, |s, cx| s.store_refresh(store, false, false, cx)),
            "update-all" => self.svc.update(cx, |s, cx| s.store_update_all(store, cx)),
            "import" => self.open_import(window, cx),
            _ => {}
        }
        cx.notify();
    }

    fn page_modal(&self) -> bool {
        self.dialog.is_some()
    }
}
