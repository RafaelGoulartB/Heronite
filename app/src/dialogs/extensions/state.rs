//! What the app knows of the installed extensions and of the lists that
//! offer more, kept by the metadata hub ([`MetadataHub::extensions`]) for
//! Settings → Metadata (an extension's row, the "Get more sources" row) and
//! the gallery. Read off the UI thread: the kept copies of the lists when
//! Settings → Metadata shows, the network when the gallery opens or is
//! refreshed.

#[cfg(debug_assertions)]
use std::path::Path;
use std::path::PathBuf;

use gpui_kit::*;
use heronite_metadata::MetadataService;
use heronite_metadata_ext::index::IndexEntry;
use heronite_metadata_ext::lists::{self, Available, ListCopy, ListLocation, Lists, Origin, HERONITE_LIST};
use heronite_metadata_ext::{ExtensionsExt, InstalledExtension};

use super::fake::Fake;
use crate::dialogs::metadata_wizard::{HubEvent, MetadataHub};

/// What the Collection is asked to open.
#[derive(Clone, Debug, PartialEq)]
pub enum SourcesRequest {
    /// "Get more sources", with an entry picked (its id).
    Gallery { pick: Option<String> },
    /// The installer of the package at this path (a zip or a folder).
    File(PathBuf),
}

/// The installed extensions and the lists, as last read.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub installed: Vec<InstalledExtension>,
    /// The lists in use: `(address, the app's own)`.
    pub in_use: Vec<(String, bool)>,
    pub copies: Vec<ListCopy>,
    pub updates: Vec<Available>,
}

/// An entry of the lists as the gallery shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The list that offers it.
    pub list: String,
    pub builtin: bool,
    pub entry: IndexEntry,
    pub state: OfferState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfferState {
    New,
    Installed,
    /// Installed from this list in an older version (the one installed).
    Update(String),
    /// Made for a newer Heronite.
    Unsupported,
}

impl Snapshot {
    /// Reads what is installed and the lists (from the network when
    /// `network`, else the copies kept). Blocking.
    pub fn read(service: &MetadataService, lists: &Lists, network: bool, fake: Option<&Fake>) -> Self {
        let mut installed = service.installed_extensions();
        let (in_use, copies) = match fake {
            Some(f) => {
                f.adjust(&mut installed, &lists.builtin);
                (f.in_use(&lists.builtin), f.copies(&lists.builtin))
            }
            None => (lists.in_use(service), if network { lists.refresh(service) } else { lists.cached(service) }),
        };
        let updates = lists::updates(&copies, &installed);
        Self { installed, in_use, copies, updates }
    }

    pub fn installed(&self, id: &str) -> Option<&InstalledExtension> {
        self.installed.iter().find(|e| e.id == id)
    }

    pub fn update_for(&self, id: &str) -> Option<&Available> {
        self.updates.iter().find(|u| u.id == id)
    }

    /// Where installed extension `id` came from.
    pub fn origin(&self, lists: &Lists, id: &str) -> Origin {
        let list = self.installed(id).and_then(|e| e.list.as_deref());
        lists::origin_among(lists, &self.in_use, list)
    }

    /// Every entry of the lists, each id once (the first list that offers
    /// it, unless a later one offers the update of what was installed from
    /// it), with how it stands here.
    pub fn offers(&self) -> Vec<Offer> {
        let mut out: Vec<Offer> = Vec::new();
        for copy in &self.copies {
            let Some(index) = &copy.index else { continue };
            for entry in &index.entries {
                let state = match (self.installed(&entry.id), self.update_for(&entry.id)) {
                    (_, Some(u)) if u.list == copy.url => OfferState::Update(u.installed.clone()),
                    (Some(_), _) => OfferState::Installed,
                    (None, _) if !entry.supported() => OfferState::Unsupported,
                    (None, _) => OfferState::New,
                };
                let offer = Offer { list: copy.url.clone(), builtin: copy.builtin, entry: entry.clone(), state };
                match out.iter_mut().find(|o| o.entry.id == entry.id) {
                    Some(first) if matches!(offer.state, OfferState::Update(_)) => *first = offer,
                    Some(_) => {}
                    None => out.push(offer),
                }
            }
        }
        out
    }

