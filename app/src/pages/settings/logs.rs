//! Settings → Logs: the launcher's own log, Heroic's logs and the log of
//! every game, in the shared log viewer (`dialogs/log_viewer.rs`).
//!
//! Also here: the system information text that opens the launcher log and
//! every game's launch log (Heroic's `formatSystemInfo`).
//!
//! Debug: `HERONITE_DEBUG_LOG=launcher|<appName>` picks the log shown first.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::sysinfo;
use super::SettingsPage;
use crate::dialogs::log_viewer::{LogViewer, Scope, Start};

#[derive(Default)]
pub struct State {
    viewer: Option<Entity<LogViewer>>,
}

impl State {
    /// Called when the page comes back: logs may have been written meanwhile.
    pub fn refresh(&mut self, cx: &mut Context<SettingsPage>) {
        if let Some(v) = self.viewer.clone() {
            v.update(cx, |v, cx| v.refresh(cx));
        }
    }
}

impl State {
    /// Drops the viewer (it holds up to 32 MiB of lines): the section was left
    /// or the page hidden. Shown again, it reads the logs from the start.
    pub fn release(&mut self) {
        self.viewer = None;
    }
}

/// Where the viewer starts: `HERONITE_DEBUG_LOG` or the newest log.
fn start() -> Start {
    match crate::env::value("HERONITE_DEBUG_LOG").as_deref() {
        Some("launcher") => Start::Launcher,
        Some(app) => Start::Game(app.to_string()),
        _ => Start::First,
    }
}

impl SettingsPage {
    pub(super) fn logs_section(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.logs.viewer.is_none() {
            let data_paths = self.ctx.data_paths.clone();
            let viewer = cx.new(|cx| LogViewer::new(data_paths, Scope::All, start(), true, None, cx));
            self.logs.viewer = Some(viewer);
        }
        self.logs.viewer.clone().expect("created above").into_any_element()
    }
}

/// Console mode: the logs viewer handles its own rows (see `console.rs`).
pub(super) fn console_rows(_: &SettingsPage) -> Vec<crate::console::list::Row> {
    Vec::new()
}

pub(super) fn console_apply(_: &mut SettingsPage, _: crate::console::list::Outcome, _: &mut Context<SettingsPage>) {}

impl SettingsPage {
    /// The viewer, created on first use (console mode).
    pub(super) fn logs_viewer(&mut self, cx: &mut Context<Self>) -> Entity<LogViewer> {
        if self.logs.viewer.is_none() {
            let data_paths = self.ctx.data_paths.clone();
            self.logs.viewer = Some(cx.new(|cx| LogViewer::new(data_paths, Scope::All, start(), true, None, cx)));
        }
        self.logs.viewer.clone().expect("created above")
    }
}

/// Heroic's `formatSystemInfo` for this machine: CPU, memory, GPUs and OS.
pub fn system_info_text() -> String {
    format_system_info(&sysinfo::gather())
}

pub fn format_system_info(s: &sysinfo::SystemInfo) -> String {
    let mut out = vec![
        format!("Heronite {}", env!("CARGO_PKG_VERSION")),
        format!("CPU: {}x {}", s.cores, s.cpu),
        format!("Memory: {}", sysinfo::human_memory(s.memory_kb)),
        "GPUs:".to_string(),
    ];
    if s.gpus.is_empty() {
        out.push("  none detected".to_string());
    }
    for (i, g) in s.gpus.iter().enumerate() {
        out.push(format!("  GPU {i}:"));
        out.push(format!("    Name: {} {}", g.vendor, g.name));
        let driver = match &g.driver_version {
            Some(v) => format!("{} {v}", g.driver),
            None => g.driver.clone(),
        };
        out.push(format!("    Driver: {driver}"));
    }
    out.push(format!("OS: {} {} ({})", s.os, s.kernel, std::env::consts::OS));
    if !s.session.is_empty() {
        out.push(format!("Session: {}", s.session));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui prelude has its own `test` attribute.
    use super::format_system_info;
    use crate::pages::settings::sysinfo::{Gpu, SystemInfo};

    #[test]
    fn system_info_reads_like_heroics() {
        let s = SystemInfo {
            os: "Arch Linux".into(),
            kernel: "6.9.1".into(),
            cpu: "AMD Ryzen 5".into(),
            cores: 12,
            memory_kb: 32 * 1024 * 1024,
            gpus: vec![Gpu { vendor: "NVIDIA".into(), name: "RTX".into(), driver: "nvidia".into(), driver_version: Some("580.1".into()) }],
            session: "wayland / Hyprland".into(),
        };
        let text = format_system_info(&s);
        assert!(text.contains("CPU: 12x AMD Ryzen 5\nMemory: 32.0 GB\nGPUs:\n  GPU 0:\n    Name: NVIDIA RTX\n    Driver: nvidia 580.1\nOS: Arch Linux 6.9.1 (linux)"), "{text}");
        assert!(text.ends_with("Session: wayland / Hyprland"));
    }
}
