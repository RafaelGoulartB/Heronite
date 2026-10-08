//! Data layer shared with Heronite: same files under `~/.config/heroic`.

pub mod json_store;
pub mod models;
pub mod num;
pub mod paths;
pub mod text;

pub mod art;
pub mod collection;
pub mod config_games;
pub mod game_config;
pub mod focus;
pub mod heronite_ui_import;
pub mod hltb;
pub mod library;
pub mod playtime;
pub mod sessions;
pub mod settings;
pub mod sideload;
pub mod status;
pub mod ui_state;

pub use art::{ArtContext, ImageRef};
pub use collection::{
    FacetFilters, FacetKind, InstallFilter, SortMode, StatusGroup, ViewQuery,
};
pub use library::{CollectionGame, Library};
pub use num::JsNum;
pub use paths::{AppPaths, HeroicPaths};

#[cfg(test)]
mod roundtrip_tests {
    //! Run with `HERONITE_FIXTURE=/tmp/heronite-fixture cargo test -p heronite-data`
    //! against a copy of real `~/.config/heroic` data.
    use crate::json_store;
    use crate::models::*;
    use serde::{de::DeserializeOwned, Serialize};
    use std::path::PathBuf;

    fn check<T: DeserializeOwned + Serialize + Default>(root: &std::path::Path, rel: &str) {
        let p = root.join(rel);
        if !p.exists() {
            eprintln!("skip {rel}");
            return;
        }
        let original = json_store::read_value(&p).unwrap();
        let typed: T = json_store::read(&p).unwrap_or_else(|e| panic!("{rel}: {e:#}"));
        let back = serde_json::to_value(&typed).unwrap();
        assert!(original == back, "{rel} did not round-trip");
    }

    #[test]
    fn real_files_round_trip() {
        let Some(root) = std::env::var_os("HERONITE_FIXTURE").map(PathBuf::from) else {
            eprintln!("HERONITE_FIXTURE not set; skipping");
            return;
        };
        check::<LocalMetaFile>(&root, "local_library/library.json");
        check::<SessionsFile>(&root, "local_library/sessions.json");
        check::<StatusesFile>(&root, "local_library/statuses.json");
        check::<CollectionSettingsFile>(&root, "local_library/settings.json");
        check::<MetadataFile>(&root, "local_library/metadata.json");
        check::<ArtFile>(&root, "local_library/art.json");
        check::<TimestampFile>(&root, "store/timestamp.json");
        check::<HeroicConfigFile>(&root, "store/config.json");
        check::<SideloadLibraryFile>(&root, "sideload_apps/library.json");
    }
}