    /// How many sources the kept lists offer that are not installed, and
    /// the list they come from when only one does: `None` without any copy.
    pub fn more(&self) -> Option<(usize, Option<&ListCopy>)> {
        let offers = self.offers();
        if self.copies.iter().all(|c| c.index.is_none()) {
            return None;
        }
        let new: Vec<&Offer> = offers.iter().filter(|o| o.state == OfferState::New).collect();
        let mut from: Vec<&str> = new.iter().map(|o| o.list.as_str()).collect();
        from.dedup();
        let single = if from.len() == 1 { self.copies.iter().find(|c| c.url == from[0]) } else { None };
        Some((new.len(), single))
    }
}

/// [`MetadataHub::extensions`].
pub struct ExtensionsState {
    pub lists: Lists,
    pub snapshot: Snapshot,
    /// An extension just installed: Settings → Metadata opens its row and
    /// scrolls to it once it is listed.
    pub reveal: Option<String>,
    /// The lists are being read from the network.
    pub reading: bool,
    /// `HERONITE_DEBUG_EXTENSIONS`: made-up lists (`fake.rs`).
    pub fake: Option<Fake>,
    /// The app's own list when it is this repository's
    /// `extensions/metadata/index.json` ([`own_list`]), by its key: the
    /// gallery says it reads "this copy of the repository".
    pub checkout: Option<String>,
    /// Bumped by each read, so an older one that ends later is dropped.
    generation: u64,
    /// The `generation` of the last read from the network: only a later
    /// read from the network drops it.
    network_generation: u64,
    /// The copies the last read from the network gave, laid over those of
    /// a later read of the kept copies (which may have read them before
    /// that read kept its own).
    refreshed: Vec<ListCopy>,
}

/// Where the app's own list is read from:
/// - `HERONITE_EXTENSIONS_INDEX` (`env`) when set: an `https` address or
///   the path of an `index.json` (a test list, a branch's list before it
///   is merged, an author's own);
/// - else, in a debug build, the repository's
///   `extensions/metadata/index.json` (`checkout_index`, when it is there),
///   so its examples install before they are published;
/// - else [`HERONITE_LIST`]: always in a release build.
pub(crate) fn own_list(debug: bool, env: Option<String>, checkout_index: Option<PathBuf>) -> ListLocation {
    match (env, checkout_index) {
        (Some(text), _) => match ListLocation::of(&text) {
            // A relative path is taken from where the app was started.
            ListLocation::File(path) => ListLocation::File(std::path::absolute(&path).unwrap_or(path)),
            address => address,
        },
        (None, Some(index)) if debug => ListLocation::File(index),
        _ => ListLocation::Address(HERONITE_LIST.to_owned()),
    }
}

/// The repository's `extensions/metadata/index.json`, beside the `app`
/// crate this build was made from, when it is still there. A release build neither
/// looks nor carries the path. Built from the crate's absolute folder,
/// never canonicalized: on Windows that gives a `\\?\C:\…` path, shown in
/// the gallery and recorded in `install.json`.
#[cfg(debug_assertions)]
fn checkout_index() -> Option<PathBuf> {
    Some(Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.join("extensions").join("metadata").join("index.json")).filter(|p| p.is_file())
}

#[cfg(not(debug_assertions))]
fn checkout_index() -> Option<PathBuf> {
    None
}

