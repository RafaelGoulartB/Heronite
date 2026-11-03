//! The Epic Games Store (port of Heroic's `backend/storeManagers/legendary/**`
//! and the download queue), on [`heronite_epic_api`] (Epic's services and
//! legendary's files) and [`heronite_depot`] (downloads). Everything here is
//! blocking: call it from worker threads.
//!
//! * [`client`]: [`Epic`], the handle every operation goes through.
//! * [`user`]: login / logout and the account kept in `store/config.json`.
//! * [`library`]: refreshing the library, `store_cache/legendary_library.json`,
//!   update detection.
//! * [`info`]: install information, selective downloads, API overrides.
//! * [`download`]: installs, updates and repairs written by heronite-depot.
//! * [`ops`]: install / update / repair / uninstall / move / import.
//! * [`moves`]: the resumable install move and its journal.
//! * [`queue`], [`manager`]: the download queue (`store/download-manager.json`).
//! * [`progress`]: what the queue shows of a download (and the progress
//!   lines of gogdl and nile).
//! * [`saves`]: cloud saves.
//! * [`playtime`]: Epic's play time for the games never played here.
//! * [`session`]: a sign-in that expired, for every store.
//! * [`autoupdate`]: which updates `autoUpdateGames` may queue now.
//! * [`eos_overlay`]: the EOS Overlay.
//! * [`third_party`]: games of the EA app and Ubisoft Connect.
//! * [`egl`]: games the Epic Games Launcher installed (Windows).
//! * [`cmd`]: running a store's command-line tool (legendary still starts
//!   the games; gogdl and nile).
//!
//! The account, library and installs stay in legendary's files
//! (`legendaryConfig/legendary/**`), in its formats, so installs made with
//! legendary carry over; `third-party-installed.json` there is Heroic's.

pub mod autoupdate;
pub mod cache_store;
pub mod client;
pub mod cmd;
pub mod disk;
pub mod download;
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

pub use client::Epic;
