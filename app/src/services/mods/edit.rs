//! Changes to a game's mod list. The list is shared with the job threads
//! (`GameShared`); a change is applied to it at once, the screen follows, and
//! the file is written in the background: always from the latest list, one
//! writer at a time, so a slow write never puts an older list back.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_mods::games::Profile;
use heronite_mods::{overwrite, store, GameDirs, GameLinks, ModEntry, ModList, ModsRoot};

use super::{ModsEvent, ModsService};

struct Slot {
    list: ModList,
    loaded: bool,
}

/// A game's list as the service and the job threads share it.
pub(super) struct GameShared {
    slot: Mutex<Slot>,
    /// Held while `mods.json` or a mod folder is being written.
    io: Mutex<()>,
    /// Held by the install that is running (including the wait for a FOMOD answer).
    install: Mutex<()>,
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl GameShared {
    pub fn new() -> Self {
        Self { slot: Mutex::new(Slot { list: ModList::default(), loaded: false }), io: Mutex::new(()), install: Mutex::new(()) }
    }

    /// The list, read from disk the first time. Blocking.
    pub fn ensure_loaded(&self, root: &ModsRoot, key: &str, title: &str) -> ModList {
        if !locked(&self.slot).loaded {
            // Read outside the lock: the screen never waits for the disk.
            let disk = store::load_or_new(root, key, title);
            let mut slot = locked(&self.slot);
            if !slot.loaded {
                slot.list = disk;
                slot.loaded = true;
            }
        }
        let mut slot = locked(&self.slot);
        if !title.is_empty() && slot.list.title != title {
            slot.list.title = title.to_string();
        }
        slot.list.clone()
    }

    pub fn list(&self) -> ModList {
        locked(&self.slot).list.clone()
    }

    #[cfg(test)]
    pub fn is_loaded(&self) -> bool {
        locked(&self.slot).loaded
    }

    /// Changes the list in memory only; gives what `change` returned and the list.
    pub fn edit<R>(&self, change: impl FnOnce(&mut ModList) -> R) -> (R, ModList) {
        let mut slot = locked(&self.slot);
        let result = change(&mut slot.list);
        (result, slot.list.clone())
    }

    /// Puts a mod a job built into the list (see [`merge_installed`]).
    pub fn merge(&self, entry: ModEntry, replace: Option<&str>) {
        merge_installed(&mut locked(&self.slot).list, entry, replace);
    }

    /// Writes the latest list.
    pub fn save(&self, dirs: &GameDirs) -> io::Result<()> {
        let _io = locked(&self.io);
        let snapshot = self.list();
        store::save(dirs, &snapshot)
    }

    pub fn lock_io(&self) -> MutexGuard<'_, ()> {
        locked(&self.io)
    }

