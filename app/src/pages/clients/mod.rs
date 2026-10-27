//! The Battle.net, EA app, Ubisoft Connect and Rockstar Games pages of the
//! Stores: the store's own program, run by Wine in a prefix of its own
//! (`services::clients`). The page sets it up, opens and closes it, lists
//! the games it installed (they come into the Collection by themselves) and
//! opens its Wine settings. One view serves the four; the client is a type
//! parameter so each page has its own palette commands.

mod console;
mod logic;
mod palette;
mod render;

use std::marker::PhantomData;
use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::clients::Client;

use super::{Page, PageCtx, PageEvent};
use crate::dialogs::client_settings::ClientSettingsDialog;
use crate::dialogs::DialogEvent;
use crate::services::clients::{debug_client, Clients};

/// Picks the client a [`ClientPage`] shows.
pub trait ClientKind: 'static {
    const CLIENT: Client;
    const PAGE: Page;
}

pub struct BattleNetKind;
pub struct EaKind;
pub struct UbisoftKind;
pub struct RockstarKind;

impl ClientKind for BattleNetKind {
    const CLIENT: Client = Client::BattleNet;
    const PAGE: Page = Page::BattleNet;
}

impl ClientKind for EaKind {
    const CLIENT: Client = Client::Ea;
    const PAGE: Page = Page::Ea;
}

impl ClientKind for UbisoftKind {
    const CLIENT: Client = Client::Ubisoft;
    const PAGE: Page = Page::Ubisoft;
}

impl ClientKind for RockstarKind {
    const CLIENT: Client = Client::Rockstar;
    const PAGE: Page = Page::Rockstar;
}

pub type BattleNetPage = ClientPage<BattleNetKind>;
pub type EaPage = ClientPage<EaKind>;
pub type UbisoftPage = ClientPage<UbisoftKind>;
pub type RockstarPage = ClientPage<RockstarKind>;

pub struct ClientPage<K: ClientKind> {
    ctx: PageCtx,
    svc: Entity<Clients>,
    /// "Remove from Heronite" asked and waiting for the answer.
    confirm_forget: bool,
    /// Name of the Wine build the client runs with (`None` while read).
    wine: Option<String>,
    /// The prefix `wine` was read for.
    wine_for: Option<PathBuf>,
    dialog: Option<AnyView>,
    dialog_sub: Option<Subscription>,
    /// Console mode: focused row and the open sheet / browser.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
    kind: PhantomData<K>,
}

impl<K: ClientKind> EventEmitter<PageEvent> for ClientPage<K> {}

impl<K: ClientKind> ClientPage<K> {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = crate::services::clients(cx);
        cx.observe(&svc, |this, _, cx| {
            this.load_wine(cx);
            cx.notify();
        })
        .detach();
        super::stores::watch(cx);
        // A window closed to the tray takes its pages without hiding them
        // first: the client is not asked whether it runs any more.
        cx.on_release(|this, cx| this.svc.update(cx, |s, cx| s.set_visible(K::CLIENT, false, cx))).detach();
        let mut this = Self {
            ctx,
            svc,
            confirm_forget: false,
            wine: None,
            wine_for: None,
            dialog: None,
            dialog_sub: None,
            console_list: Default::default(),
            layer: Default::default(),
            kind: PhantomData,
        };
        // Screenshots: see `services::clients::debug`.
        match debug_client().filter(|(c, _)| *c == K::CLIENT).as_ref().map(|(_, s)| s.as_str()) {
            Some("forget") => this.confirm_forget = true,
            Some("settings") => this.open_settings(window, cx),
            _ => {}
        }
        this
    }

    /// The page came on screen or went away: the client is only asked
    /// whether it runs while it is on screen.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.set_visible(K::CLIENT, shown, cx));
        if shown {
            self.load_wine(cx);
        } else {
            self.confirm_forget = false;
        }
    }

    /// Reads the name of the client's Wine build when its prefix changed.
    fn load_wine(&mut self, cx: &mut Context<Self>) {
        let prefix = self.svc.read(cx).state(K::CLIENT).prefix().map(|p| p.to_path_buf());
        if prefix.is_none() || prefix == self.wine_for {
            return;
        }
        self.wine_for = prefix;
        self.read_wine(cx);
    }

    fn read_wine(&mut self, cx: &mut Context<Self>) {
        let paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let name = cx
                .background_spawn(async move {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
                    let settings = heronite_data::game_config::load_game(&paths, K::CLIENT.config_app(), &home);
                    crate::dialogs::settings_panel::logic::display_wine_name(
                        &crate::dialogs::settings_panel::logic::wine_of(&settings).name,
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                this.wine = Some(name);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_dialog<V>(&mut self, view: Entity<V>, cx: &mut Context<Self>)
    where
        V: Render + EventEmitter<DialogEvent>,
    {
        self.dialog_sub = Some(cx.subscribe(&view, |this, _, ev: &DialogEvent, cx| {
            if matches!(ev, DialogEvent::Close | DialogEvent::ChangedAndClose) {
                this.dialog = None;
                this.dialog_sub = None;
                // The settings may have moved the client to another Wine or prefix.
                this.read_wine(cx);
                this.svc.update(cx, |s, cx| s.look(cx));
            }
            cx.notify();
        }));
        self.dialog = Some(view.into());
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.ctx.data_paths.clone();
        let dialog = cx.new(|cx| ClientSettingsDialog::new(paths, K::CLIENT, window, cx));
        self.open_dialog(dialog, cx);
    }

    /// Opens the prefix folder in the file manager.
    fn open_prefix_folder(&self, cx: &App) {
        if let Some(prefix) = self.svc.read(cx).state(K::CLIENT).prefix() {
            crate::ui::open_external(&prefix.to_string_lossy());
        }
    }

    /// Uses the client installed in the prefix at `picked` (a folder).
    fn use_existing(&mut self, picked: PathBuf, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.adopt(K::CLIENT, picked, cx));
    }

    fn choose_existing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::ui::fields::choose_path(window, cx, true, heronite_i18n::tr!("Wine prefix"), |this: &mut Self, path, _, cx| {
            this.use_existing(PathBuf::from(path), cx);
        });
    }

    /// Runs what a button of the page (or a console row) asks.
    fn run_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let client = K::CLIENT;
        match id {
            "setup" => self.svc.update(cx, |s, cx| s.install(client, cx)),
            "cancel" => self.svc.update(cx, |s, cx| s.cancel_install(client, cx)),
            "open" => self.svc.update(cx, |s, cx| s.open(client, cx)),
            "close" => self.svc.update(cx, |s, cx| s.close(client, cx)),
            "sync" => self.svc.update(cx, |s, cx| s.sync(client, cx)),
            "settings" => self.open_settings(window, cx),
            "folder" => self.open_prefix_folder(cx),
            "adopt" => self.choose_existing(window, cx),
            "forget" => self.confirm_forget = true,
            "forget-no" => self.confirm_forget = false,
            "forget-yes" => {
                self.confirm_forget = false;
                self.svc.update(cx, |s, cx| s.forget(client, cx));
            }
            _ => {}
        }
        cx.notify();
    }

    /// Selects `game_id` of this client on the Collection.
    fn show_game(&mut self, game_id: &str, cx: &mut Context<Self>) {
        let key = logic::collection_key(K::CLIENT, game_id);
        cx.emit(PageEvent::ShowGame(key));
    }
}
