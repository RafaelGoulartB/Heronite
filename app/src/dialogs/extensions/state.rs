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
    /// it), with how it stands here.
    pub fn offers(&self) -> Vec<Offer> {
        let mut out: Vec<Offer> = Vec::new();
        for copy in &self.copies {
            let Some(index) = &copy.index else { continue };
            for entry in &index.entries {
                if out.iter().any(|o| o.entry.id == entry.id) {
                    continue;
                }
                let state = match (self.installed(&entry.id), self.update_for(&entry.id)) {
                    (_, Some(u)) if u.list == copy.url => OfferState::Update(u.installed.clone()),
                    (Some(_), _) => OfferState::Installed,
                    (None, _) if !entry.supported() => OfferState::Unsupported,
                    (None, _) => OfferState::New,
                };
                out.push(Offer { list: copy.url.clone(), builtin: copy.builtin, entry: entry.clone(), state });
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
    /// `extensions/index.json` ([`own_list`]), by its key: the gallery says
    /// it reads "this copy of the repository".
    pub checkout: Option<String>,
    /// Bumped by each read, so an older one that ends later is dropped.
    generation: u64,
}

/// Where the app's own list is read from:
/// - `HERONITE_EXTENSIONS_INDEX` (`env`) when set: an `https` address or
///   the path of an `index.json` (a test list, a branch's list before it
///   is merged, an author's own);
/// - else, in a debug build, the repository's `extensions/index.json`
///   (`checkout_index`, when it is there), so its examples install before
///   they are published;
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

/// The repository's `extensions/index.json`, beside the `app` crate this
/// build was made from, when it is still there. A release build neither
/// looks nor carries the path.
#[cfg(debug_assertions)]
fn checkout_index() -> Option<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../extensions/index.json").canonicalize().ok().filter(|p| p.is_file())
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
        Self { lists: Lists::new(own), snapshot: Snapshot::default(), reveal: None, reading: false, fake, checkout, generation: 0 }
    }

    /// Whether the list known by `url` is this repository's own.
    pub fn is_checkout(&self, url: &str) -> bool {
        self.checkout.as_deref() == Some(url)
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
        self.extensions.generation += 1;
        let generation = self.extensions.generation;
        self.extensions.reading |= network;
        cx.spawn(async move |this, cx| {
            let snapshot = cx.background_spawn(async move { Snapshot::read(&service, &lists, network, fake.as_ref()) }).await;
            this.update(cx, |h, cx| {
                if h.extensions.generation != generation {
                    return;
                }
                h.extensions.reading = false;
                h.extensions.snapshot = snapshot;
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
        let checkout = Some(PathBuf::from("/src/heronite/extensions/index.json"));
        let heronite = ListLocation::Address(HERONITE_LIST.into());
        assert_eq!(own_list(false, None, checkout.clone()), heronite, "a release build: always the Heronite list");
        assert_eq!(own_list(false, None, None), heronite);
        assert_eq!(own_list(true, None, checkout.clone()), ListLocation::File("/src/heronite/extensions/index.json".into()));
        assert_eq!(own_list(true, None, None), heronite, "a debug build away from its repository");
        // The switch wins in any build: an address or a path.
        let branch = "https://raw.githubusercontent.com/someone/Heronite/branch/extensions/index.json";
        for debug in [false, true] {
            assert_eq!(own_list(debug, Some(branch.into()), checkout.clone()), ListLocation::Address(branch.into()));
            assert_eq!(
                own_list(debug, Some("/home/me/list/index.json".into()), checkout.clone()),
                ListLocation::File("/home/me/list/index.json".into())
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
            installed: vec![installed("wikipedia", "1.0.0", Some("https://own")), installed("pcgamingwiki", "1.0.0", None)],
            in_use: vec![("https://own".into(), true)],
            copies: vec![own],
            updates: Vec::new(),
        };
        snap.updates = lists::updates(&snap.copies, &snap.installed);
        let offers = snap.offers();
        let state = |id: &str| offers.iter().find(|o| o.entry.id == id).map(|o| o.state.clone());
        assert_eq!(state("wikipedia"), Some(OfferState::Update("1.0.0".into())));
        assert_eq!(state("pcgamingwiki"), Some(OfferState::Installed), "installed from a file: never flagged");
        assert_eq!(state("indie-shelf"), Some(OfferState::New));
        assert_eq!(state("future-db"), Some(OfferState::Unsupported));
        let (n, single) = snap.more().unwrap();
        assert_eq!(n, offers.iter().filter(|o| o.state == OfferState::New).count());
        assert_eq!(single.map(|c| c.url.as_str()), Some("https://own"));

        snap.copies[0].index = None;
        snap.copies[0].error = Some(ListError::Status(404));
        assert!(snap.more().is_none(), "no copy: just the action");
    }
}