impl ExtensionsState {
    /// The app's own list as [`own_list`] picks it.
    pub fn from_env() -> Self {
        let fake = Fake::from_env();
        let checkout = checkout_index();
        // The made-up lists stand for the Heronite list.
        let debug = cfg!(debug_assertions) && fake.is_none();
        let own = own_list(debug, crate::env::value("HERONITE_EXTENSIONS_INDEX"), checkout.clone()).key();
        let checkout = checkout.map(|p| ListLocation::File(p).key()).filter(|k| *k == own);
        Self {
            lists: Lists::new(own),
            snapshot: Snapshot::default(),
            reveal: None,
            reading: false,
            fake,
            checkout,
            generation: 0,
            network_generation: 0,
            refreshed: Vec::new(),
        }
    }

    /// Whether the list known by `url` is this repository's own.
    pub fn is_checkout(&self, url: &str) -> bool {
        self.checkout.as_deref() == Some(url)
    }

    /// Starts a read (from the network when `network`): its generation.
    fn begin_read(&mut self, network: bool) -> u64 {
        self.generation += 1;
        if network {
            self.network_generation = self.generation;
            self.reading = true;
        }
        self.generation
    }

    /// Takes what read `generation` found; `false` when it is dropped. A
    /// read of the kept copies is dropped by any later read; one from the
    /// network only by a later one from the network. When a read of the
    /// kept copies started after it (an install, a list removed, Settings
    /// shown), its copies join what that read found.
    fn finish_read(&mut self, mut snapshot: Snapshot, network: bool, generation: u64) -> bool {
        if network {
            if generation != self.network_generation {
                return false;
            }
            self.reading = false;
            self.refreshed = snapshot.copies.clone();
            if generation != self.generation {
                snapshot = self.snapshot.clone();
            }
        } else if generation != self.generation {
            return false;
        }
        for copy in &mut snapshot.copies {
            if let Some(fresh) = self.refreshed.iter().find(|r| r.url == copy.url && r.read_at >= copy.read_at) {
                *copy = fresh.clone();
            }
        }
        snapshot.updates = lists::updates(&snapshot.copies, &snapshot.installed);
        self.snapshot = snapshot;
        true
    }
}

