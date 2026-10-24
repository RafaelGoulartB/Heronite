//! Settings → Storage: what the games take on disk and cleaning it up
//! (`heronite_storage`). Nothing is read until the section is shown: then the
//! inventory is listed, drawn at once, and the sizes no store or manifest
//! gives are walked one item at a time (cached in `install-sizes.json`). The
//! walk stops when another section is shown. Removing something rescans.
//!
//! Debug: `HERONITE_DEBUG_PAGE=settings:storage`; `HERONITE_DEBUG_SWEEPS_OPEN=1`
//! opens every "Free up space" choice.

mod console;
mod render;
mod treemap;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_data::Library;
use heronite_i18n::{tr, trn};
use heronite_images::ImageRef;
use heronite_storage::{Ctx, Group, Inventory, Item, Kind, Removed, State as Use, Sweep};

pub(super) use console::{console_apply, console_rows};
pub(super) use render::{render, MAX_W};

use super::SettingsPage;
use crate::pages::{PageCtx, PageEvent};
use crate::services::widgets::Covers;
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameSort {
    Size,
    Played,
}

/// A part of a disk's bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seg {
    Group(Group),
    /// Used by everything else on the disk.
    Other,
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Hover {
    /// A game's treemap tile (item id).
    Tile(String),
    /// A part of disk `.0`'s bar.
    Segment(usize, Seg),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Confirm {
    /// "Free up space" with the ticked choices.
    Sweep,
    /// One item (id).
    Item(String),
}

pub struct State {
    ctx: Ctx,
    inv: Option<Inventory>,
    /// The library the Collection loaded, with its art (covers, wide art).
    lib: Option<Arc<Library>>,
    chains: HashMap<SharedString, Vec<ImageRef>>,
    stages: HashMap<SharedString, Vec<ImageRef>>,
    /// Made the first time the section is shown.
    covers: Option<Covers>,
    /// Bumped to stop the scan running.
    generation: Arc<AtomicU64>,
    /// Listing, or walking `(done, total)`.
    loading: bool,
    walking: Option<(usize, usize)>,
    removing: bool,
    sweeps: HashSet<Sweep>,
    /// "Free up space" choices showing what they remove.
    open_sweeps: HashSet<Sweep>,
    confirm: Option<Confirm>,
    sort: GameSort,
    /// Groups showing every row.
    expanded: HashSet<Group>,
    hover: Option<Hover>,
    /// Console mode: the row whose next A removes it.
    armed: Option<SharedString>,
}

impl State {
    pub fn new(ctx: &PageCtx) -> Self {
        Self {
            ctx: Ctx::new(ctx.data_paths.clone(), ctx.app.clone()),
            inv: None,
            lib: None,
            chains: HashMap::new(),
            stages: HashMap::new(),
            covers: None,
            generation: Arc::default(),
            loading: false,
            walking: None,
            removing: false,
            sweeps: Sweep::ALL.into_iter().filter(|s| s.default_on()).collect(),
            open_sweeps: if crate::env::flag("HERONITE_DEBUG_SWEEPS_OPEN") { Sweep::ALL.into_iter().collect() } else { HashSet::new() },
            confirm: None,
            sort: GameSort::Size,
            expanded: HashSet::new(),
            hover: None,
            armed: None,
        }
    }

    /// The Collection's library and art. `true` when the library changed.
    pub fn set_library(
        &mut self,
        lib: Arc<Library>,
        chains: HashMap<SharedString, Vec<ImageRef>>,
        stages: HashMap<SharedString, Vec<ImageRef>>,
    ) -> bool {
        self.chains = chains;
        self.stages = stages;
        if self.lib.as_ref().is_some_and(|l| Arc::ptr_eq(l, &lib)) {
            return false;
        }
        self.lib = Some(lib);
        true
    }

    /// Stops the walk running (another section is shown).
    pub fn stop(&mut self) {
        if self.loading {
            self.generation.fetch_add(1, Ordering::Relaxed);
            self.loading = false;
            self.walking = None;
        }
    }

    /// The section left the screen (another section, or the page was hidden):
    /// stops the walk and frees the cover textures, which load again from the
    /// disk cache when it is shown. The covers stay in place: their loader
    /// still hands over results.
    pub fn hidden(&mut self, window: &mut Window) {
        self.stop();
        if let Some(covers) = self.covers.as_mut() {
            covers.release(window);
        }
    }

    /// Section shown: lists everything again and walks what has no size.
    pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
        if d.storage.covers.is_none() {
            let covers = Covers::new(&d.ctx.data_paths, &d.ctx.app, cx, |p: &mut SettingsPage| {
                p.storage.covers.as_mut().expect("made before any image is requested")
            });
            d.storage.covers = Some(covers);
        }
        scan(d, cx);
    }

    fn items(&self) -> impl Iterator<Item = &Item> {
        self.inv.iter().flat_map(|i| i.items.iter())
    }

    fn item(&self, id: &str) -> Option<&Item> {
        self.items().find(|i| i.id == id)
    }

    /// Bytes `item` takes by itself (without games installed inside it).
    fn own(&self, item: &Item) -> Option<u64> {
        self.inv.as_ref().and_then(|inv| inv.own_size(item))
    }

    fn group_items(&self, group: Group) -> Vec<&Item> {
        self.items().filter(|i| i.group == group).collect()
    }

    fn group_bytes(&self, group: Group) -> u64 {
        self.items().filter(|i| i.group == group).filter_map(|i| self.own(i)).sum()
    }

    /// Items and bytes a "Free up space" choice takes.
    fn sweep_parts(&self, sweep: Sweep) -> (Vec<&Item>, u64) {
        let items: Vec<&Item> = self.items().filter(|i| i.sweep() == Some(sweep)).collect();
        let bytes = items.iter().filter_map(|i| self.own(i)).sum();
        (items, bytes)
    }

    /// Bytes the ticked choices free.
    fn ticked_bytes(&self) -> u64 {
        self.sweeps.iter().map(|s| self.sweep_parts(*s).1).sum()
    }

    /// The parts of disk `disk`'s bar: each group on it, the rest of what is
    /// used, and the free space.
    fn disk_parts(&self, disk: usize) -> Vec<(Seg, u64)> {
        let Some(d) = self.inv.as_ref().and_then(|i| i.disks.get(disk)) else { return Vec::new() };
        let mut parts: Vec<(Seg, u64)> = Group::ALL
            .iter()
            .map(|g| (Seg::Group(*g), self.items().filter(|i| i.group == *g && i.disk == Some(disk)).filter_map(|i| self.own(i)).sum()))
            .filter(|(_, b)| *b > 0)
            .collect();
        let ours: u64 = parts.iter().map(|(_, b)| *b).sum();
        parts.push((Seg::Other, d.used().saturating_sub(ours)));
        parts.push((Seg::Free, d.free));
        parts
    }

    /// Removing: other removals wait. Measuring does not block them (a
    /// removal stops it and lists everything again).
    fn busy(&self) -> bool {
        self.removing
    }
}

