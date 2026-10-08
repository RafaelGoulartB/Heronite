//! The Epic side of the first launch of a game (prerequisites, EOS overlay).

use heronite_data::HeroicPaths;
use heronite_epic::cmd::Legendary;
use heronite_epic::eos_overlay;
use heronite_epic::info;

/// What the first launch of an Epic game in a prefix asks legendary for
/// (the launcher crate cannot depend on the Epic crate): the game's
/// prerequisites and the EOS overlay of the prefix.
pub struct EpicLaunchSetup(pub HeroicPaths);

impl heronite_launch::first_launch::EpicSetup for EpicLaunchSetup {
    fn prerequisites(&self, app_name: &str, platform: &str) -> Result<Option<heronite_launch::first_launch::Prerequisite>, String> {
        let lg = Legendary::locate(&self.0).map_err(|e| format!("{e:#}"))?;
        let info = info::install_info(&self.0, &lg, app_name, platform).map_err(|e| format!("{e:#}"))?;
        Ok(info.prerequisites().map(|(path, args)| heronite_launch::first_launch::Prerequisite { path, args }))
    }

    fn eos_overlay_installed(&self) -> bool {
        eos_overlay::status(&self.0).installed
    }

    fn eos_overlay_enabled(&self, prefix: &str) -> bool {
        Legendary::locate(&self.0).map(|lg| eos_overlay::is_enabled(&lg, Some(prefix))).unwrap_or(false)
    }

    fn eos_overlay_enable(&self, prefix: &str) -> Result<(), String> {
        let lg = Legendary::locate(&self.0).map_err(|e| format!("{e:#}"))?;
        let r = eos_overlay::enable(&lg, Some(prefix));
        if r.ok { Ok(()) } else { Err(r.message) }
    }
}
