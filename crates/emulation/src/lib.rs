//! Emulation and backups backend compatible with Heronite's data files.
//!
//! * [`catalog`], [`detect`], [`store`]: emulator catalog, detection (PATH,
//!   Flatpak, libretro cores) and user emulator / ROM scanner CRUD
//!   (`emulators.json`, `rom-scanners.json`).
//! * [`scan`], [`import`]: ROM scanning into a [`types::RomScanPreview`] and
//!   importing it as sideload games + overlay entries; auto-scan on refresh.
//! * [`disc_id`]: PS1/PS2/PSP/PS3/GameCube/Wii disc identifiers.
//! * [`save_states`]: per-engine save state listing, thumbnails and one-shot
//!   launch arguments.
//! * [`ludusavi`], [`backup`], [`settings_ext`]: Ludusavi save backups and the
//!   scheduled config backup, with their status fields in `settings.json`.
//! * [`jobs`]: the idle jobs schedule.

pub mod backup;
pub mod catalog;
pub mod detect;
pub mod discover;
pub mod disc_id;
pub mod import;
mod iso;
pub mod jobs;
pub mod manager;
pub mod ludusavi;
pub mod names;
pub mod pathutil;
pub mod save_states;
pub mod scan;
pub mod settings_ext;
pub mod sha1;
pub mod store;
pub mod sysfiles;
pub mod types;

pub use catalog::{emulator_definitions, EmulatorDefinition, EmulatorDefinitionProfile};
pub use detect::{add_detected_emulators, detect_emulators, DetectEnv, SystemEnv};
pub use import::{
    import_scanned_roms, run_auto_rom_scanners, run_rom_scanners, set_emulator_launch_rom, AutoScanReport,
};
pub use save_states::{
    list_emulator_save_states, remember_save_state_key, set_pending_launch_state, LaunchStateArgs,
    PendingLaunchStates,
};
pub use scan::{preview_rom_scan, scan_rom_directory};
pub use store::{
    duplicate_user_emulator, insert_user_emulator, replace_user_emulator, list_rom_scanners, list_user_emulators, remove_rom_scanner, remove_user_emulator,
    upsert_rom_scanner, upsert_user_emulator,
};
pub use sysfiles::HostEnv;
pub use types::*;

use anyhow::Result;
use heronite_data::HeroicPaths;

/// `getEmulationState`: catalog (without `custom`), the user's emulators and
/// scanners, and (when `refresh_detect`) what is installed on this machine.
pub fn get_emulation_state(
    paths: &HeroicPaths,
    env: &dyn DetectEnv,
    refresh_detect: bool,
) -> Result<EmulationState> {
    let emulators = store::list_user_emulators(paths)?;
    let detected = if refresh_detect {
        detect::detect_emulators(env, &emulators)
    } else {
        vec![]
    };
    Ok(EmulationState {
        catalog: catalog::emulator_definitions()
            .iter()
            .filter(|d| d.id != "custom")
            .cloned()
            .collect(),
        emulators,
        scanners: store::list_rom_scanners(paths)?,
        detected,
    })
}
