//! Visual tokens, grouped by kind. Everything is re-exported here, so callers
//! use `theme::X` whatever the file.
//!
//! - [`color`]: palette (Heroic's `nord-dark`), text, controls and status colours.
//! - [`font`]: families and weights.
//! - [`layout`]: sizes and radii (px at a 16px root unless noted).
//! - [`glass`]: the runtime appearance settings and the glass surfaces derived from them.
//! - [`scheme`]: light or dark, and the screens that stay dark.
//! - [`component`]: the gpui-component theme.
//!
//! Never hard-code one of these values at a call site.

mod color;
mod component;
mod font;
mod glass;
mod layout;
mod scheme;

pub use color::*;
pub use component::apply_component_theme;
pub use font::*;
pub use glass::*;
pub use layout::*;
pub use scheme::*;

/// `HERONITE_NO_ANIMATIONS=1` turns every animation off whatever Settings →
/// Appearance → Motion says (also needed for off-screen screenshots, where
/// the compositor stops sending frames and fades would stay at opacity 0).
fn motion_forced_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| crate::env::flag("HERONITE_NO_ANIMATIONS"))
}

/// Fades and transitions of the interface: on only with Motion "Full".
pub fn animations() -> bool {
    !motion_forced_off() && appearance().motion == heronite_data::appearance::Motion::Full
}

/// The background's cross-fade and the covers fading in: kept by Motion
/// "Reduced" too.
pub fn fades() -> bool {
    !motion_forced_off() && appearance().motion != heronite_data::appearance::Motion::Off
}
