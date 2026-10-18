//! GOG through the `gogdl` CLI and GOG's web APIs. Everything here is
//! blocking: call it from worker threads.
//!
//! * [`cmd`]: running gogdl (binary lookup, auth file, config folder, cancel).
//! * [`user`]: login / logout and the account kept in `gog_store/config.json`.
//! * [`library`]: owned games, `store_cache/gog_library.json`.
//! * [`installed`]: the installed games, kept in `gog_store/installed.json`.
//! * [`info`]: `gogdl info` (sizes, languages, DLCs).
//! * [`ops`]: install / update (queue job), repair, uninstall, import, move.
//! * [`updates`]: which installed games have a newer build.
//! * [`saves`]: cloud saves through `gogdl save-sync`.
//! * [`playtime`]: GOG's play time for the games never played here.
//! * [`setup`]: Windows programs to run in the Wine prefix at the first launch.

pub mod cmd;
pub mod info;
pub mod installed;
pub mod library;
pub mod ops;
pub mod playtime;
pub mod saves;
pub mod setup;
pub mod updates;
pub mod user;

mod api;
mod disk;
mod gamesdb;
mod lang;
mod moves;
