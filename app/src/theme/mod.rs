//! Visual tokens, grouped by kind. Everything is re-exported here, so callers
//! use `theme::X` whatever the file.
//!
//! - [`color`]: palette (Heroic's `nord-dark`), text, controls and status colours.
//! - [`font`]: families and weights.
//! - [`layout`]: sizes and radii (px at a 16px root unless noted).
//! - [`glass`]: the runtime appearance settings and the glass surfaces derived from them.
//! - [`component`]: the gpui-component theme.
//!
//! Never hard-code one of these values at a call site.

mod color;
mod component;
mod font;
mod glass;
mod layout;

pub use color::*;
pub use component::apply_component_theme;
pub use font::*;
pub use glass::*;
pub use layout::*;

/// Fades and transitions on/off. `HERONITE_NO_ANIMATIONS=1` disables them
/// (reduced motion; also needed for off-screen screenshots, where the
/// compositor stops sending frames and fades would stay at opacity 0).
pub fn animations() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !crate::env::flag("HERONITE_NO_ANIMATIONS"))
}
