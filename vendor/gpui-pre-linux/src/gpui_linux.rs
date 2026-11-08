#![cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

pub use linux::{current_platform, hide_cursor_until_mouse_moves, linux_platform, show_cursor};
