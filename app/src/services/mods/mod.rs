//! The mods service (`docs/MODS.md`): one entity, kept in a global, that owns
//! everything about mods that outlives a dialog: each game's mod list as last
//! loaded, the downloads and installs in progress, the FOMOD installer and the
//! game picker waiting for the user, the Nexus Mods account, and the launch
//! hook that mounts a game's mods while it runs.
//!
//! Views read it and call its methods; it never opens a view. What needs the
//! user arrives as [`ModsEvent`]s, which the Collection turns into dialogs.
//!
//! The public items of this file are what the views use; the bodies live in
//! the sibling modules.

mod account;
mod edit;
mod facts;
mod handlers;
mod install;
mod jobs;
mod launch;
mod links;
mod load;
mod steam;
mod updates;

pub use launch::{startup, ModsLaunch};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui_kit::*;
use heronite_data::{CollectionGame, DataPaths};
use heronite_mods::conflicts::Conflicts;
use heronite_mods::fomod::Wizard;
use heronite_mods::games::{KnownGame, Profile};
use heronite_mods::nexus::StoredAccount;
use heronite_mods::tool::Support;
use heronite_mods::{GameLinks, ModList, ModsRoot, Progress};

/// What the manager shows about one game, resolved off the UI thread.
#[derive(Clone, Debug)]
pub struct GameModInfo {
    pub title: String,
    /// The folder the overlay mounts over; `None` = not found on this
    /// computer (mods cannot be applied, the manager says so).
    pub game_dir: Option<PathBuf>,
    /// The Wine prefix (the folder holding `drive_c`), for `Plugins.txt`.
    pub prefix: Option<PathBuf>,
    pub steam_app_id: Option<String>,
    /// Launched through Steam (`steam://`): environment variables cannot be
    /// added to the launch, they go in Steam's launch options.
    pub through_steam: bool,
    /// A Linux build: no Wine, no DLL overrides, no `Plugins.txt`.
    pub native: bool,
    pub known: Option<&'static KnownGame>,
    pub profile: Profile,
    /// The links in effect: the list's own, else found (known table, site search).
    pub nexus_domain: Option<String>,
    pub thunderstore: Option<String>,
}

/// One game as the manager shows it.
#[derive(Clone, Debug)]
pub struct GameMods {
    pub info: GameModInfo,
    pub list: ModList,
    /// Who wins which files among the enabled mods; `None` while computing.
    pub conflicts: Option<Arc<Conflicts>>,
    /// Files and bytes the game wrote while modded (`overwrite/`).
    pub overwrite: (u64, u64),
    /// Steam game whose enabled mods need `WINEDLLOVERRIDES` that its Steam
    /// launch options do not have yet: the value to add (`winhttp=n,b`).
    pub steam_overrides_missing: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStage {
    Waiting,
    /// fuse-overlayfs being fetched (first use).
    Tool,
    Downloading,
    Extracting,
    /// The FOMOD installer waits for the user ([`ModsService::fomod`]).
    NeedsChoices,
    Installing,
    Done,
    Failed,
    Cancelled,
}

/// A download or install, shown in the manager (and as a notice when it ends).
#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub game_key: String,
    /// The mod's name as known so far (file name until the site answers).
    pub name: String,
    pub stage: JobStage,
    pub progress: Progress,
    /// Translated reason when `Failed`.
    pub error: Option<String>,
    /// Thunderstore dependencies installed along ("+2 dependencies").
    pub dependencies: usize,
}

/// A FOMOD installer waiting for the user's choices. The wizard dialog drives
/// `wizard` and answers with [`ModsService::finish_fomod`] or
/// [`ModsService::cancel_fomod`].
#[derive(Clone)]
pub struct FomodRequest {
    pub job_id: u64,
    pub mod_name: String,
    pub wizard: Arc<Mutex<Wizard>>,
}

/// A mod link whose game could not be told for sure: the user picks it.
#[derive(Clone, Debug)]
pub struct LinkRequest {
    pub id: u64,
    /// The game's name on the site ("Skyrim Special Edition"), if known.
    pub site_game: Option<String>,
    /// Installed games that may be it (best first); may be empty.
    pub candidates: Vec<String>,
}

