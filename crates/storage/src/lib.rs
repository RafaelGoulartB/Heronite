//! What the games take on disk, for Settings → Storage: the installed games,
//! the Wine prefixes, the Proton / Wine builds and runtimes, what Steam keeps
//! for games that are no longer installed (Proton prefixes, shader caches),
//! the caches that rebuild themselves, and game folders nothing points to any
//! more. And removing what is not needed.
//!
//! [`inventory`] lists everything quickly (the sizes stores and manifests give
//! are filled in, nothing is walked); [`measure`] walks one item whose size is
//! unknown; [`remove`] cleans one item. All blocking: the app calls them off
//! the UI thread, only while the Storage section is on screen.

mod builds;
mod caches;
mod clean;
pub mod disks;
mod games;
mod leftovers;
mod prefixes;
mod steam;
pub mod walk;

use std::path::{Path, PathBuf};

use heronite_data::install_size::SizeCache;
use heronite_data::{AppPaths, DataPaths, Library};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::steam::SteamLocator;
use heronite_launch::SystemProbe;

pub use clean::{remove, Removed};
pub use disks::Disk;

/// Where to look.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub data_paths: DataPaths,
    pub app: AppPaths,
    pub home: PathBuf,
}

impl Ctx {
    pub fn new(data_paths: DataPaths, app: AppPaths) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self { data_paths, app, home }
    }

    fn home_str(&self) -> String {
        self.home.to_string_lossy().into_owned()
    }

    fn steam(&self) -> SteamLocator {
        SteamLocator::detect(&GlobalSettings::load(&self.data_paths), &SystemProbe)
    }
}

/// The blocks of the page, in the order they are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    Games,
    Prefixes,
    /// Proton / Wine builds, runtimes and tools.
    Builds,
    /// Proton prefixes and shader caches Steam keeps for games no longer installed.
    Steam,
    Caches,
    /// Game folders nothing points to any more.
    Leftovers,
}

impl Group {
    pub const ALL: [Group; 6] = [Group::Games, Group::Prefixes, Group::Builds, Group::Steam, Group::Caches, Group::Leftovers];
}

/// Whether something is needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// An installed game, or what one (or the defaults, or Steam) uses.
    InUse,
    /// Nothing installed uses it.
    Unused,
    /// Rebuilt by itself when needed.
    Cache,
    /// Needed to run games (runtimes, tools): shown, never removed from here.
    Needed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    /// Covers and art resized for the screen.
    Images,
    /// Covers and art as downloaded (`images-cache`); fetched again when shown.
    Downloads,
    /// Store and wiki game info (`store_cache`, rebuilt by the stores' refresh).
    StoreInfo,
    /// Downloads of install scripts.
    InstallScripts,
    /// Installers winetricks downloaded.
    Winetricks,
    /// Mesa's and NVIDIA's compiled shaders.
    GpuShaders,
    /// Steam's shader caches of the installed games.
    SteamShaders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An installed game (`CollectionGame::key`): removed by uninstalling it.
    Game {
        key: String,
    },
    /// A Wine prefix of this app's games.
    Prefix {
        default: bool,
    },
    /// A build of the Wine Manager (`WineVersionInfo::version`).
    Build {
        version: String,
        default: bool,
    },
    /// A runtime or tools folder.
    Runtime,
    /// `steamapps/compatdata/<id>` of a game no longer installed.
    SteamPrefix {
        app_id: String,
    },
    /// `steamapps/shadercache/<id>` of a game no longer installed.
    SteamShaders {
        app_id: String,
    },
    Cache(CacheKind),
    /// A folder in a games folder that no game uses.
    Leftover,
}

/// What "Free up space" can take, chosen together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sweep {
    Prefixes,
    Steam,
    Builds,
    Leftovers,
    /// The caches of this app and winetricks.
    Caches,
    /// Compiled shaders: games stutter for a while after they go.
    Shaders,
}

impl Sweep {
    pub const ALL: [Sweep; 6] = [Sweep::Prefixes, Sweep::Steam, Sweep::Builds, Sweep::Leftovers, Sweep::Caches, Sweep::Shaders];

    /// Ticked when the page opens.
    pub fn default_on(self) -> bool {
        self != Sweep::Shaders
    }
}

/// One thing on disk.
#[derive(Clone, Debug)]
pub struct Item {
    /// Unique: the main path.
    pub id: String,
    pub group: Group,
    pub kind: Kind,
    pub state: State,
    pub title: String,
    /// The folder to open (and whose disk it is on).
    pub folder: PathBuf,
    /// The folders (or files) it is made of: what is measured and removed.
    pub paths: Vec<PathBuf>,
    /// Bytes on disk; `None` until measured (or when it cannot be).
    pub size: Option<u64>,
    /// Whether [`measure`] should walk it.
    pub needs_walk: bool,
    /// Collection keys of the installed games using it (a game: itself).
    pub games: Vec<String>,
    /// The store clients (Battle.net, the EA app...) keeping a prefix or a
    /// Wine build in use by themselves: they are no game of the Collection,
    /// so [`games`](Self::games) may be empty while the item is in use.
    pub clients: Vec<String>,
    /// Store or source shown next to the title (a game's "Steam", "Epic"...).
    pub source: Option<String>,
    /// When it was last played (games; RFC 3339 as the library keeps it).
    pub last_played: Option<String>,
    /// Index into [`Inventory::disks`].
    pub disk: Option<usize>,
    /// Ids of the games installed inside it (a prefix's `drive_c`): their
    /// space is theirs, not the prefix's (see [`Inventory::own_size`]).
    pub holds: Vec<String>,
}

