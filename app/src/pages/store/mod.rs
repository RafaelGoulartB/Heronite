//! GOG and Amazon Games pages: log in (the embedded login window, or the
//! browser plus the pasted code), account, library refresh and importing an
//! installed game. One view serves both stores; the store is a type
//! parameter so each page has its own palette commands.

mod console;
mod palette;
mod render;

use std::marker::PhantomData;
use std::process::{Command, Stdio};

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trc};

use super::{PageCtx, PageEvent};
use crate::dialogs::store_import::StoreImportPicker;
use crate::dialogs::store_install::{StoreInstallDialog, StoreInstallMode};
use crate::dialogs::DialogEvent;
use crate::services::downloads::Store;
use crate::services::{self, Downloads};
use crate::ui::fields::new_input;

/// Picks the store a [`StorePage`] shows.
pub trait StoreKind: 'static {
    const STORE: Store;
}

pub struct GogStore;
pub struct AmazonStore;

impl StoreKind for GogStore {
    const STORE: Store = Store::Gog;
}

impl StoreKind for AmazonStore {
    const STORE: Store = Store::Amazon;
}

pub type GogPage = StorePage<GogStore>;
pub type AmazonPage = StorePage<AmazonStore>;

/// What to do with the store's sign-in page address once it is known.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    Window,
    Browser,
    CopyLink,
}

pub struct StorePage<K: StoreKind> {
    ctx: PageCtx,
    svc: Entity<Downloads>,
    code: Entity<InputState>,
    confirm_logout: bool,
    /// The embedded login window is open.
    window_open: bool,
    /// Amazon: nile is preparing the sign-in page.
    fetching_url: bool,
    /// Amazon: the sign-in page made by nile. Its verifier lives in the
    /// service, so the same address is reused until a code is submitted.
    login_url: Option<String>,
    /// Amazon: the Amazon Games app has games on this PC (Windows).
    amazon_app: bool,
    /// GOG: GOG Galaxy has games on this PC (Windows).
    galaxy: bool,
    notice: Option<(SharedString, bool)>,
    dialog: Option<AnyView>,
    dialog_sub: Option<Subscription>,
    /// Console mode: focused row and the open sheet / keyboard.
    console_list: crate::console::list::ListNav,
    layer: crate::console::Layer,
    kind: PhantomData<K>,
}

impl<K: StoreKind> EventEmitter<PageEvent> for StorePage<K> {}

/// `heronite-epic-login` (the login window of every store) next to this
/// executable, else on `PATH`.
pub(crate) fn login_helper() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "heronite-epic-login.exe" } else { "heronite-epic-login" };
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.exists()))
}

/// Exit status of the login window on Windows when the Microsoft Edge
/// WebView2 runtime it shows the page with is not installed.
pub(crate) const LOGIN_NO_WEBVIEW: i32 = 5;

/// What to tell the user then: install the runtime, or log in with the
/// browser.
pub(crate) fn no_webview_notice() -> SharedString {
    tr!("The login window needs the Microsoft Edge WebView2 Runtime, which is not installed. Install it from Microsoft's website, or use the browser method below.").into()
}

/// "1 installed" / "3 installed" (Portuguese inflects it).
pub(crate) fn installed_count(n: usize) -> String {
    if n == 1 {
        trc!("one", "{n} installed", n)
    } else {
        trc!("other", "{n} installed", n)
    }
}

impl<K: StoreKind> StorePage<K> {
    pub fn new(ctx: PageCtx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let svc = services::downloads(cx);
        cx.observe(&svc, |this, svc, cx| {
            if svc.read(cx).store(K::STORE).account.logged_in {
                this.login_url = None;
            }
            cx.notify();
        })
        .detach();
        super::stores::watch(cx);
        let code = new_input(window, cx, "", tr!("Paste the address of the page or its code"));
        let mut this = Self {
            ctx,
            svc,
            code,
            confirm_logout: false,
            window_open: false,
            fetching_url: false,
            login_url: None,
            amazon_app: K::STORE == Store::Amazon && heronite_amazon::amazon_app::database().is_some(),
            galaxy: K::STORE == Store::Gog && heronite_gog::galaxy::has_games(),
            notice: None,
            dialog: None,
            dialog_sub: None,
            console_list: Default::default(),
            layer: Default::default(),
            kind: PhantomData,
        };
        // Screenshots: `HERONITE_DEBUG_STORE_PAGE=import|logout`.
        match crate::env::value("HERONITE_DEBUG_STORE_PAGE").as_deref() {
            Some("import") => this.open_import(window, cx),
            Some("logout") => this.confirm_logout = true,
            _ => {}
        }
        this
    }

