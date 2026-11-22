//! Mods on the game's own drive (`heronite_mods::game_drive`): the question
//! each drive gets once, the moves it starts, and what Settings → Mods shows.
//! Windows only: there a game on another drive than its mods gets them
//! copied in at every launch, while Linux lays them over the game's folder
//! from wherever they are.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::*;
use heronite_data::{CollectionGame, Library};
use heronite_i18n::tr;
use heronite_mods::game_drive::{self, Choice, Destination, Placement};
use heronite_mods::paths::safe_name;
use heronite_mods::{store, ModsRoot};

use super::facts;
use super::jobs::Outcome;
use super::{JobStage, ModsEvent, ModsService};

/// One move at a time: each writes gigabytes to a disk.
static MOVES: Mutex<()> = Mutex::new(());

/// Whether mods may be kept on the game's drive on this system.
pub fn offered() -> bool {
    cfg!(windows)
}

/// What Settings → Mods shows: every drive that was asked, and its games.
#[derive(Clone, Debug, Default)]
pub struct DrivesInfo {
    pub drives: Vec<DriveRow>,
}

#[derive(Clone, Debug)]
pub struct DriveRow {
    /// `D:\`.
    pub volume: PathBuf,
    pub choice: Choice,
    /// `D:\.heronite-mods`.
    pub store: PathBuf,
    /// The games whose mods are in `store`.
    pub games: Vec<StoredGame>,
}

#[derive(Clone, Debug)]
pub struct StoredGame {
    /// The game's folder name in the mods root.
    pub key: String,
    pub title: String,
    pub bytes: u64,
    /// Still installed in the Collection; when not, Settings offers to delete.
    pub installed: bool,
}

/// Where the game's mods are against its folder; `Linked` where the question
/// does not arise.
pub(super) fn placement_of(root: &ModsRoot, game_key: &str, game_dir: Option<&Path>) -> Placement {
    match game_dir {
        Some(dir) if offered() => game_drive::placement(root, game_key, dir),
        _ => Placement::Linked,
    }
}

/// Blocking: the answers, the games stored on each drive and their sizes.
fn read_drives(paths: &heronite_data::DataPaths, root: &ModsRoot) -> DrivesInfo {
    let library = Library::load_with(paths, false).ok();
    let stored = game_drive::stored(root);
    let drives = game_drive::choices(root)
        .into_iter()
        .map(|c| {
            let store = game_drive::store_of(&c.volume);
            let games = stored
                .iter()
                .filter(|s| s.dir.parent() == Some(store.as_path()))
                .map(|s| {
                    let game = library.as_ref().and_then(|l| l.games.iter().find(|g| safe_name(&g.key) == s.key));
                    let list_title = store::load(&root.game(&s.key)).title;
                    let title = game.map(|g| g.title.clone()).filter(|t| !t.is_empty()).unwrap_or(if list_title.is_empty() {
                        s.key.clone()
                    } else {
                        list_title
                    });
                    // Without a readable library nothing is called uninstalled.
                    let installed = library.is_none() || game.is_some_and(|g| g.installed);
                    StoredGame { key: s.key.clone(), title, bytes: game_drive::size_of(&s.dir), installed }
                })
                .collect();
            DriveRow { volume: c.volume, choice: c.choice, store, games }
        })
        .collect();
    DrivesInfo { drives }
}

impl ModsService {
    pub fn drives(&self) -> &DrivesInfo {
        &self.drives
    }

    /// Reads [`drives`](Self::drives) again in the background.
    pub fn refresh_drives(&mut self, cx: &mut Context<Self>) {
        if !offered() {
            return;
        }
        let (paths, root) = (self.data_paths.clone(), self.root.clone());
        cx.spawn(async move |this, cx| {
            let info = cx.background_spawn(async move { read_drives(&paths, &root) }).await;
            this.update(cx, |s, cx| {
                s.drives = info;
                cx.emit(ModsEvent::Drives);
            })
            .ok();
        })
        .detach();
    }

