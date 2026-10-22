//! Mods in the Collection: opens the manager, the FOMOD wizard and the game
//! picker, and turns the mods service's events into dialogs and toasts. The
//! work itself is in `services::mods` and `dialogs::mods`.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::{CollectionView, ToastAction};
use crate::dialogs::mod_installer::{self, ModInstallerDialog};
use crate::dialogs::mod_link_game::ModLinkGameDialog;
use crate::dialogs::mods::{ModsDialog, ModsDialogEvent, Tab};
use crate::dialogs::DialogEvent;
use crate::services::mods::{self, LinkRequest, ModsEvent};

#[derive(Default)]
pub(super) struct ModsUi {
    /// Subscription to the manager's own requests.
    manager_sub: Option<Subscription>,
    /// Subscription to the wizard or picker that replaced the manager.
    nested_sub: Option<Subscription>,
    /// The game whose manager was open when the wizard or picker came up.
    reopen: Option<String>,
    counts_asked: bool,
}

impl CollectionView {
    /// Listens to the service (once, when the view is created).
    pub(super) fn init_mods(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let service = mods::service(cx);
        cx.subscribe_in(&service, window, |this, _, ev: &ModsEvent, window, cx| this.on_mods_event(ev, window, cx)).detach();
    }

    /// The Focus panel's "Mods" row needs the counts: asked once the library is here.
    pub(super) fn mods_library_loaded(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.mods.counts_asked, true) {
            return;
        }
        mods::service(cx).update(cx, |s, cx| s.load_counts(cx));
    }

    /// "12 · 10 on" for the facts card, when the game has mods.
    pub(super) fn mods_counts_text(&self, key: &str, cx: &App) -> Option<String> {
        let (total, on) = mods::started(cx)?.read(cx).counts(key)?;
        Some(tr!("{total} \u{b7} {on} on", total, on))
    }

    pub(super) fn open_mods(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_mods_tab(key, Tab::Installed, window, cx);
    }

    pub(super) fn open_mods_tab(&mut self, key: &str, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ctx), Some(game)) = (self.dialog_ctx(), self.lib.as_ref().and_then(|l| l.get(key)).cloned()) else { return };
        let running = self.playing.contains_key(&game.app_name);
        let view = cx.new(|cx| ModsDialog::new(ctx, game, tab, running, window, cx));
        self.mods.manager_sub = Some(cx.subscribe_in(&view, window, |this, _, ev: &ModsDialogEvent, window, cx| match ev {
            ModsDialogEvent::OpenSettings => {
                this.dialog = None;
                this.dialog_sub = None;
                // The Settings section for Mods is added with the Settings page of mods: use its `Section::Mods` here.
                this.open_settings_section(crate::pages::settings::Section::General, window, cx);
            }
            ModsDialogEvent::OpenInstaller => this.open_installer(window, cx),
        }));
        self.open_dialog(view, cx);
    }

    /// The manager open now, if that is the dialog on screen.
    fn mods_dialog(&self) -> Option<Entity<ModsDialog>> {
        self.dialog.clone()?.downcast::<ModsDialog>().ok()
    }

    /// Tells the open manager whether its game is running.
    pub(super) fn sync_mods_running(&self, cx: &mut Context<Self>) {
        let Some(d) = self.mods_dialog() else { return };
        let running = self.playing.contains_key(d.read(cx).app_name());
        d.update(cx, |d, cx| d.set_running(running, cx));
    }

    fn on_mods_event(&mut self, ev: &ModsEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            ModsEvent::Changed { .. } => cx.notify(),
            ModsEvent::Fomod => self.open_installer(window, cx),
            ModsEvent::PickGame => {
                let Some(req) = mods::service(cx).read(cx).link_request().cloned() else { return };
                self.open_link_picker(req, window, cx);
            }
            ModsEvent::OpenManager { game_key } => {
                self.select(game_key.clone().into(), true);
                self.reveal_selected();
                self.open_mods(game_key, window, cx);
            }
            ModsEvent::Notice { game_key, message, error } => {
                let manager_open = self.mods_dialog().is_some_and(|d| d.read(cx).game_key() == game_key);
                self.show_toast(message.clone(), *error, cx);
                if !error && !manager_open {
                    self.toast_action = Some(ToastAction::OpenMods(game_key.clone()));
                }
            }
            ModsEvent::Jobs | ModsEvent::Account => {}
        }
    }

    /// The FOMOD wizard for the installer the service holds; the manager
    /// comes back when it ends.
    fn open_installer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(req), Some(ctx)) = (mods::service(cx).read(cx).fomod().cloned(), self.dialog_ctx()) else { return };
        self.open_installer_request(req, ctx, window, cx);
    }

    fn open_installer_request(
        &mut self,
        req: mods::FomodRequest,
        ctx: crate::dialogs::DialogCtx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_manager(cx);
        let view = cx.new(|cx| ModInstallerDialog::new(ctx, req, cx));
        self.reopen_manager_when_closed(&view, window, cx);
        self.open_dialog(view, cx);
    }

    fn open_link_picker(&mut self, req: LinkRequest, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ctx) = self.dialog_ctx() else { return };
        self.remember_manager(cx);
        let view = cx.new(|cx| ModLinkGameDialog::new(ctx, req, window, cx));
        self.reopen_manager_when_closed(&view, window, cx);
        self.open_dialog(view, cx);
    }

    /// Notes the manager on screen, so the dialog that takes its place can give it back.
    fn remember_manager(&mut self, cx: &App) {
        if let Some(d) = self.mods_dialog() {
            self.mods.reopen = Some(d.read(cx).game_key().to_string());
        }
    }

    fn reopen_manager_when_closed<V: 'static + EventEmitter<DialogEvent>>(
        &mut self,
        view: &Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mods.nested_sub = Some(cx.subscribe_in(view, window, |this, _, ev: &DialogEvent, window, cx| {
            if !matches!(ev, DialogEvent::Close | DialogEvent::ChangedAndClose) {
                return;
            }
            // `open_dialog`'s own handler closes the dialog; the manager opens after it.
            if let Some(key) = this.mods.reopen.take() {
                cx.defer_in(window, move |this, window, cx| this.open_mods(&key, window, cx));
            }
        }));
    }

    /// `HERONITE_DEBUG_DIALOG=mods:<game key>[:<tab>]`, `mod-installer[:<steps>]`
    /// (the installer folder in `HERONITE_DEBUG_FOMOD`) and `mod-link`.
    pub(super) fn mods_debug_dialog(&mut self, which: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(rest) = which.strip_prefix("mods:") {
            let (key, tab) = match rest.rsplit_once(':') {
                Some((k, t)) if Tab::parse(t).is_some() => (k, Tab::parse(t).unwrap()),
                _ => (rest, Tab::Installed),
            };
            self.open_mods_tab(key, tab, window, cx);
            return true;
        }
        if let Some(rest) = which.strip_prefix("mod-installer") {
            let Some(dir) = crate::env::value("HERONITE_DEBUG_FOMOD") else {
                log::warn!("mod-installer: set HERONITE_DEBUG_FOMOD to a folder holding fomod/ModuleConfig.xml");
                return true;
            };
            match (mod_installer::debug_request(std::path::Path::new(&dir), "debug"), self.dialog_ctx()) {
                (Ok(req), Some(ctx)) => {
                    // `mod-installer:3` shows the third step.
                    let steps = rest.trim_start_matches(':').parse::<usize>().unwrap_or(1).saturating_sub(1);
                    for _ in 0..steps {
                        if let Ok(mut w) = req.wizard.lock() {
                            w.next();
                        }
                    }
                    self.open_installer_request(req, ctx, window, cx);
                }
                (Err(e), _) => log::warn!("mod-installer: {e}"),
                _ => {}
            }
            return true;
        }
        if which == "mod-link" {
            let candidates = self
                .lib
                .as_ref()
                .map(|l| l.games.iter().filter(|g| g.installed).take(2).map(|g| g.key.clone()).collect())
                .unwrap_or_default();
            let req = LinkRequest {
                id: 0,
                link: "nxm://skyrimspecialedition/mods/3863/files/1".into(),
                site_game: Some("Skyrim Special Edition".into()),
                candidates,
            };
            self.open_link_picker(req, window, cx);
            return true;
        }
        false
    }
}
