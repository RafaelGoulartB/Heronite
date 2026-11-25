//! The "Install folder" card of a store's page (`heronite_data::store_folders`):
//! where the store's new games go. None is set at first. Epic Games, GOG and
//! Amazon Games then use the default install folder (Settings → General), as
//! the install dialog shows; a store client under Wine (Battle.net, the EA
//! app, Ubisoft Connect, Rockstar Games) its own folder, inside its prefix.
//! A client's folder is written into the client where it keeps one
//! (`heronite_clients::games_folder`), and the card shows how the client sees
//! it (`Z:\...`). Steam and the Xbox app install their games themselves.
//!
//! One view serves every page: the desktop card ([`Render`]), and the
//! console rows ([`StoreFolder::console_rows`]) whose actions the page runs
//! ([`StoreFolder::console_activate`]).

use std::path::{Path, PathBuf};

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::clients::Client;
use heronite_data::{game_config, store_folders, DataPaths};
use heronite_i18n::tr;

use crate::console::browser::PickKind;
use crate::console::layers::{open_browser, HasLayer};
use crate::console::list::Row;
use crate::icons::Icon;
use crate::pages::list_item;
use crate::theme;
use crate::ui::controls::{button_sm, caption, card, icon_button_sm, row, ButtonKind};
use crate::ui::fields::{choose_path, meta_text};
use crate::ui::icon;

/// Whose folder it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// A store Heronite installs for: its key in `store_folders` and its name.
    Store(&'static str, &'static str),
    Client(Client),
}

impl Owner {
    fn key(self) -> &'static str {
        match self {
            Owner::Store(key, _) => key,
            Owner::Client(c) => c.id(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Owner::Store(_, name) => name,
            Owner::Client(c) => c.name(),
        }
    }
}

/// What was read from the settings.
#[derive(Clone, Debug, Default, PartialEq)]
struct Folder {
    own: Option<String>,
    /// Where a store's games go without one: the default install folder.
    default: String,
    /// A client's folder as the client sees it (`Z:\data\Games\EA`).
    windows: Option<String>,
}

pub struct StoreFolder {
    paths: DataPaths,
    owner: Owner,
    /// `None` until read.
    folder: Option<Folder>,
    saving: bool,
    error: Option<String>,
    /// Changed while the client was open: it uses the new folder once it
    /// opens again (its container and settings are already set).
    reopen: bool,
}

fn home() -> String {
    // Windows has no `HOME` of its own: the user's folder.
    if cfg!(windows) {
        heronite_platform::dirs::home().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        std::env::var("HOME").unwrap_or_default()
    }
}

/// `path` with the home folder written `~`.
fn tilde(path: &str, home: &str) -> String {
    match Path::new(path).strip_prefix(home) {
        Ok(rest) if !home.is_empty() && home != "/" && !cfg!(windows) => {
            if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.to_string_lossy())
            }
        }
        _ => path.to_string(),
    }
}

/// Reads the folder of `owner` (a few small file reads: off the UI thread).
fn read(paths: &DataPaths, owner: Owner) -> Folder {
    let home = home();
    let global = game_config::load_global(paths, &home);
    let own = store_folders::own(&global, owner.key(), &home);
    let windows = match (owner, &own) {
        (Owner::Client(_), Some(f)) => Some(heronite_clients::games_folder::windows_path(Path::new(f))),
        _ => None,
    };
    Folder { own, default: store_folders::default_dir(&global, &home), windows }
}

impl StoreFolder {
    pub fn new(paths: DataPaths, owner: Owner, cx: &mut Context<Self>) -> Self {
        // Screenshots: `HERONITE_DEBUG_STORE_FOLDER=reopen` (with an open client).
        let reopen = crate::env::value("HERONITE_DEBUG_STORE_FOLDER").as_deref() == Some("reopen");
        let mut this = Self { paths, owner, folder: None, saving: false, error: None, reopen };
        this.reload(cx);
        this
    }

