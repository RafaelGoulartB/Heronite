//! The graphics card that draws Heronite's windows (Linux; Settings →
//! Advanced → Troubleshooting). GPUI picks the card the compositor reports,
//! else the dedicated one. On a hybrid laptop that can be the card the screen
//! is not on, and every frame is then copied between the two cards, which
//! makes scrolling stutter (COSMIC on hybrid NVIDIA laptops). The saved card
//! is handed to GPUI through `ZED_DEVICE_ID`, which it reads when it creates
//! its GPU context: before the first window, so a change needs a restart.
//!
//! A card the compositor cannot show leaves the window black. A new choice is
//! therefore on trial (`heronite_data::interface_gpu`): kept only once the
//! user confirms it, else the next start goes back to automatic.

use std::sync::{Mutex, OnceLock};

use heronite_data::interface_gpu::{self, InterfaceGpu};
use heronite_data::AppPaths;
use heronite_i18n::tr;

/// GPUI's override: a PCI device id in hex.
const ENV: &str = "ZED_DEVICE_ID";

/// What this run draws with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    /// `None`: automatic, or a `ZED_DEVICE_ID` from the environment.
    pub gpu: Option<InterfaceGpu>,
    /// The card is on trial: not confirmed yet.
    pub trial: bool,
    /// `ZED_DEVICE_ID` came from the environment, so the setting is unused.
    pub from_environment: bool,
}

static APPLIED: OnceLock<Applied> = OnceLock::new();
/// The toast of this start (a card on trial, or one given up), once.
static NOTICE: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// Before the first window (and before the app starts its threads): hands
/// the saved card to GPUI.
pub fn apply(paths: &AppPaths) {
    let applied = if cfg!(target_os = "linux") { choose(paths) } else { Applied::default() };
    let _ = APPLIED.set(applied);
}

fn choose(paths: &AppPaths) -> Applied {
    if let Some(v) = std::env::var_os(ENV) {
        log::info!("interface GPU: {ENV}={} from the environment", v.to_string_lossy());
        return Applied { from_environment: true, ..Default::default() };
    }
    let Some(saved) = interface_gpu::load(paths) else { return Applied::default() };
    if !saved.confirmed {
        if saved.tried {
            // The last start drew with it and nobody kept it: maybe it showed nothing.
            log::warn!("interface GPU: {} was tried but not kept, back to automatic", saved.gpu.id());
            if let Err(e) = interface_gpu::save(paths, None) {
                log::warn!("interface GPU: could not reset the choice: {e:#}");
            }
            set_notice(tr!("The graphics card chosen for the interface was not kept, so Heronite went back to Automatic."), true);
            return Applied::default();
        }
        if let Err(e) = interface_gpu::mark_tried(paths, saved) {
            // Without the mark a black window would stay black: do not try it.
            log::warn!("interface GPU: could not mark {} as tried, not using it: {e:#}", saved.gpu.id());
            return Applied::default();
        }
        set_notice(
            tr!("Heronite now draws with the graphics card you chose. If it looks right, keep it in Settings → Advanced; otherwise the next start goes back to Automatic."),
            false,
        );
    }
    std::env::set_var(ENV, format!("{:04x}", saved.gpu.device_id));
    log::info!("interface GPU: {}{} (Settings → Advanced)", saved.gpu.id(), if saved.confirmed { "" } else { ", on trial" });
    Applied { gpu: Some(saved.gpu), trial: !saved.confirmed, from_environment: false }
}

fn set_notice(text: impl Into<String>, error: bool) {
    *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) = Some((text.into(), error));
}

/// What this run draws with.
pub fn applied() -> Applied {
    APPLIED.get().copied().unwrap_or_default()
}

/// The notice of this start, for the Collection's first toast, once.
pub fn take_notice() -> Option<(String, bool)> {
    NOTICE.lock().unwrap_or_else(|e| e.into_inner()).take()
}