    pub fn try_install(&self) -> Option<MutexGuard<'_, ()>> {
        match self.install.try_lock() {
            Ok(g) => Some(g),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// A mod built by a job goes where the one it replaces was (and keeps its
/// switch); a new one goes last.
pub(super) fn merge_installed(list: &mut ModList, mut entry: ModEntry, replace: Option<&str>) {
    let at = replace.and_then(|r| list.mods.iter().position(|m| m.id == r)).or_else(|| list.mods.iter().position(|m| m.id == entry.id));
    match at {
        Some(pos) => {
            entry.enabled = list.mods[pos].enabled;
            list.mods[pos] = entry;
        }
        None => list.mods.push(entry),
    }
}

impl ModsService {
    pub(super) fn shared_for(&mut self, game_key: &str) -> Arc<GameShared> {
        self.shared.entry(game_key.to_string()).or_insert_with(|| Arc::new(GameShared::new())).clone()
    }

    /// Writes the list in the background; a failure becomes a notice.
    pub(super) fn save_later(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let Some(shared) = self.shared.get(game_key).cloned() else { return };
        let dirs = self.root.game(game_key);
        let key = game_key.to_string();
        cx.spawn(async move |this, cx| {
            if let Err(e) = cx.background_spawn(async move { shared.save(&dirs) }).await {
                log::warn!("mods: could not save the list of {key}: {e}");
                this.update(cx, |_, cx| {
                    cx.emit(ModsEvent::Notice {
                        game_key: key,
                        message: tr!("Could not save the list of mods: {error}", error = e),
                        error: true,
                    })
                })
                .ok();
            }
        })
        .detach();
    }

    /// Applies `change` to the loaded game's list at once. When it says it
    /// changed something: the screen follows and the file is saved.
    fn edit_list(&mut self, game_key: &str, check: bool, change: impl FnOnce(&mut ModList) -> bool, cx: &mut Context<Self>) -> bool {
        let (Some(shared), Some(game)) = (self.shared.get(game_key).cloned(), self.games.get_mut(game_key)) else {
            log::warn!("mods: a change to {game_key}, whose list is not loaded");
            return false;
        };
        let (changed, list) = shared.edit(change);
        if !changed {
            return false;
        }
        game.list = list;
        cx.emit(ModsEvent::Changed { game_key: game_key.to_string() });
        if check {
            self.recheck(game_key, cx);
        }
        self.save_later(game_key, cx);
        true
    }

    /// The view follows the shared list after a job changed it.
    pub(super) fn refresh_from_shared(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let Some(shared) = self.shared.get(game_key).cloned() else { return };
        let list = shared.list();
        match self.games.get_mut(game_key) {
            Some(game) => game.list = list,
            None => {
                self.counts.insert(game_key.to_string(), (list.mods.len(), list.mods.iter().filter(|m| m.enabled).count()));
            }
        }
        cx.emit(ModsEvent::Changed { game_key: game_key.to_string() });
        self.recheck(game_key, cx);
    }

    pub(super) fn edit_active(&mut self, game_key: &str, on: bool, cx: &mut Context<Self>) {
        let written = self.games.get(game_key).map(|g| g.overwrite).unwrap_or_default();
        let changed = self.edit_list(
            game_key,
            false,
            |l| {
                let changed = l.active != on;
                l.active = on;
                changed
            },
            cx,
        );
        if changed && !on && !written.is_empty() {
            self.notice(game_key, tr!("What the game wrote while modded (settings, saves) stays in Overwrite. The game will not see it until mods are on again, unless you turn it into a mod."), false, cx);
        }
        if changed {
            self.recheck(game_key, cx);
        }
    }

    pub(super) fn edit_enabled(&mut self, game_key: &str, mod_id: &str, on: bool, cx: &mut Context<Self>) {
        self.edit_list(
            game_key,
            true,
            |l| match l.get_mut(mod_id) {
                Some(m) if m.enabled != on => {
                    m.enabled = on;
                    true
                }
                _ => false,
            },
            cx,
        );
    }

    pub(super) fn edit_move(&mut self, game_key: &str, mod_id: &str, to_index: usize, cx: &mut Context<Self>) {
        self.edit_list(
            game_key,
            true,
            |l| {
                let before: Vec<String> = l.mods.iter().map(|m| m.id.clone()).collect();
                store::move_mod(l, mod_id, to_index);
                before.iter().ne(l.mods.iter().map(|m| &m.id))
            },
            cx,
        );
    }

    pub(super) fn edit_rename(&mut self, game_key: &str, mod_id: &str, name: String, cx: &mut Context<Self>) {
        self.edit_list(
            game_key,
            false,
            |l| {
                let before = l.get(mod_id).map(|m| m.name.clone());
                store::rename_mod(l, mod_id, &name);
                l.get(mod_id).map(|m| m.name.clone()) != before
            },
            cx,
        );
    }

    pub(super) fn edit_plugin(&mut self, game_key: &str, plugin: &str, on: bool, cx: &mut Context<Self>) {
        let plugin = plugin.to_lowercase();
        self.edit_list(
            game_key,
            false,
            |l| {
                let at = l.disabled_plugins.iter().position(|p| p.eq_ignore_ascii_case(&plugin));
                match (on, at) {
                    (true, Some(i)) => {
                        l.disabled_plugins.remove(i);
                        true
                    }
                    (false, None) => {
                        l.disabled_plugins.push(plugin);
                        true
                    }
                    _ => false,
                }
            },
            cx,
        );
    }

    /// Runs `then` unless the game runs with its mods (a notice says why
    /// not). The mount table is read off the UI thread.
    pub(super) fn unless_mounted(
        &mut self,
        game_key: &str,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let (root, game_dir, key) =
            (self.root.clone(), self.games.get(game_key).and_then(|g| g.info.game_dir.clone()), game_key.to_string());
        cx.spawn(async move |this, cx| {
            let refused = cx.background_spawn(async move { refuse_while_mounted(&root, game_dir.as_deref()) }).await;
            this.update(cx, |s, cx| match refused {
                Ok(()) => then(s, cx),
                Err(message) => cx.emit(ModsEvent::Notice { game_key: key, message, error: true }),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn edit_remove(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        let (key, id) = (game_key.to_string(), mod_id.to_string());
        self.unless_mounted(game_key, cx, move |s, cx| s.remove_now(&key, &id, cx));
    }

    /// Takes the mod off the list at once; its folder and archive go in the background.
    fn remove_now(&mut self, game_key: &str, mod_id: &str, cx: &mut Context<Self>) {
        let Some(shared) = self.shared.get(game_key).cloned() else { return };
        if let Some(flagged) = self.flagged.get_mut(game_key) {
            flagged.remove(mod_id);
        }
        // The folder is removed from a copy that still has the entry.
        let mut with_entry = shared.list();
        if !self.edit_list(game_key, true, |l| remove_entry(l, mod_id), cx) {
            return;
        }
        let (dirs, id, key) = (self.root.game(game_key), mod_id.to_string(), game_key.to_string());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let _io = shared.lock_io();
                    store::remove_mod(&dirs, &mut with_entry, &id)
                })
                .await;
            if let Err(e) = result {
                this.update(cx, |_, cx| {
                    cx.emit(ModsEvent::Notice {
                        game_key: key,
                        message: tr!("Could not delete the files of the mod: {error}", error = e),
                        error: true,
                    })
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn edit_links(&mut self, game_key: &str, links: GameLinks, cx: &mut Context<Self>) {
        let new = links.clone();
        if !self.edit_list(
            game_key,
            false,
            |l| {
                let changed = l.links != links;
                l.links = links;
                changed
            },
            cx,
        ) {
            return;
        }
        // A cleared link falls back to what the sites say; the profile follows.
        if let Some(game) = self.games.get_mut(game_key) {
            game.info.nexus_domain = new.nexus_domain.clone();
            game.info.thunderstore = new.thunderstore.clone();
            game.info.profile = Profile::of(&new, game.info.known);
        }
        self.find_links(game_key, false, cx);
    }

    pub(super) fn empty_overwrite(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let Some(shared) = self.shared.get(game_key).cloned() else { return };
        let (dirs, game_dir, key) =
            (self.root.game(game_key), self.games.get(game_key).and_then(|g| g.info.game_dir.clone()), game_key.to_string());
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    refuse_while_mounted(&root, game_dir.as_deref())?;
                    let _io = shared.lock_io();
                    overwrite::clear(&dirs).map_err(|e| tr!("Could not empty the overwrite folder: {error}", error = e))
                })
                .await;
            this.update(cx, |s, cx| {
                if let Err(message) = result {
                    cx.emit(ModsEvent::Notice { game_key: key.clone(), message, error: true });
                }
                s.recheck(&key, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn turn_overwrite_into_mod(&mut self, game_key: &str, name: String, cx: &mut Context<Self>) {
        let Some(shared) = self.shared.get(game_key).cloned() else { return };
        let (dirs, game_dir, key) =
            (self.root.game(game_key), self.games.get(game_key).and_then(|g| g.info.game_dir.clone()), game_key.to_string());
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    refuse_while_mounted(&root, game_dir.as_deref())?;
                    let _turn = loop {
                        if let Some(g) = shared.try_install() {
                            break g;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    };
                    let mut scratch = shared.list();
                    let id = {
                        let _io = shared.lock_io();
                        overwrite::into_mod(&dirs, &mut scratch, name.trim())?
                    };
                    let entry = scratch.get(&id).cloned().ok_or_else(|| tr!("The mod was not created").to_string())?;
                    shared.merge(entry, None);
                    shared.save(&dirs).map_err(|e| tr!("Could not save the list of mods: {error}", error = e))
                })
                .await;
            this.update(cx, |s, cx| {
                if let Err(message) = result {
                    cx.emit(ModsEvent::Notice { game_key: key.clone(), message, error: true });
                }
                s.refresh_from_shared(&key, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Deletes everything the mods root keeps for the game.
    pub(super) fn forget(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let key = game_key.to_string();
        self.unless_mounted(game_key, cx, move |s, cx| s.forget_now(&key, cx));
    }

    fn forget_now(&mut self, game_key: &str, cx: &mut Context<Self>) {
        self.flagged.remove(game_key);
        let (root, key) = (self.root.clone(), game_key.to_string());
        let game_dir = self.games.get(game_key).and_then(|g| g.info.game_dir.clone());
        cx.spawn(async move |this, cx| {
            let k = key.clone();
            let result = cx.background_spawn(async move { store::forget_game(&root, &k, game_dir.as_deref()) }).await;
            this.update(cx, |s, cx| match result {
                Ok(()) => {
                    s.games.remove(&key);
                    s.shared.remove(&key);
                    s.counts.remove(&key);
                    cx.emit(ModsEvent::Changed { game_key: key });
                }
                Err(message) => cx.emit(ModsEvent::Notice { game_key: key, message, error: true }),
            })
            .ok();
        })
        .detach();
    }
}

fn remove_entry(list: &mut ModList, id: &str) -> bool {
    let before = list.mods.len();
    list.mods.retain(|m| m.id != id);
    list.mods.len() != before
}

/// While the overlay is mounted (the game runs with its mods) the mods' files
/// are in use and what the game writes lives in the upper layer: nothing of
/// them is deleted, replaced or emptied. Blocking (reads the mount table).
pub(super) fn refuse_while_mounted(root: &ModsRoot, game_dir: Option<&std::path::Path>) -> Result<(), String> {
    if game_dir.is_some_and(|d| heronite_mods::deploy::is_ours_mounted(root, d)) {
        return Err(tr!("The game is running with mods; close it first").to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{merge_installed, refuse_while_mounted, GameShared};
    use heronite_mods::{deploy, store, tool, ModEntry, ModList, ModsRoot};

    fn entry(id: &str, enabled: bool) -> ModEntry {
        ModEntry { id: id.into(), name: id.into(), enabled, ..Default::default() }
    }

    #[test]
    fn a_new_mod_goes_last_and_a_replacement_keeps_place_and_switch() {
        let mut list = ModList::new("g", "G");
        list.mods = vec![entry("a", true), entry("b", false), entry("c", true)];
        merge_installed(&mut list, entry("d", true), None);
        assert_eq!(list.mods.iter().map(|m| m.id.as_str()).collect::<String>(), "abcd");
        merge_installed(&mut list, entry("b2", true), Some("b"));
        let ids: Vec<_> = list.mods.iter().map(|m| (m.id.as_str(), m.enabled)).collect();
        assert_eq!(ids, vec![("a", true), ("b2", false), ("c", true), ("d", true)]);
        // The replaced mod was removed meanwhile: the new one goes last.
        merge_installed(&mut list, entry("e", true), Some("gone"));
        assert_eq!(list.mods.last().map(|m| m.id.as_str()), Some("e"));
        // Same id again (a reinstall): in place.
        merge_installed(&mut list, entry("a", false), None);
        assert_eq!((list.mods[0].id.as_str(), list.mods[0].enabled), ("a", true));
        assert_eq!(list.mods.len(), 5);
    }

    #[test]
    fn edits_apply_to_the_shared_list_and_save_the_latest() {
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path());
        let shared = GameShared::new();
        assert!(!shared.is_loaded());
        let list = shared.ensure_loaded(&root, "g", "Game");
        assert_eq!((list.game_key.as_str(), list.title.as_str()), ("g", "Game"));
        shared.merge(entry("a", true), None);
        let (changed, copy) = shared.edit(|l| {
            if let Some(m) = l.get_mut("a") {
                m.enabled = false;
            }
            true
        });
        assert!(changed && !copy.mods[0].enabled);
        shared.save(&root.game("g")).unwrap();
        assert_eq!(store::load(&root.game("g")).mods.len(), 1);
        assert!(!store::load(&root.game("g")).mods[0].enabled);
    }

    /// Needs FUSE and fuse-overlayfs (`HERONITE_FUSE_OVERLAYFS=...`); skipped without.
    #[test]
    fn nothing_is_deleted_while_the_game_runs_with_its_mods() {
        let nowhere = std::path::Path::new("/nonexistent");
        let Some(bin) = tool::find(nowhere).filter(|_| std::path::Path::new("/dev/fuse").exists() && tool::fusermount().is_some()) else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let root = ModsRoot::at(t.path().join("mods"));
        let dirs = root.game("g");
        let game = t.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::create_dir_all(dirs.mod_dir("a")).unwrap();
        assert!(refuse_while_mounted(&root, Some(&game)).is_ok());
        assert!(refuse_while_mounted(&root, None).is_ok());
        let mount = deploy::mount(&bin, &dirs, &game, &[dirs.mod_dir("a")]).unwrap();
        let said = refuse_while_mounted(&root, Some(&game)).unwrap_err();
        assert_eq!(said, "The game is running with mods; close it first");
        drop(mount);
        assert!(refuse_while_mounted(&root, Some(&game)).is_ok());
    }
}