    /// The user's answer for the games of `volume`. Every game with mods
    /// that the answer puts elsewhere starts moving, one at a time.
    pub fn answer_drive(&mut self, volume: PathBuf, choice: Choice, cx: &mut Context<Self>) {
        let (paths, root) = (self.data_paths.clone(), self.root.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    game_drive::set_choice(&root, &volume, choice).map_err(|e| e.to_string())?;
                    let mut moves: Vec<(CollectionGame, Destination)> = Vec::new();
                    for key in store::game_keys(&root) {
                        let Ok(game) = facts::find_game(&paths, &key) else { continue };
                        let dir = facts::resolve(&paths, &game).dir;
                        if let Placement::Move(to) = placement_of(&root, &key, dir.as_deref()) {
                            moves.push((game, to));
                        }
                    }
                    Ok::<_, String>(moves)
                })
                .await;
            this.update(cx, |s, cx| {
                match result {
                    Ok(moves) => {
                        for (game, to) in moves {
                            let key = game.key.clone();
                            s.sources.entry(key.clone()).or_insert_with(|| Arc::new(game));
                            s.start_move(&key, to, cx);
                        }
                    }
                    Err(e) => cx.emit(ModsEvent::Notice {
                        game_key: String::new(),
                        message: tr!("Could not keep the answer: {error}", error = e),
                        error: true,
                    }),
                }
                // The question leaves the managers of the drive's other games too.
                let loaded: Vec<Arc<CollectionGame>> = s.games.keys().filter_map(|k| s.sources.get(k).cloned()).collect();
                for game in loaded {
                    s.load_game(&game, cx);
                }
                s.refresh_drives(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Settings: deletes the mods a game that is no longer installed left on
    /// its drive.
    pub fn delete_stored(&mut self, folder_key: String, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let k = folder_key.clone();
            let result = cx.background_spawn(async move { store::forget_game(&root, &k, None) }).await;
            this.update(cx, |s, cx| {
                if let Err(message) = result {
                    cx.emit(ModsEvent::Notice { game_key: String::new(), message, error: true });
                }
                s.counts.remove(&folder_key);
                s.refresh_drives(cx);
            })
            .ok();
        })
        .detach();
    }

    /// A move of this game's mods is running.
    pub fn moving(&self, game_key: &str) -> bool {
        self.jobs.iter().any(|j| j.game_key == game_key && j.moving && super::is_running(j.stage))
    }

    /// While the game's mods move nothing else touches them: a notice, and `true`.
    pub(super) fn refuse_while_moving(&self, game_key: &str, cx: &mut Context<Self>) -> bool {
        if !self.moving(game_key) {
            return false;
        }
        cx.emit(ModsEvent::Notice {
            game_key: game_key.to_string(),
            message: tr!("The mods of this game are being moved. Try again when the move ends.").to_string(),
            error: true,
        });
        true
    }

    /// Starts moving the game's mods to `to`, unless another job of the game
    /// is running (it writes into the folder being copied).
    pub(super) fn start_move(&mut self, game_key: &str, to: Destination, cx: &mut Context<Self>) {
        if self.jobs.iter().any(|j| j.game_key == game_key && super::is_running(j.stage)) {
            cx.emit(ModsEvent::Notice {
                game_key: game_key.to_string(),
                message: tr!("The mods of {game} move to its drive once its downloads and installs end.", game = self.game_title(game_key)),
                error: false,
            });
            return;
        }
        let name = match &to {
            Destination::Home => tr!("Moving the mods to {path}", path = self.root.path().display()),
            Destination::Store(store) => tr!("Moving the mods to {path}", path = store.display()),
        };
        let title = self.game_title(game_key);
        let id = self.start_job(game_key, name, cx, move |env| {
            let _turn = env.install_turn().ok_or_else(|| tr!("Cancelled").to_string())?;
            let _io = env.shared.lock_io();
            let _one = MOVES.lock().unwrap_or_else(|e| e.into_inner());
            env.stage(JobStage::Moving);
            let dir = facts::resolve(&env.paths, &env.game).dir;
            let mut progress = env.progress();
            game_drive::relocate(&env.root, &env.key, dir.as_deref(), &to, &mut progress, &env.cancel)?;
            Ok(Outcome::Finished(match &to {
                Destination::Home => tr!("The mods of {game} are back in Heronite's folder", game = title),
                Destination::Store(_) => {
                    tr!("The mods of {game} are now on its drive: launches link them instead of copying", game = title)
                }
            }))
        });
        if let Some(job) = id.and_then(|id| self.jobs.iter_mut().find(|j| j.id == id)) {
            job.moving = true;
        }
    }

    /// The manager opened a game whose drive was answered "keep them on the
    /// game's drive" while its mods are elsewhere: they move now. Not again
    /// after a failed move, which stays in the list until dismissed.
    pub(super) fn move_if_answered(&mut self, game_key: &str, cx: &mut Context<Self>) {
        let Some(Placement::Move(to)) = self.games.get(game_key).map(|g| g.info.placement.clone()) else { return };
        if self.jobs.iter().any(|j| j.game_key == game_key && j.moving) {
            return;
        }
        self.start_move(game_key, to, cx);
    }
}