impl MetadataHub {
    /// Reads the installed extensions and the lists again off the UI thread
    /// (the network only when `network`), then announces
    /// [`HubEvent::ExtensionsChanged`].
    pub fn read_extensions(&mut self, network: bool, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let lists = self.extensions.lists.clone();
        let fake = self.extensions.fake.clone();
        let generation = self.extensions.begin_read(network);
        cx.spawn(async move |this, cx| {
            let snapshot = cx.background_spawn(async move { Snapshot::read(&service, &lists, network, fake.as_ref()) }).await;
            this.update(cx, |h, cx| {
                if !h.extensions.finish_read(snapshot, network, generation) {
                    return;
                }
                cx.emit(HubEvent::ExtensionsChanged);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Asks the Collection for "Get more sources" or a file's installer.
    pub fn request_sources(&mut self, request: SourcesRequest, cx: &mut Context<Self>) {
        cx.emit(HubEvent::OpenSources(request));
    }

    /// An extension was installed or updated: its row opens in Settings →
    /// Metadata once the rows are read again.
    pub fn extension_installed(&mut self, id: &str, cx: &mut Context<Self>) {
        self.extensions.reveal = Some(id.to_owned());
        self.read_extensions(false, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use heronite_metadata_ext::index;
    use heronite_metadata_ext::lists::ListError;

    use super::{lists, own_list, InstalledExtension, ListCopy, ListLocation, OfferState, Snapshot, HERONITE_LIST};

    #[test]
    fn a_debug_build_reads_the_repository_list_and_a_release_build_never_does() {
        let checkout = Some(PathBuf::from("/src/heronite/extensions/metadata/index.json"));
        let heronite = ListLocation::Address(HERONITE_LIST.into());
        assert_eq!(own_list(false, None, checkout.clone()), heronite, "a release build: always the Heronite list");
        assert_eq!(own_list(false, None, None), heronite);
        assert_eq!(own_list(true, None, checkout.clone()), ListLocation::File("/src/heronite/extensions/metadata/index.json".into()));
        assert_eq!(own_list(true, None, None), heronite, "a debug build away from its repository");
        // The switch wins in any build: an address or a path.
        let branch = "https://raw.githubusercontent.com/someone/Heronite/branch/extensions/metadata/index.json";
        for debug in [false, true] {
            assert_eq!(own_list(debug, Some(branch.into()), checkout.clone()), ListLocation::Address(branch.into()));
            // Absolute here; on Windows that adds the current drive.
            assert_eq!(
                own_list(debug, Some("/home/me/list/index.json".into()), checkout.clone()),
                ListLocation::File(std::path::absolute("/home/me/list/index.json").unwrap())
            );
        }
        let relative = own_list(false, Some("list/index.json".into()), None);
        assert_eq!(relative, ListLocation::File(std::env::current_dir().unwrap().join("list/index.json")), "taken from where it started");
    }

    fn copy(url: &str, builtin: bool, json: &str) -> ListCopy {
        ListCopy { url: url.into(), builtin, index: Some(index::parse(json.as_bytes()).unwrap()), read_at: None, error: None }
    }

    fn installed(id: &str, version: &str, list: Option<&str>) -> InstalledExtension {
        InstalledExtension {
            id: id.into(),
            name: id.into(),
            version: version.into(),
            author: String::new(),
            kind: None,
            installed_at: String::new(),
            from: String::new(),
            list: list.map(Into::into),
            hosts: Vec::new(),
            image_hosts: Vec::new(),
            sha256: String::new(),
            state: heronite_metadata_ext::ExtensionState::Loaded,
        }
    }

    #[test]
    fn offers_say_how_each_entry_stands_here() {
        let own = copy("https://own", true, super::super::fake::MADE_UP);
        let mut snap = Snapshot {
            installed: vec![installed("wikipedia", "1.0.0", Some("https://own")), installed("vndb", "1.0.0", None)],
            in_use: vec![("https://own".into(), true)],
            copies: vec![own],
            updates: Vec::new(),
        };
        snap.updates = lists::updates(&snap.copies, &snap.installed);
        let offers = snap.offers();
        let state = |id: &str| offers.iter().find(|o| o.entry.id == id).map(|o| o.state.clone());
        assert_eq!(state("wikipedia"), Some(OfferState::Update("1.0.0".into())));
        assert_eq!(state("vndb"), Some(OfferState::Installed), "installed from a file: never flagged");
        assert_eq!(state("indie-shelf"), Some(OfferState::New));
        assert_eq!(state("future-db"), Some(OfferState::Unsupported));
        let (n, single) = snap.more().unwrap();
        assert_eq!(n, offers.iter().filter(|o| o.state == OfferState::New).count());
        assert_eq!(single.map(|c| c.url.as_str()), Some("https://own"));

        snap.copies[0].index = None;
        snap.copies[0].error = Some(ListError::Status(404));
        assert!(snap.more().is_none(), "no copy: just the action");
    }

    const OTHER: &str = "https://lists.example.org/index.json";

    #[test]
    fn the_update_shows_from_the_list_it_was_installed_from_whatever_list_comes_first() {
        let own = copy("https://own", true, super::super::fake::MADE_UP);
        let other = copy(OTHER, false, &super::super::fake::MADE_UP.replace("\"1.1.0\"", "\"1.2.0\""));
        let mut snap = Snapshot {
            installed: vec![installed("wikipedia", "1.1.0", Some(OTHER))],
            in_use: vec![("https://own".into(), true), (OTHER.into(), false)],
            copies: vec![own, other],
            updates: Vec::new(),
        };
        snap.updates = lists::updates(&snap.copies, &snap.installed);
        let offers = snap.offers();
        let wiki: Vec<&super::Offer> = offers.iter().filter(|o| o.entry.id == "wikipedia").collect();
        assert_eq!(wiki.len(), 1, "each id once");
        assert_eq!((wiki[0].list.as_str(), &wiki[0].state), (OTHER, &OfferState::Update("1.1.0".into())));
        assert_eq!(wiki[0].entry.version.to_string(), "1.2.0");
        // The other ids still come from the first list.
        assert!(offers.iter().filter(|o| o.entry.id != "wikipedia").all(|o| o.list == "https://own"));
    }

    fn state() -> super::ExtensionsState {
        super::ExtensionsState {
            lists: lists::Lists::new("https://own"),
            snapshot: Snapshot::default(),
            reveal: None,
            reading: false,
            fake: None,
            checkout: None,
            generation: 0,
            network_generation: 0,
            refreshed: Vec::new(),
        }
    }

    /// What a read finds: `wikipedia` at `version` installed from the
    /// list, which offers `offered`, read `ago` seconds ago.
    fn found(version: &str, offered: &str, ago: u64) -> Snapshot {
        let mut c = copy("https://own", true, &super::super::fake::MADE_UP.replace("\"1.1.0\"", &format!("\"{offered}\"")));
        c.read_at = Some(std::time::SystemTime::now() - std::time::Duration::from_secs(ago));
        Snapshot {
            installed: vec![installed("wikipedia", version, Some("https://own"))],
            in_use: vec![("https://own".into(), true)],
            copies: vec![c],
            updates: Vec::new(),
        }
    }

    fn offered(s: &super::ExtensionsState) -> String {
        s.snapshot.copies[0].index.as_ref().unwrap().get("wikipedia").unwrap().version.to_string()
    }

    #[test]
    fn a_read_of_the_kept_copies_never_drops_a_refresh_from_the_network() {
        let mut s = state();
        // The gallery refreshes; meanwhile an install reads the kept copies
        // (from before the refresh) and ends first.
        let network = s.begin_read(true);
        let kept = s.begin_read(false);
        assert!(s.reading);
        assert!(s.finish_read(found("1.1.0", "1.1.0", 3 * 86_400), false, kept));
        assert!(s.reading, "the refresh still runs");
        // The refresh ends: its copies are taken, what is installed is the
        // later read's.
        assert!(s.finish_read(found("1.0.0", "1.2.0", 0), true, network));
        assert!(!s.reading);
        assert_eq!(offered(&s), "1.2.0");
        assert_eq!(s.snapshot.installed[0].version, "1.1.0");
        assert_eq!(s.snapshot.updates.len(), 1, "worked out again: {:?}", s.snapshot.updates);
        // A later read of the kept copies, read before the refresh kept its
        // own, still shows the refresh's.
        let kept = s.begin_read(false);
        assert!(s.finish_read(found("1.1.0", "1.1.0", 3 * 86_400), false, kept));
        assert_eq!(offered(&s), "1.2.0");

        // A refresh is dropped by a later refresh only; a read of the kept
        // copies by any later read.
        let first = s.begin_read(true);
        let second = s.begin_read(true);
        assert!(s.finish_read(found("1.1.0", "1.3.0", 0), true, second));
        assert!(!s.finish_read(found("1.1.0", "1.2.5", 0), true, first));
        assert_eq!(offered(&s), "1.3.0");
        let old = s.begin_read(false);
        let new = s.begin_read(false);
        assert!(s.finish_read(found("1.1.0", "1.3.0", 0), false, new));
        assert!(!s.finish_read(found("1.1.0", "1.3.0", 0), false, old));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_checkouts_list_is_named_by_a_plain_absolute_path() {
        use std::path::Component;
        let path = super::checkout_index().expect("this checkout has extensions/metadata/index.json");
        assert!(path.is_absolute() && path.ends_with("extensions/metadata/index.json"), "{path:?}");
        // No `..`, and no `\\?\` prefix on Windows (shown and recorded as is).
        assert!(
            path.components().all(|c| match c {
                Component::ParentDir => false,
                Component::Prefix(p) => !p.kind().is_verbatim(),
                _ => true,
            }),
            "{path:?}"
        );
    }
}
