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
//! * [`moves`]: the resumable install move and its journal.
//! * [`queue`], [`manager`]: the download queue (`store/download-manager.json`).
//! * [`progress`]: legendary's download progress lines.
//! * [`saves`]: cloud saves.
//! * [`playtime`]: Epic's play time for the games never played here.
//! * [`session`]: a sign-in that expired, for every store.
//! * [`autoupdate`]: which updates `autoUpdateGames` may queue now.
//! * [`eos_overlay`]: the EOS Overlay (`legendary eos-overlay`).
//! * [`third_party`]: games of the EA app and Ubisoft Connect.
//! * [`egl`]: games the Epic Games Launcher installed (Windows).
//!
//! legendary's own files (`legendaryConfig/legendary/**`) are only ever
//! written by legendary; `third-party-installed.json` there is Heroic's.

pub mod autoupdate;
pub mod cache_store;
pub mod cmd;
pub mod disk;
pub mod egl;
pub mod eos_overlay;
pub mod info;
pub mod launch_options;
pub mod library;
pub mod manager;
pub mod moves;
pub mod ops;
pub mod playtime;
pub mod progress;
pub mod queue;
pub mod saves;
pub mod session;
pub mod third_party;
pub mod user;
pub mod util;
