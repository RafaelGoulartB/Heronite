//! The Epic side of the first launch of a game (prerequisites, EOS overlay).

use heronite_data::DataPaths;
use heronite_epic::eos_overlay;
use heronite_epic::info;
use heronite_epic::Epic;

/// What the first launch of an Epic game in a prefix needs of the Epic
/// crate (the launcher crate cannot depend on it): the game's
/// prerequisites and the EOS overlay of the prefix.
pub struct EpicLaunchSetup(pub DataPaths);

impl heronite_launch::first_launch::EpicSetup for EpicLaunchSetup {
    fn prerequisites(&self, app_name: &str, platform: &str) -> Result<Option<heronite_launch::first_launch::Prerequisite>, String> {
        let found = info::prerequisites(&self.0, &Epic::new(&self.0), app_name, platform).map_err(|e| format!("{e:#}"))?;
        Ok(found.map(|(path, args)| heronite_launch::first_launch::Prerequisite { path, args }))
    }

    fn eos_overlay_installed(&self) -> bool {
        eos_overlay::status(&self.0).installed
    }

    fn eos_overlay_enabled(&self, prefix: &str) -> bool {
        eos_overlay::is_enabled(&Epic::new(&self.0), Some(prefix))
    }

    fn eos_overlay_enable(&self, prefix: &str) -> Result<(), String> {
        let r = eos_overlay::enable(&Epic::new(&self.0), Some(prefix));
        if r.ok {
            Ok(())
        } else {
            Err(r.message)
        }
    }
}
