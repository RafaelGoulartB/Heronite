//! What the Linux tools print, turned into [`crate::Monitor`]s. Plain parsing,
//! compiled and tested on every platform; running the tools is
//! `crate::linux`.

pub mod hyprland;
pub mod kscreen;
pub mod mutter;
pub mod wlr;
