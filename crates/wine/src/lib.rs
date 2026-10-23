//! Wine/Proton manager and compatibility layer detection (port of Heroic's
//! `backend/wine/manager/**` and `backend/utils/compatibility_layers.ts`).

pub mod auto;
pub mod checks;
pub mod default;
mod detect;
pub mod github;
pub mod install;
pub mod manager;
pub mod prefixes;
pub mod repos;
pub mod steam_link;
pub mod store;
mod types;
mod vdf;

pub use checks::{CheckMode, CheckOutcome};
pub use detect::{list_installations, list_installations_with, steam_libraries, DetectEnv};
pub use manager::{Job, JobEvent, JobKind, JobPhase, WineManager};
pub use repos::Repository;
pub use types::{Progress, WineConfig, WineInstallation, WineVersionInfo};
