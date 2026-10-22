//! Debug only: a wizard over an installer folder on disk
//! (`HERONITE_DEBUG_FOMOD=<folder holding fomod/ModuleConfig.xml>`).

use std::path::Path;
use std::sync::{Arc, Mutex};

use heronite_mods::fomod::{self, FileState, Wizard};

use crate::services::mods::FomodRequest;

/// The request the service would hand over for the installer in `dir`.
pub fn debug_request(dir: &Path, game_key: &str) -> Result<FomodRequest, String> {
    let root = fomod::find(dir).ok_or_else(|| "no fomod/ModuleConfig.xml in that folder".to_string())?;
    let installer = fomod::load(&root)?;
    let wizard = Wizard::new(installer, Box::new(|_| FileState::Missing), None);
    let mod_name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(FomodRequest { job_id: 0, game_key: game_key.to_string(), mod_name, wizard: Arc::new(Mutex::new(wizard)) })
}
