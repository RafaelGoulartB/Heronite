//! The mods service (placeholder until the service lands).

use gpui_kit::*;

/// An `nxm://` / `ror2mm://` link from the command line or another invocation.
pub fn open_link(_cx: &mut App, link: String) {
    log::info!("mod link: {link}");
}
