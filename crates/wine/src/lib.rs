//! Wine/Proton manager and compatibility layer detection (port of Heroic's
//! `backend/wine/manager/**` and `backend/utils/compatibility_layers.ts`).

mod detect;
mod types;
mod vdf;

pub use detect::{list_installations, list_installations_with, steam_libraries, DetectEnv};
pub use types::{Progress, WineConfig, WineInstallation, WineVersionInfo};