    fn store(&self) -> Store {
        K::STORE
    }

    /// The page of this store.
    fn page(&self) -> crate::pages::Page {
        match K::STORE {
            Store::Gog => crate::pages::Page::Gog,
            Store::Amazon => crate::pages::Page::Amazon,
        }
    }

    fn open_dialog<V>(&mut self, view: Entity<V>, window: &mut Window, cx: &mut Context<Self>)
    where
        V: Render + EventEmitter<DialogEvent>,
    {
        self.dialog_sub = Some(cx.subscribe_in(&view, window, |this, view, ev: &DialogEvent, window, cx| {
            if matches!(ev, DialogEvent::Close | DialogEvent::ChangedAndClose) {
                this.dialog = None;
                this.dialog_sub = None;
                // The import picker hands over the chosen game.
                if let Ok(picker) = view.clone().into_any().downcast::<StoreImportPicker>() {
                    if let Some(game) = picker.update(cx, |p, _| p.picked.take()) {
                        let (data_paths, store) = (this.ctx.data_paths.clone(), this.store());
                        let dlg = cx.new(|cx| StoreInstallDialog::new(data_paths, store, game, StoreInstallMode::Import, window, cx));
                        this.open_dialog(dlg, window, cx);
                    }
                }
            }
            cx.notify();
        }));
        self.dialog = Some(view.into());
        cx.notify();
    }