/// Where Heronite stands with `nxm://` / `ror2mm://` links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerState {
    Unknown,
    /// Heronite opens them.
    Ours,
    /// Another program does (Vortex in Wine, r2modman…).
    Other,
    /// Nothing does.
    None,
}

#[derive(Clone, Debug)]
pub enum ModsEvent {
    /// A game's list, conflicts or overwrite changed.
    Changed { game_key: String },
    /// Jobs started, moved on or ended.
    Jobs,
    /// A FOMOD installer waits ([`ModsService::fomod`]).
    Fomod,
    /// A link needs its game picked ([`ModsService::link_request`]).
    PickGame,
    /// The Nexus account or the link handlers changed.
    Account,
    /// Show the manager of this game (a link was accepted for it).
    OpenManager { game_key: String },
    /// A job ended: a short line for a toast ("SkyUI installed for Skyrim").
    Notice { game_key: String, message: String, error: bool },
}

pub struct ModsService {
    data_paths: DataPaths,
    root: ModsRoot,
    games: HashMap<String, GameMods>,
    jobs: Vec<Job>,
    fomod: Option<FomodRequest>,
    link_request: Option<LinkRequest>,
    account: Option<StoredAccount>,
    support: Option<Support>,
    handlers: HandlerState,
    next_id: u64,
    /// Lists shared with the worker threads, one per game used so far.
    shared: HashMap<String, Arc<edit::GameShared>>,
    /// The games as the Collection last handed them over, for the jobs.
    sources: HashMap<String, Arc<CollectionGame>>,
    /// Mods and enabled mods of games whose list is not loaded.
    counts: HashMap<String, (usize, usize)>,
    work: jobs::Work,
    /// Counts runs of the conflict / Steam checks, so only the last one lands.
    checks: HashMap<String, u64>,
    links: links::Links,
}

impl EventEmitter<ModsEvent> for ModsService {}

struct ModsGlobal(Entity<ModsService>);
impl Global for ModsGlobal {}

/// The service, created on first use.
pub fn service(cx: &mut App) -> Entity<ModsService> {
    if let Some(g) = cx.try_global::<ModsGlobal>() {
        return g.0.clone();
    }
    let data_paths = DataPaths::detect();
    let entity = cx.new(|cx| ModsService::new(data_paths, cx));
    cx.set_global(ModsGlobal(entity.clone()));
    entity
}

/// The service if something already created it.
pub fn started(cx: &App) -> Option<Entity<ModsService>> {
    cx.try_global::<ModsGlobal>().map(|g| g.0.clone())
}

/// An `nxm://` / `ror2mm://` link from the command line or another invocation.
pub fn open_link(cx: &mut App, link: String) {
    service(cx).update(cx, |s, cx| s.open_link(link, cx));
}

impl ModsService {
    fn new(data_paths: DataPaths, cx: &mut Context<Self>) -> Self {
        let root = ModsRoot::default_for(&data_paths);
        let (work, messages) = jobs::Work::new();
        let mut this = Self {
            data_paths,
            root,
            games: HashMap::new(),
            jobs: Vec::new(),
            fomod: None,
            link_request: None,
            account: None,
            support: None,
            handlers: HandlerState::Unknown,
            next_id: 1,
            shared: HashMap::new(),
            sources: HashMap::new(),
            counts: HashMap::new(),
            work,
            checks: HashMap::new(),
            links: links::Links::default(),
        };
        // Every event is also a change of what the views read: observers redraw.
        cx.subscribe_self(|_, _: &ModsEvent, cx| cx.notify()).detach();
        this.listen(messages, cx);
        this.read_account(cx);
        this
    }

    // ---- Reading (cheap, for render) ----

    pub fn root(&self) -> &ModsRoot {
        &self.root
    }

    /// The game as last loaded; `None` until [`load`](Self::load) lands.
    pub fn game(&self, game_key: &str) -> Option<&GameMods> {
        self.games.get(game_key)
    }