impl Item {
    fn new(group: Group, kind: Kind, state: State, title: impl Into<String>, path: PathBuf) -> Self {
        Self {
            id: path.to_string_lossy().into_owned(),
            group,
            kind,
            state,
            title: title.into(),
            folder: path.clone(),
            paths: vec![path],
            size: None,
            needs_walk: true,
            games: Vec::new(),
            clients: Vec::new(),
            source: None,
            last_played: None,
            disk: None,
            holds: Vec::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.folder
    }

    /// Which "Free up space" choice takes it, if any.
    pub fn sweep(&self) -> Option<Sweep> {
        match (&self.kind, self.state) {
            (Kind::Cache(CacheKind::GpuShaders | CacheKind::SteamShaders), _) => Some(Sweep::Shaders),
            (Kind::Cache(_), _) => Some(Sweep::Caches),
            (_, State::InUse | State::Needed | State::Cache) => None,
            (Kind::Prefix { .. }, State::Unused) => Some(Sweep::Prefixes),
            (Kind::Build { .. }, State::Unused) => Some(Sweep::Builds),
            (Kind::SteamPrefix { .. } | Kind::SteamShaders { .. }, State::Unused) => Some(Sweep::Steam),
            (Kind::Leftover, State::Unused) => Some(Sweep::Leftovers),
            _ => None,
        }
    }

    /// Whether [`remove`] can take it (games go through their uninstall).
    pub fn removable(&self) -> bool {
        self.sweep().is_some()
    }

    /// Whether removing it moves it to the trash (it may hold saves or
    /// settings) instead of deleting it.
    pub fn goes_to_trash(&self) -> bool {
        matches!(self.kind, Kind::Prefix { .. } | Kind::SteamPrefix { .. } | Kind::Leftover)
    }
}

/// Everything found, with the disks it lives on.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub items: Vec<Item>,
    pub disks: Vec<Disk>,
}

/// Lists everything. Blocking but quick: reads settings, manifests and folder
/// listings, never walks a folder (see [`measure`]). `lib` is the library the
/// Collection loaded.
pub fn inventory(ctx: &Ctx, lib: &Library) -> Inventory {
    let steam = ctx.steam();
    let walked = SizeCache::load(&ctx.app.cache);
    let mut items = games::items(lib, &steam, &walked);
    items.extend(prefixes::items(ctx, lib, &walked));
    nest_prefixes(&mut items);
    items.extend(builds::items(ctx, lib, &steam));
    let steam_found = steam::items(lib, &ctx.data_paths, &steam);
    items.extend(steam_found.leftovers);
    items.extend(caches::items(ctx, steam_found.shader_caches));
    items.extend(leftovers::items(ctx, lib, &items, &walked));

    let mounts = disks::mounts();
    let mut found: Vec<Disk> = Vec::new();
    for item in &mut items {
        item.disk = disks::locate(&mounts, item.path(), &mut found);
    }
    // The data folder's disk shows even with nothing on it.
    disks::locate(&mounts, &ctx.data_paths.root, &mut found);
    Inventory { items, disks: found }
}

/// A prefix in a game's folder goes with the game: it is left out. A game
/// installed inside a prefix is noted in the prefix's [`Item::holds`] (and
/// keeps the prefix in use).
fn nest_prefixes(items: &mut Vec<Item>) {
    let games: Vec<(String, PathBuf)> = items
        .iter()
        .filter(|i| matches!(i.kind, Kind::Game { .. }) && i.path().is_dir())
        .map(|i| (i.id.clone(), i.folder.clone()))
        .collect();
    items.retain(|i| !matches!(i.kind, Kind::Prefix { .. }) || !games.iter().any(|(_, g)| i.path().starts_with(g)));
    for item in items.iter_mut().filter(|i| matches!(i.kind, Kind::Prefix { .. })) {
        item.holds = games.iter().filter(|(_, g)| g.starts_with(&item.folder)).map(|(id, _)| id.clone()).collect();
        // Removing it would take the game too.
        if !item.holds.is_empty() {
            item.state = State::InUse;
        }
    }
}

/// Walks `item` and returns its size; `None` when `stop` asked or it cannot
/// be measured. Games, prefixes and leftovers keep the result in
/// `install-sizes.json`, so the next visit does not walk them again.
pub fn measure(ctx: &Ctx, item: &Item, stop: &dyn Fn() -> bool) -> Option<u64> {
    let cached = matches!(item.kind, Kind::Game { .. } | Kind::Prefix { .. } | Kind::Leftover);
    let mut total = 0;
    for path in &item.paths {
        total += walk::disk_usage(path, stop)?;
    }
    if cached {
        if let Err(err) = SizeCache::put(&ctx.app.cache, item.path(), total) {
            log::warn!("saving the size of {}: {err:#}", item.path().display());
        }
    }
    Some(total)
}

impl Inventory {
    /// The space `item` takes by itself: its size without the games installed
    /// inside it. `None` until measured.
    pub fn own_size(&self, item: &Item) -> Option<u64> {
        let size = item.size?;
        let inner: u64 = item.holds.iter().filter_map(|id| self.items.iter().find(|i| &i.id == id)?.size).sum();
        Some(size.saturating_sub(inner))
    }
}