    /// Amazon: brings in the games the Amazon Games app installed.
    fn import_app_games(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.amazon_import_app_games(false, cx));
    }

    fn import_galaxy_games(&mut self, cx: &mut Context<Self>) {
        self.svc.update(cx, |s, cx| s.gog_import_galaxy(cx));
    }

    fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (data_paths, store) = (self.ctx.data_paths.clone(), self.store());
        let picker = cx.new(|cx| StoreImportPicker::new(data_paths, store, window, cx));
        self.open_dialog(picker, window, cx);
    }

    /// The login window helper, or a notice that points to the browser method.
    fn helper_or_notice(&mut self, cx: &mut Context<Self>) -> Option<std::path::PathBuf> {
        let helper = login_helper();
        if helper.is_none() {
            self.notice = Some((
                tr!("The login window (heronite-epic-login) was not found next to the app. Use the browser method below.").into(),
                true,
            ));
            cx.notify();
        }
        helper
    }

    /// Opens the login window (the store's own page, typing the password
    /// needs a keyboard).
    fn start_window_login(&mut self, cx: &mut Context<Self>) {
        if self.helper_or_notice(cx).is_none() {
            return;
        }
        self.with_login_url(Then::Window, cx);
    }

    /// Hands `then` the sign-in page address. Amazon's is made by nile, which
    /// also remembers the verifier the code is checked against, so the
    /// browser steps reuse the address they already have.
    fn with_login_url(&mut self, then: Then, cx: &mut Context<Self>) {
        let store = self.store();
        if store == Store::Gog {
            self.url_ready(Ok(Some(heronite_gog::user::LOGIN_URL.to_string())), then, cx);
            return;
        }
        if then != Then::Window {
            if let Some(url) = self.login_url.clone() {
                self.url_ready(Ok(Some(url)), then, cx);
                return;
            }
        }
        if self.fetching_url {
            return;
        }
        self.fetching_url = true;
        self.notice = None;
        cx.notify();
        let page = cx.entity().downgrade();
        self.svc.update(cx, |s, cx| {
            s.amazon_begin_login(cx, move |_, res, cx| {
                page.update(cx, |this, cx| {
                    this.fetching_url = false;
                    this.url_ready(res, then, cx);
                })
                .ok();
            })
        });
    }

    fn url_ready(&mut self, res: Result<Option<String>, String>, then: Then, cx: &mut Context<Self>) {
        match res {
            Ok(Some(url)) => {
                if self.store() == Store::Amazon {
                    self.login_url = Some(url.clone());
                }
                match then {
                    // GOG's window knows its own page.
                    Then::Window => self.run_login_window((self.store() == Store::Amazon).then_some(url), cx),
                    Then::Browser => self.open_browser(&url, cx),
                    Then::CopyLink => cx.write_to_clipboard(ClipboardItem::new_string(url)),
                }
            }
            // Already logged in: the service reloads the account.
            Ok(None) => self.login_url = None,
            Err(err) => self.notice = Some((err.lines().next().unwrap_or("").to_string().into(), true)),
        }
        cx.notify();
    }

    /// Runs `heronite-epic-login --store <store> [--url <url>]` and logs in
    /// with the code it prints.
    fn run_login_window(&mut self, url: Option<String>, cx: &mut Context<Self>) {
        let Some(helper) = self.helper_or_notice(cx) else { return };
        let store = self.store();
        self.window_open = true;
        self.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move {
                    // Guarded helper: closes with the app instead of being orphaned.
                    let mut cmd = Command::new(helper);
                    cmd.arg("--store").arg(if store == Store::Gog { "gog" } else { "amazon" });
                    if let Some(url) = &url {
                        cmd.arg("--url").arg(url);
                    }
                    cmd.stdin(Stdio::null());
                    heronite_data::proc::guard(&mut cmd).output()
                })
                .await;
            this.update(cx, |this, cx| {
                this.window_open = false;
                match out {
                    Ok(o) if o.status.success() => {
                        let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if code.is_empty() {
                            this.notice = Some((tr!("The login window closed without a code.").into(), true));
                        } else {
                            this.svc.update(cx, |s, cx| s.store_login(store, code, cx));
                        }
                    }
                    // 127: the loader could not start it (WebKitGTK 4.1 is missing).
                    Ok(o) if !cfg!(windows) && o.status.code() == Some(127) => {
                        this.notice = Some((
                            tr!("The login window needs WebKitGTK 4.1, which is not installed. Use the browser method below.").into(),
                            true,
                        ))
                    }
                    Ok(o) if cfg!(windows) && o.status.code() == Some(LOGIN_NO_WEBVIEW) => {
                        this.notice = Some((no_webview_notice(), true));
                    }
                    // `CLOSED` of the helper: the user closed the window.
                    Ok(o) if o.status.code() == Some(4) => {}
                    Ok(o) => {
                        log::warn!("the login window stopped ({}): {}", o.status, String::from_utf8_lossy(&o.stderr).trim());
                        this.notice = Some((tr!("The login window stopped unexpectedly. Use the browser method below.").into(), true));
                    }
                    Err(err) => this.notice = Some((tr!("Could not open the login window: {err}", err).into(), true)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_browser(&mut self, url: &str, cx: &mut Context<Self>) {
        if let Err(err) = heronite_platform::shell::open(url) {
            self.notice = Some((tr!("Could not open the browser: {err}", err).into(), true));
            cx.notify();
        }
    }

    /// Whether a pasted code can be used now (Amazon needs its sign-in page
    /// opened first: the code is checked against that request).
    fn can_submit(&self) -> bool {
        self.store() == Store::Gog || self.login_url.is_some()
    }

    fn submit_code(&mut self, text: String, cx: &mut Context<Self>) {
        if text.trim().is_empty() {
            return;
        }
        if !self.can_submit() {
            self.notice = Some((tr!("Open the Amazon sign-in page first").into(), true));
            cx.notify();
            return;
        }
        self.notice = None;
        let store = self.store();
        self.svc.update(cx, |s, cx| s.store_login(store, text, cx));
    }
}