    /// Number of mods and of enabled mods of a game whose list is loaded
    /// (the Focus panel's row; `None` = not loaded or no mods). Before
    /// [`load`](Self::load), the numbers [`load_counts`](Self::load_counts) read.
    pub fn counts(&self, game_key: &str) -> Option<(usize, usize)> {
        let counted = match self.games.get(game_key) {
            Some(g) => (g.list.mods.len(), g.list.mods.iter().filter(|m| m.enabled).count()),
            None => *self.counts.get(game_key)?,
        };
        (counted.0 > 0).then_some(counted)
    }

    /// Jobs of a game (all games with `None`), oldest first.
    pub fn jobs<'a>(&'a self, game_key: Option<&'a str>) -> impl Iterator<Item = &'a Job> + 'a {
        self.jobs.iter().filter(move |j| game_key.is_none_or(|k| j.game_key == k))
    }

    pub fn fomod(&self) -> Option<&FomodRequest> {
        self.fomod.as_ref()
    }

    pub fn link_request(&self) -> Option<&LinkRequest> {
        self.link_request.as_ref()
    }

    pub fn account(&self) -> Option<&StoredAccount> {
        self.account.as_ref()
    }

    /// fuse-overlayfs / FUSE availability, once checked.
    pub fn support(&self) -> Option<&Support> {
        self.support.as_ref()
    }

    pub fn handlers(&self) -> HandlerState {
        self.handlers
    }

    // ---- Loading ----

    /// Loads (or reloads) the game's list, resolves its folder, prefix and
    /// links, then computes conflicts and the overwrite size; emits
    /// `Changed` as each part lands. Also checks [`support`](Self::support).
    pub fn load(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        self.load_game(game, cx);
    }

    /// Reads the counts of every game with a `mods.json` (Focus panel rows)
    /// without resolving folders.
    pub fn load_counts(&mut self, cx: &mut Context<Self>) {
        self.read_counts(cx);
    }

    // ---- Adding mods ----

    /// An archive or a folder the user picked or dropped.
    pub fn install_path(&mut self, game_key: &str, path: PathBuf, cx: &mut Context<Self>) {
        self.start_path_install(game_key, path, cx);
    }

    /// A Nexus mod from the Browse tab (premium accounts: the main file, or
    /// `file_id`).
    pub fn install_nexus(&mut self, game_key: &str, mod_id: u64, file_id: Option<u64>, cx: &mut Context<Self>) {
        self.start_nexus_install(game_key, mod_id, file_id, cx);
    }

    /// A Thunderstore package with its dependencies.
    pub fn install_thunderstore(&mut self, game_key: &str, namespace: &str, name: &str, cx: &mut Context<Self>) {
        self.start_thunderstore_install(game_key, namespace, name, cx);
    }

    /// An `nxm://` / `ror2mm://` link: finds the game (or asks with
    /// `PickGame`), then downloads and installs.
    pub fn open_link(&mut self, link: String, cx: &mut Context<Self>) {
        self.receive_link(link, cx);
    }

    /// The user picked the game of a [`LinkRequest`] (`None` = cancel).
    pub fn answer_link(&mut self, id: u64, game_key: Option<String>, cx: &mut Context<Self>) {
        self.pick_link_game(id, game_key, cx);
    }

    /// The FOMOD wizard is done: installs `wizard.files()` and remembers
    /// `wizard.choices()`.
    pub fn finish_fomod(&mut self, job_id: u64, cx: &mut Context<Self>) {
        self.answer_fomod(job_id, jobs::Answer::Finish, cx);
    }

    /// The FOMOD wizard was closed: `plain` installs the archive as it is
    /// instead (scripted installers, or the user's choice), else the job ends.
    pub fn cancel_fomod(&mut self, job_id: u64, plain: bool, cx: &mut Context<Self>) {
        self.answer_fomod(job_id, if plain { jobs::Answer::Plain } else { jobs::Answer::Cancel }, cx);
    }

    pub fn cancel_job(&mut self, job_id: u64, cx: &mut Context<Self>) {
        self.cancel(job_id, cx);
    }

    /// Removes a finished or failed job from the list.
    pub fn dismiss_job(&mut self, job_id: u64, cx: &mut Context<Self>) {
        self.dismiss(job_id, cx);
    }

    // ---- Managing ----

    /// "Play with mods".
    pub fn set_active(&mut self, game_key: &str, on: bool, cx: &mut Context<Self>) {
        self.edit_active(game_key, on, cx);
    }

    pub fn set_enabled(&mut self, game_key: &str, mod_id: &str, on: bool, cx: &mut Context<Self>) {
        self.edit_enabled(game_key, mod_id, on, cx);
    }

    /// Moves a mod to `to_index` of the load order.
    pub fn move_mod(&mut self, game_key: &str, mod_id: &str, to_index: usize, cx: &mut Context<Self>) {
        self.edit_move(game_key, mod_id, to_index, cx);
    }

    pub fn rename(&mut self, game_key: &str, mod_id: &str, name: String, cx: &mut Context<Self>) {
        self.edit_rename(game_key, mod_id, name, cx);
    }

    pub fn remove(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        self.edit_remove(game_key, mod_id, cx);
    }

    /// Installs the mod again from its kept archive (FOMOD: the wizard opens
    /// with the previous choices).
    pub fn reinstall(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        self.start_reinstall(game_key, mod_id, cx);
    }

    /// Downloads and installs the newer version in place of the mod.
    pub fn update(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        self.start_update(game_key, mod_id, cx);
    }

    /// Checks the site for newer versions of the game's mods (Nexus needs the
    /// account); results land in each `ModEntry::update`.
    pub fn check_updates(&mut self, game_key: &str, cx: &mut Context<Self>) {
        self.look_for_updates(game_key, cx);
    }

    /// Turns a Bethesda plugin on or off (`Plugins.txt`).
    pub fn set_plugin(&mut self, game_key: &str, plugin: &str, on: bool, cx: &mut Context<Self>) {
        self.edit_plugin(game_key, plugin, on, cx);
    }

    /// The manager's game settings: Nexus game, Thunderstore community, mods folder.
    pub fn set_links(&mut self, game_key: &str, links: GameLinks, cx: &mut Context<Self>) {
        self.edit_links(game_key, links, cx);
    }

    /// Adds the missing `WINEDLLOVERRIDES` to the Steam game's launch options.
    pub fn add_steam_overrides(&mut self, game_key: &str, cx: &mut Context<Self>) {
        self.write_steam_overrides(game_key, cx);
    }

    pub fn clear_overwrite(&mut self, game_key: &str, cx: &mut Context<Self>) {
        self.empty_overwrite(game_key, cx);
    }

    /// Makes a mod (at the end of the list) of what the game wrote.
    pub fn overwrite_into_mod(&mut self, game_key: &str, name: String, cx: &mut Context<Self>) {
        self.turn_overwrite_into_mod(game_key, name, cx);
    }

    /// Removes every mod of the game and its folder under the mods root.
    pub fn forget_game(&mut self, game_key: &str, cx: &mut Context<Self>) {
        self.forget(game_key, cx);
    }

    // ---- Account and links ----

    /// Checks the key with Nexus Mods and keeps it when valid; `Account`
    /// event either way, the error as a `Notice`.
    pub fn set_api_key(&mut self, key: String, cx: &mut Context<Self>) {
        self.validate_key(key, cx);
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.forget_account(cx);
    }

    /// Reads who opens `nxm://` / `ror2mm://` (xdg-mime), off the UI thread.
    pub fn refresh_handlers(&mut self, cx: &mut Context<Self>) {
        self.read_handlers(cx);
    }

    /// Makes Heronite open `nxm://` and `ror2mm://` links.
    pub fn register_handlers(&mut self, cx: &mut Context<Self>) {
        self.write_handlers(cx);
    }

    /// Downloads fuse-overlayfs now (Settings → Mods), instead of at the
    /// first modded launch.
    pub fn fetch_tool(&mut self, cx: &mut Context<Self>) {
        self.download_tool(cx);
    }
}