    /// Reads the folder again (the default one may have changed in Settings).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (paths, owner) = (self.paths.clone(), self.owner);
        cx.spawn(async move |this, cx| {
            let folder = cx.background_spawn(async move { read(&paths, owner) }).await;
            this.update(cx, |this, cx| {
                // A save in flight reads the folder again when it is done.
                if !this.saving && this.folder.as_ref() != Some(&folder) {
                    this.folder = Some(folder);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Chooses `folder` for the store (`None`: back to the default). A
    /// client's own settings follow.
    pub fn set(&mut self, folder: Option<String>, cx: &mut Context<Self>) {
        let (paths, owner) = (self.paths.clone(), self.owner);
        let old = self.folder.as_ref().and_then(|f| f.own.clone());
        self.saving = true;
        self.error = None;
        self.reopen = self.client_open(cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (saved, folder) = cx
                .background_spawn(async move {
                    let home = home();
                    let saved = store_folders::set(&paths, &home, owner.key(), folder.as_deref()).map_err(|e| format!("{e:#}"));
                    if let (Ok(()), Owner::Client(client)) = (&saved, owner) {
                        if let Some(old) = old.filter(|o| folder.as_deref() != Some(o)) {
                            heronite_clients::games_folder::release(&paths, client, &home, Path::new(&old));
                        }
                        heronite_clients::games_folder::apply(&paths, client, &home);
                    }
                    (saved, read(&paths, owner))
                })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                this.error = saved.err();
                this.folder = Some(folder);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Whether the store is a client that is open now.
    fn client_open(&self, cx: &App) -> bool {
        let Owner::Client(c) = self.owner else { return false };
        crate::services::clients_started(cx).is_some_and(|s| s.read(cx).state(c).running)
    }

    fn own(&self) -> Option<&str> {
        self.folder.as_ref().and_then(|f| f.own.as_deref())
    }

    /// The folder new games go to, when Heronite knows it (a client's own
    /// folder is the client's).
    fn current(&self) -> Option<&str> {
        let f = self.folder.as_ref()?;
        match self.owner {
            Owner::Store(..) => Some(f.own.as_deref().unwrap_or(&f.default)),
            Owner::Client(_) => f.own.as_deref(),
        }
    }

    /// Where the folder picker opens.
    fn start(&self) -> Option<PathBuf> {
        self.current().map(PathBuf::from).filter(|p| p.is_dir())
    }

    fn id(&self, name: &str) -> SharedString {
        SharedString::from(format!("store-folder-{}-{name}", self.owner.key()))
    }

    fn prompt(&self) -> String {
        tr!("Choose the install folder of {store}", store = self.owner.name())
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.prompt();
        choose_path(window, cx, true, &prompt, |this: &mut Self, path, _, cx| this.set(Some(path), cx));
    }

    fn open(&self) {
        if let Some(f) = self.own() {
            crate::ui::open_external(f);
        }
    }

    /// The path shown, and the line under it.
    fn lines(&self) -> (String, String) {
        let home = home();
        let name = self.owner.name();
        let Some(f) = &self.folder else { return ("\u{2026}".into(), String::new()) };
        match (self.owner, &f.own) {
            (_, Some(own)) => (tilde(own, &home), tr!("Only for {store}", store = name)),
            (Owner::Store(..), None) => (tilde(&f.default, &home), tr!("Default install folder, from Settings → General").to_string()),
            (Owner::Client(_), None) => (tr!("Inside its Wine prefix").to_string(), tr!("{client}'s own folder", client = name)),
        }
    }

    /// The note under the card.
    fn hint(&self) -> String {
        let name = self.owner.name();
        match (self.owner, self.own()) {
            (Owner::Store(..), _) => {
                tr!("New {store} games are installed here. The install dialog can still put one somewhere else.", store = name)
            }
            (Owner::Client(_), None) => tr!("Choose a folder to keep the games of {client} out of its Wine prefix.", client = name),
            (Owner::Client(c), Some(_)) if heronite_clients::games_folder::sets_it(c) => tr!(
                "Heronite sets this folder in {client} each time it opens it, so new games go here. Games already installed stay where they are.",
                client = name
            ),
            (Owner::Client(_), Some(_)) => {
                let path = self.folder.as_ref().and_then(|f| f.windows.clone()).unwrap_or_default();
                tr!(
                    "{client} asks where to install each game: choose {path}. Games already installed stay where they are.",
                    client = name,
                    path = path
                )
            }
        }
    }

    /// The console rows (`folder-change`, `folder-copy`, `folder-default`),
    /// under a heading.
    pub fn console_rows(&self) -> Vec<Row> {
        let (path, line) = self.lines();
        let mut rows = vec![
            Row::header(tr!("Install folder")),
            Row::action("folder-change", path).desc(line).icon(Icon::FolderOpen).enabled(!self.saving && self.folder.is_some()),
        ];
        if self.own().is_some() {
            if let Some(win) = self.folder.as_ref().and_then(|f| f.windows.clone()) {
                rows.push(
                    Row::action("folder-copy", tr!("Path in {client}", client = self.owner.name())).desc(win).icon(Icon::ContentCopy),
                );
            }
            rows.push(Row::action("folder-default", tr!("Use the default folder")).icon(Icon::Undo).enabled(!self.saving));
        }
        rows
    }

    /// Runs a console row of [`console_rows`](Self::console_rows); `false`
    /// when `id` is not one. The folder picker opens in the page's layer.
    pub fn console_activate<V: HasLayer + 'static>(
        this: &Entity<Self>,
        id: &str,
        layer: &mut crate::console::Layer,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> bool {
        match id {
            "folder-change" => {
                let (title, start) = this.read_with(cx, |f, _| (f.prompt(), f.start()));
                let folder = this.clone();
                open_browser(layer, &title, PickKind::Folder, start.as_deref(), window, cx, move |_, picked, _, cx| {
                    let picked = picked.to_string_lossy().into_owned();
                    folder.update(cx, |f, cx| f.set(Some(picked), cx));
                });
            }
            "folder-default" => this.update(cx, |f, cx| f.set(None, cx)),
            "folder-copy" => {
                if let Some(win) = this.read(cx).folder.as_ref().and_then(|f| f.windows.clone()) {
                    cx.write_to_clipboard(ClipboardItem::new_string(win));
                }
            }
            _ => return false,
        }
        true
    }
}

impl Render for StoreFolder {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Closed since: it opens with the new folder.
        self.reopen &= self.client_open(cx);
        let (path, line) = self.lines();
        let own = self.own().is_some();
        let idle = !self.saving && self.folder.is_some();
        let content = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(div().text_size(px(14.)).text_color(theme::text_primary()).truncate().child(path))
            .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(line));
        let mut actions = div().flex().items_center().gap(px(6.));
        if own {
            actions = actions
                .child(icon_button_sm(
                    self.id("open"),
                    ButtonKind::Glass,
                    Icon::FolderOpen,
                    tr!("Open"),
                    true,
                    cx.listener(|this, _, _, _| this.open()),
                ))
                .child(
                    button_sm(self.id("change"), ButtonKind::Glass, tr!("Change…"))
                        .when(idle, |b| b.on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)))),
                )
                .child(
                    button_sm(self.id("default"), ButtonKind::Ghost, tr!("Use default"))
                        .when(idle, |b| b.on_click(cx.listener(|this, _, _, cx| this.set(None, cx)))),
                );
        } else {
            let label = if matches!(self.owner, Owner::Client(_)) { tr!("Choose…") } else { tr!("Change…") };
            actions = actions.child(
                button_sm(self.id("change"), ButtonKind::Glass, label)
                    .when(idle, |b| b.on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)))),
            );
        }
        let lead =
            icon(if own { Icon::FolderOpen } else { Icon::Folder }, px(26.), if own { theme::accent() } else { theme::text_muted() });
        let mut rows = vec![list_item(Some(lead.into_any_element()), content, actions).into_any_element()];
        if let Some(win) = self.folder.as_ref().filter(|_| own).and_then(|f| f.windows.clone()) {
            let copied = win.clone();
            let copy = icon_button_sm(
                self.id("copy"),
                ButtonKind::Ghost,
                Icon::ContentCopy,
                tr!("Copy"),
                true,
                cx.listener(move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))),
            );
            rows.push(row(tr!("Path in {client}", client = self.owner.name()), Some(&win), copy).into_any_element());
        }
        let note = match &self.error {
            Some(e) => div()
                .mt(px(8.))
                .mx(px(6.))
                .text_size(px(12.))
                .text_color(theme::danger_text())
                .child(e.lines().next().unwrap_or("").to_string()),
            None if self.reopen => div()
                .mt(px(8.))
                .mx(px(6.))
                .text_size(px(12.))
                .text_color(theme::warning())
                .child(tr!("{client} is open: close it and open it again for the change to apply.", client = self.owner.name())),
            None => div().mt(px(8.)).mx(px(6.)).child(meta_text(self.hint())),
        };
        div().flex().flex_col().child(caption(tr!("Install folder"))).child(card(rows)).child(note)
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's `test` attribute would come with it.
    use super::{home, read, tilde, Folder, Owner};
    use heronite_data::clients::Client;
    use heronite_data::{game_config, store_folders, DataPaths};

    #[test]
    fn home_is_written_as_a_tilde() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(tilde("/home/u/Games", "/home/u"), "~/Games");
        assert_eq!(tilde("/home/u", "/home/u"), "~");
        assert_eq!(tilde("/data/Games", "/home/u"), "/data/Games");
        assert_eq!(tilde("/data/Games", "/"), "/data/Games");
    }

    #[test]
    fn a_store_without_its_own_folder_shows_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let home = home();
        game_config::set_global_setting(&paths, &home, "defaultInstallPath", serde_json::json!("/data/Games")).unwrap();
        let f = read(&paths, Owner::Store(store_folders::EPIC, "Epic Games"));
        assert_eq!(f, Folder { own: None, default: "/data/Games".into(), windows: None });
        store_folders::set(&paths, &home, Client::Ea.id(), Some("/data/EA")).unwrap();
        let ea = read(&paths, Owner::Client(Client::Ea));
        assert_eq!(ea.own.as_deref(), Some("/data/EA"));
        assert_eq!(ea.windows.as_deref(), Some(r"Z:\data\EA"));
    }
}
