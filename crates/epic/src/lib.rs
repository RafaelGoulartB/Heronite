//! Epic Games Store through the `legendary` CLI (port of Heroic's
//! `backend/storeManagers/legendary/**` and the download queue). Everything
//! here is blocking: call it from worker threads.
//!
//! * [`cmd`]: running legendary (binary lookup, env, streaming, cancel).
//! * [`user`]: login / logout and the account kept in `store/config.json`.
//! * [`library`]: `legendary list`, `store_cache/legendary_library.json`,
//!   update detection.
//! * [`info`]: `legendary info`, selective downloads, API overrides.
//! * [`ops`]: install / update / repair / uninstall / move / import.
//! * [`queue`], [`manager`]: the download queue (`store/download-manager.json`).
//! * [`progress`]: legendary's download progress lines.
//! * [`saves`]: cloud saves.
//!
//! legendary's own files (`legendaryConfig/legendary/**`) are only ever
//! written by legendary.

pub mod cache_store;
pub mod cmd;
pub mod disk;
pub mod info;
pub mod library;
pub mod manager;
pub mod ops;
pub mod progress;
pub mod queue;
pub mod saves;
pub mod user;
pub mod util;