/// Lists everything (off the UI thread), then walks the items without a size.
fn scan(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let st = &mut d.storage;
    let mine = st.generation.fetch_add(1, Ordering::Relaxed) + 1;
    st.loading = true;
    st.walking = None;
    let generation = st.generation.clone();
    let ctx = st.ctx.clone();
    let lib = st.lib.clone();
    // Shown while they are measured again.
    let before: HashMap<String, u64> = st.items().filter_map(|i| Some((i.id.clone(), i.size?))).collect();
    cx.notify();
    cx.spawn(async move |this, cx| {
        let c = ctx.clone();
        let inv = cx
            .background_spawn(async move {
                let lib = match lib {
                    Some(lib) => lib,
                    None => Arc::new(Library::load(&c.data_paths).map_err(|e| log::warn!("storage: loading the library: {e:#}")).ok()?),
                };
                Some(heronite_storage::inventory(&c, &lib))
            })
            .await;
        let current = |g: &Arc<AtomicU64>| g.load(Ordering::Relaxed) == mine;
        let Some(mut inv) = inv else {
            this.update(cx, |d, cx| {
                if current(&d.storage.generation) {
                    d.storage.loading = false;
                }
                cx.notify();
            })
            .ok();
            return;
        };
        for item in inv.items.iter_mut().filter(|i| i.size.is_none()) {
            item.size = before.get(&item.id).copied();
        }
        // The small things first, the game folders (the long walks) last.
        let mut walks: Vec<Item> = inv.items.iter().filter(|i| i.needs_walk).cloned().collect();
        walks.sort_by_key(|i| (i.group == Group::Games, i.group));
        let total = walks.len();
        let alive = this
            .update(cx, |d, cx| {
                if current(&d.storage.generation) {
                    d.storage.inv = Some(inv);
                    d.storage.walking = Some((0, total));
                    cx.notify();
                }
            })
            .is_ok();
        if !alive {
            return;
        }
        for (n, item) in walks.into_iter().enumerate() {
            if !current(&generation) {
                return;
            }
            let (c, g) = (ctx.clone(), generation.clone());
            let measured = cx
                .background_spawn(async move {
                    let size = heronite_storage::measure(&c, &item, &|| g.load(Ordering::Relaxed) != mine);
                    size.map(|s| (item.id, s))
                })
                .await;
            let alive = this
                .update(cx, |d, cx| {
                    if !current(&d.storage.generation) {
                        return;
                    }
                    if let Some((id, size)) = measured {
                        if let Some(it) = d.storage.inv.as_mut().and_then(|inv| inv.items.iter_mut().find(|i| i.id == id)) {
                            it.size = Some(size);
                        }
                    }
                    d.storage.walking = Some((n + 1, total));
                    cx.notify();
                })
                .is_ok();
            if !alive {
                return;
            }
        }
        this.update(cx, |d, cx| {
            if current(&d.storage.generation) {
                d.storage.loading = false;
                d.storage.walking = None;
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

/// Removes the items `ids` (off the UI thread), says what was freed, then
/// lists everything again.
fn remove(d: &mut SettingsPage, ids: Vec<String>, cx: &mut Context<SettingsPage>) {
    let st = &mut d.storage;
    let items: Vec<Item> = st.items().filter(|i| ids.contains(&i.id) && i.removable()).cloned().collect();
    st.confirm = None;
    st.armed = None;
    if items.is_empty() {
        cx.notify();
        return;
    }
    st.stop();
    st.removing = true;
    let ctx = st.ctx.clone();
    cx.notify();
    cx.spawn(async move |this, cx| {
        let (done, errors) = cx
            .background_spawn(async move {
                let mut done = Removed::default();
                let mut errors: Vec<String> = Vec::new();
                for item in &items {
                    match heronite_storage::remove(&ctx, item) {
                        Ok(r) => {
                            done.deleted += r.deleted;
                            done.trashed += r.trashed;
                        }
                        Err(e) => {
                            log::warn!("storage: removing {}: {e:#}", item.path().display());
                            errors.push(format!("{e:#}"));
                        }
                    }
                }
                (done, errors)
            })
            .await;
        this.update(cx, |d, cx| {
            d.storage.removing = false;
            let (text, error) = match errors.first() {
                Some(first) if errors.len() > 1 => {
                    (trn!(errors.len(), "{n} item could not be removed: {first}", "{n} items could not be removed: {first}", first), true)
                }
                Some(first) => (first.clone(), true),
                None => (removed_text(done), false),
            };
            cx.emit(PageEvent::Toast(text, error));
            scan(d, cx);
        })
        .ok();
    })
    .detach();
}

/// "12.3 GB freed", "4.1 GB moved to the trash", or both.
fn removed_text(r: Removed) -> String {
    match (r.deleted, r.trashed) {
        (0, 0) => tr!("Done").into(),
        (d, 0) => tr!("{size} freed", size = human_size(d)),
        (0, t) => tr!("{size} moved to the trash", size = human_size(t)),
        (d, t) => tr!("{freed} freed, {trashed} moved to the trash", freed = human_size(d), trashed = human_size(t)),
    }
}

/// Opens an item's folder (a file's folder for a file).
fn open(item: &Item) {
    let path = item.path();
    let dir = if path.is_file() { path.parent().unwrap_or(path) } else { path };
    if let Err(e) = super::open_folder(dir) {
        log::warn!("xdg-open {}: {e}", dir.display());
    }
}

fn group_label(g: Group) -> &'static str {
    match g {
        Group::Games => tr!("Games"),
        Group::Prefixes => tr!("Wine prefixes"),
        Group::Builds => tr!("Proton and runtimes"),
        Group::Steam => tr!("Steam leftovers"),
        Group::Caches => tr!("Caches"),
        Group::Leftovers => tr!("Leftover folders"),
    }
}

fn seg_label(s: Seg) -> &'static str {
    match s {
        Seg::Group(g) => group_label(g),
        Seg::Other => tr!("Everything else"),
        Seg::Free => tr!("Free"),
    }
}

fn group_color(g: Group) -> Hsla {
    match g {
        Group::Games => theme::chart_series(0),
        Group::Prefixes => theme::chart_series(5),
        Group::Builds => theme::chart_series(2),
        Group::Steam => theme::chart_series(1),
        Group::Caches => theme::chart_series(3),
        Group::Leftovers => theme::chart_series(4),
    }
    .into()
}

fn seg_color(s: Seg) -> Hsla {
    match s {
        Seg::Group(g) => group_color(g),
        Seg::Other => theme::chart_other(),
        Seg::Free => theme::chart_empty(),
    }
}

fn sweep_label(s: Sweep) -> &'static str {
    match s {
        Sweep::Prefixes => tr!("Unused Wine prefixes"),
        Sweep::Steam => tr!("Steam leftovers"),
        Sweep::Builds => tr!("Unused Proton and Wine"),
        Sweep::Leftovers => tr!("Leftover folders"),
        Sweep::Caches => tr!("Caches"),
        Sweep::Shaders => tr!("Compiled shaders"),
    }
}

/// What freeing a choice does, over the list of what it takes.
fn sweep_effect(s: Sweep) -> &'static str {
    match s {
        Sweep::Prefixes => tr!("No installed game uses these prefixes. They go to the trash with the saves and settings inside them, and can be restored from there."),
        Sweep::Steam => tr!("Steam keeps these for games no longer installed. Proton prefixes go to the trash, where they can be restored; shader caches are deleted."),
        Sweep::Builds => tr!("No installed game uses these builds. They are uninstalled and can be downloaded again from the Wine Manager."),
        Sweep::Leftovers => tr!("No game points to these folders any more. They go to the trash, where they can be restored."),
        Sweep::Caches => tr!("Deleted. They are rebuilt by themselves when needed."),
        Sweep::Shaders => tr!("Deleted. Games compile them again as they run, and stutter for a while until they do."),
    }
}

fn sweep_color(s: Sweep) -> Hsla {
    group_color(match s {
        Sweep::Prefixes => Group::Prefixes,
        Sweep::Steam => Group::Steam,
        Sweep::Builds => Group::Builds,
        Sweep::Leftovers => Group::Leftovers,
        Sweep::Caches | Sweep::Shaders => Group::Caches,
    })
}

/// "Unused" amber, the colour of what can go.
fn unused_color() -> Rgba {
    theme::status_color("on-hold")
}

/// The size, or a dash while it is measured.
fn size_text(bytes: Option<u64>) -> String {
    bytes.map(human_size).unwrap_or_else(|| "\u{2014}".into())
}

/// Days since an RFC 3339 / ISO date (`None`: never or unreadable).
fn days_since(date: Option<&str>) -> Option<i64> {
    let date = date?;
    let day = match chrono::DateTime::parse_from_rfc3339(date) {
        Ok(d) => d.with_timezone(&chrono::Local).date_naive(),
        Err(_) => chrono::NaiveDate::parse_from_str(date.get(..10)?, "%Y-%m-%d").ok()?,
    };
    Some((chrono::Local::now().date_naive() - day).num_days().max(0))
}

/// "Played today", "Played 3 weeks ago", "Never played".
fn played_text(days: Option<i64>) -> String {
    match days {
        None => tr!("Never played").into(),
        Some(0) => tr!("Played today").into(),
        Some(1) => tr!("Played yesterday").into(),
        Some(d @ 2..=13) => tr!("Played {n} days ago", n = d),
        Some(d @ 14..=59) => tr!("Played {n} weeks ago", n = d / 7),
        Some(d @ 60..=729) => tr!("Played {n} months ago", n = d / 30),
        Some(d) => tr!("Played {n} years ago", n = d / 365),
    }
}

/// A game not played for half a year (or never): a candidate to uninstall.
fn stale(days: Option<i64>) -> bool {
    days.is_none_or(|d| d >= 180)
}

/// What an item is, in a few words, under its title.
fn item_meta(st: &State, item: &Item) -> String {
    let titles =
        |keys: &[String]| -> Vec<String> { keys.iter().filter_map(|k| st.lib.as_ref()?.get(k).map(|g| g.title.clone())).collect() };
    match &item.kind {
        Kind::Game { .. } => {
            let played = played_text(days_since(item.last_played.as_deref()));
            match &item.source {
                Some(source) => format!("{source} \u{b7} {played}"),
                None => played,
            }
        }
        Kind::Prefix { .. } | Kind::Build { .. } if !item.games.is_empty() => join_titles(&titles(&item.games)),
        Kind::Prefix { default: true } | Kind::Build { default: true, .. } => tr!("Games without a choice of their own use it").into(),
        Kind::Build { .. } if item.state == Use::InUse => tr!("Used by Steam").into(),
        Kind::Prefix { .. } | Kind::Build { .. } => tr!("No installed game uses it").into(),
        Kind::Runtime => tr!("Needed to run games").into(),
        Kind::SteamPrefix { .. } => tr!("Proton prefix of a game no longer installed").into(),
        Kind::SteamShaders { .. } => tr!("Shader cache of a game no longer installed").into(),
        Kind::Cache(_) => tr!("Rebuilt when needed").into(),
        Kind::Leftover => item.path().display().to_string(),
    }
}

/// "Control", "Control, Hades", "Control, Hades and 3 more".
fn join_titles(titles: &[String]) -> String {
    match titles {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => tr!("{a} and {b}", a, b),
        [a, b, rest @ ..] => trn!(rest.len(), "{a}, {b} and {n} more", "{a}, {b} and {n} more", a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::{days_since, stale};

    #[test]
    fn played_and_stale() {
        assert!(stale(None));
        assert!(stale(Some(400)));
        assert!(!stale(Some(30)));
        assert_eq!(days_since(Some("nope")), None);
        let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
        assert_eq!(days_since(Some(&today)), Some(0));
        assert_eq!(days_since(Some(&format!("{today}T10:00:00.000Z"))).map(|d| d <= 1), Some(true));
    }
}
