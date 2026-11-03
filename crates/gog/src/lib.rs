//! GOG natively ([`heronite_gog_api`] for GOG's protocol, [`heronite_depot`]
//! for the downloads) and GOG's web APIs. It keeps the files gogdl kept, so
//! installs made with gogdl carry on. Everything here is blocking: call it
//! from worker threads.
//!
//! * [`user`]: login / logout and the account kept in `gog_store/config.json`.
//! * [`library`]: owned games, `store_cache/gog_library.json`.
//! * [`installed`]: the installed games, kept in `gog_store/installed.json`.
//! * [`info`]: install information (sizes, languages, DLCs).
//! * [`download`]: writing an install, update or repair to disk.
//! * [`ops`]: install / update (queue job), repair, uninstall, import, move.
//! * [`updates`]: which installed games have a newer build.
//! * [`saves`]: cloud saves.
//! * [`playtime`]: GOG's play time for the games never played here.
//! * [`setup`]: the Windows programs to run at the first launch (in the Wine
//!   prefix on Linux, as they are on Windows).
//! * [`galaxy`]: games GOG Galaxy installed on this PC (Windows).

pub mod download;
pub mod galaxy;
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

pub use heronite_gog_api::stored::{manifest_file, support_dir};

#[cfg(test)]
pub(crate) mod testing {
    use heronite_data::DataPaths;

    pub fn setup() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        (dir, paths)
    }
}
