//! Amazon Games through the `nile` CLI. Everything here is blocking: call it
//! from worker threads.
//!
//! * [`cmd`]: running nile (binary lookup, `NILE_CONFIG_PATH`, streaming, cancel).
//! * [`user`]: sign-in, sign-out and the account kept in `nile_store/config.json`.
//! * [`library`]: `store_cache/nile_library.json`, the games the Collection shows.
//! * [`installed`]: nile's `installed.json`.
//! * [`info`]: download sizes (`nile install --info`).
//! * [`ops`]: install / update (as a download queue job), verify, uninstall,
//!   import and move.
//! * [`updates`]: `nile list-updates` and the saved result.
//!
//! nile's own files (`nile_config/nile/**`) are written by nile; the only
//! edits made here are removing an entry from `installed.json` when nile
//! cannot uninstall, and rewriting a game's `path` after a move.

pub mod cmd;
pub mod info;
pub mod installed;
pub mod library;
mod moves;
pub mod ops;
pub mod updates;
pub mod user;
