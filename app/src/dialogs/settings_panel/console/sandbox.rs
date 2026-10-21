//! The Sandbox tab in console mode: the same switch, home folder and
//! exceptions as the desktop tab.

use gpui_kit::SharedString;
use heronite_i18n::tr;
use heronite_launch::sandbox::Exception;

use super::{item, Act, Item};
use crate::console::list::Row;
use crate::dialogs::settings_panel::sandbox::{exception_id, exception_text};
use crate::dialogs::settings_panel::SettingsPanel;
use crate::icons::Icon;

impl SettingsPanel {
    pub(super) fn c_sandbox(&self) -> Vec<Item> {
        let s = self.sandbox();
        let mut v = vec![item(
            Row::toggle("t:sandbox", tr!("Run in a sandbox"), s.enabled)
                .desc(tr!("No network and no home folder: the game sees the system read-only, its prefix and its folder")),
            Act::Sandbox(None),
        )];
        if let Some(why) = self.sandbox_host.clone().flatten().filter(|_| s.enabled) {
            v.push(Self::c_info("sandbox-missing", tr!("Cannot run"), why));
        } else if let Some(home) = self.sandbox_home().filter(|_| s.enabled) {
            let open = home.clone();
            v.push(
                Self::c_do("sandbox-home", tr!("Open its home folder"), Icon::FolderOpen, true, move |_, _, _| {
                    let _ = std::fs::create_dir_all(&open);
                    crate::ui::open_external(&open);
                })
                .desc_item(&home),
            );
        }
        v.push(Self::c_header(tr!("Exceptions")));
        for e in Exception::ALL {
            let (label, fixes, risk) = exception_text(e);
            let row = Row::toggle(SharedString::from(format!("t:{}", exception_id(e))), label.to_string(), s.allows(e))
                .desc(format!("{fixes} {}", tr!("Risk: {risk}", risk)))
                .enabled(s.enabled);
            v.push(item(row, Act::Sandbox(Some(e))));
        }
        v
    }
}
