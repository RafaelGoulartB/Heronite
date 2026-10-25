//! Achievements of every game, whatever its store: Steam (the client's files
//! on this PC), GOG and Epic (the signed-in accounts) and RetroAchievements
//! (ROMs, with the user's account).
//!
//! The app keeps its own copy of them (`store`), merged so it only grows
//! (`merge`): history survives a store going away, a reinstall or a new PC.
//! Every function here is blocking; the app calls them from worker threads.

pub mod accounts;
pub mod emulators;
pub mod merge;
pub mod migrate;
pub mod model;
pub mod prefs;
pub mod sources;
pub mod store;
pub mod sync;
pub mod target;

pub use model::{Achievement, AchievementSet, GameAchievements, Kind, Progress, Source};
pub use sync::{refresh, Outcome, Problem};
pub use target::{targets_of, SourceTarget};
