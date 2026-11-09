//! System information for Settings → About and the logs: OS, CPU, memory and
//! GPUs. On Linux they come from `/proc`, `/sys` and the PCI id database, on
//! Windows from the registry (`heronite_platform::system`). Nothing is spawned.

// The readers of `/proc` and `/sys` and their parsers have no caller on Windows; the parsers
// are plain text functions, so their tests still run there.
#![cfg_attr(windows, allow(dead_code))]

use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gpu {
    pub vendor: String,
    pub name: String,
    pub driver: String,
    /// Proprietary driver version (NVIDIA), when known.
    pub driver_version: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SystemInfo {
    pub os: String,
    pub kernel: String,
    pub cpu: String,
    pub cores: usize,
    pub memory_kb: u64,
    pub gpus: Vec<Gpu>,
    pub session: String,
}

/// `PRETTY_NAME` of an os-release file.
pub fn os_pretty_name(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|l| {
        let v = l.strip_prefix("PRETTY_NAME=")?;
        Some(v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

/// First `model name` of `/proc/cpuinfo`.
pub fn cpu_model(cpuinfo: &str) -> Option<String> {
    cpuinfo.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == "model name").then(|| v.split_whitespace().collect::<Vec<_>>().join(" "))
    })
}

/// `MemTotal` of `/proc/meminfo`, in kB.
pub fn mem_total_kb(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:")?.split_whitespace().next()?.parse().ok())
}

pub fn human_memory(kb: u64) -> String {
    format!("{:.1} GB", kb as f64 / 1024. / 1024.)
}

/// [`human_memory`] with the decimal separator of the interface language.
pub fn memory_label(kb: u64) -> String {
    format!("{} GB", heronite_i18n::decimal(kb as f64 / 1024. / 1024., 1))
}

/// Version out of the first line of `/proc/driver/nvidia/version`
/// ("NVRM version: NVIDIA UNIX x86_64 Kernel Module  580.82.09  Thu ...").
pub fn nvidia_version(text: &str) -> Option<String> {
    let line = text.lines().next()?;
    let after = line.split("Kernel Module").nth(1)?;
    // The open driver words it "Open Kernel Module for x86_64  580.82.09".
    after.split_whitespace().find(|t| t.starts_with(|c: char| c.is_ascii_digit()) && t.contains('.')).map(str::to_string)
}

pub fn vendor_name(id: &str) -> &'static str {
    match id.trim().trim_start_matches("0x").to_ascii_lowercase().as_str() {
        "10de" => "NVIDIA",
        "1002" | "1022" => "AMD",
        "8086" => "Intel",
        "1414" => "Microsoft",
        "15ad" => "VMware",
        "1af4" => "Red Hat (virtio)",
        _ => "Unknown vendor",
    }
}

/// Looks `vendor`/`device` (hex, lowercase, no `0x`) up in a `pci.ids` file.
/// Returns `(vendor name, device name)`.
pub fn pci_names(ids: &str, vendor: &str, device: &str) -> Option<(String, String)> {
    let mut vendor_name: Option<String> = None;
    for line in ids.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if vendor_name.is_some() {
                return None; // left the vendor block without a match
            }
            if let Some((id, name)) = line.split_once("  ") {
                if id == vendor {
                    vendor_name = Some(name.trim().to_string());
                }
            }
        } else if let (Some(v), false) = (&vendor_name, line.starts_with("\t\t")) {
            if let Some((id, name)) = line.trim_start().split_once("  ") {
                if id == device {
                    return Some((v.clone(), name.trim().to_string()));
                }
            }
        }
    }
    None
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn hex_id(path: impl AsRef<Path>) -> Option<String> {
    read(path).map(|s| s.trim().trim_start_matches("0x").to_ascii_lowercase())
}

/// GPUs from `/sys/class/drm/card*` (display class devices only).
pub fn read_gpus() -> Vec<Gpu> {
    let ids = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"].iter().find_map(read);
    let nvidia = read("/proc/driver/nvidia/version").and_then(|t| nvidia_version(&t));
    let Ok(rd) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut cards: Vec<_> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit())
        })
        .collect();
    cards.sort_by_key(|e| e.file_name());
    let mut out: Vec<Gpu> = Vec::new();
    for card in cards {
        let dev = card.path().join("device");
        let (Some(vendor), Some(device)) = (hex_id(dev.join("vendor")), hex_id(dev.join("device"))) else { continue };
        let driver =
            fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
        let (vname, name) = ids
            .as_deref()
            .and_then(|t| pci_names(t, &vendor, &device))
            .unwrap_or_else(|| (vendor_name(&vendor).to_string(), format!("device {device}")));
        let driver_version = (driver == "nvidia").then(|| nvidia.clone()).flatten();
        let gpu = Gpu { vendor: vname, name, driver, driver_version };
        if !out.contains(&gpu) {
            out.push(gpu);
        }
    }
    out
}

#[cfg(not(windows))]
pub fn gather() -> SystemInfo {
    SystemInfo {
        os: read("/etc/os-release").and_then(|t| os_pretty_name(&t)).unwrap_or_else(|| std::env::consts::OS.to_string()),
        kernel: read("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default(),
        cpu: read("/proc/cpuinfo").and_then(|t| cpu_model(&t)).unwrap_or_default(),
        cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        memory_kb: read("/proc/meminfo").and_then(|t| mem_total_kb(&t)).unwrap_or(0),
        gpus: read_gpus(),
        session: {
            let kind = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            [kind, desktop].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" / ")
        },
    }
}

/// The same through the registry and `GlobalMemoryStatusEx`. The Windows build stands where
/// Linux has the kernel version: "Windows 11 Pro 24H2" and "build 26100.4061". There is no
/// session type to name, and a card's driver is named by its maker.
#[cfg(windows)]
pub fn gather() -> SystemInfo {
    use heronite_platform::system;
    let version = system::windows_version();
    SystemInfo {
        os: version.as_ref().map_or_else(|| "Windows".to_string(), system::WindowsVersion::label),
        kernel: version.map(|v| format!("build {}", v.build_label())).unwrap_or_default(),
        cpu: system::processor_name().unwrap_or_default(),
        cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        memory_kb: system::total_memory_bytes().map_or(0, |bytes| bytes / 1024),
        gpus: system::graphics_cards()
            .into_iter()
            .map(|card| Gpu {
                driver: card.vendor.clone(),
                vendor: card.vendor,
                name: card.name,
                driver_version: Some(card.driver_version).filter(|v| !v.is_empty()),
            })
            .collect(),
        session: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_and_os_files() {
        assert_eq!(os_pretty_name("NAME=Arch\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n").as_deref(), Some("Arch Linux"));
        assert_eq!(cpu_model("processor\t: 0\nmodel name\t: AMD  Ryzen 7   5800X 8-Core\n").as_deref(), Some("AMD Ryzen 7 5800X 8-Core"));
        assert_eq!(mem_total_kb("MemTotal:       32768000 kB\nMemFree: 1 kB\n"), Some(32768000));
        assert_eq!(human_memory(16 * 1024 * 1024), "16.0 GB");
        assert_eq!(
            nvidia_version("NVRM version: NVIDIA UNIX x86_64 Kernel Module  580.82.09  Thu Aug  7 2025\nGCC version: gcc").as_deref(),
            Some("580.82.09")
        );
        assert_eq!(
            nvidia_version("NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.82.09  Release Build").as_deref(),
            Some("580.82.09")
        );
        assert_eq!(vendor_name("0x10de"), "NVIDIA");
        assert_eq!(vendor_name("1002"), "AMD");
    }

    #[cfg(windows)]
    #[test]
    fn this_windows_describes_itself() {
        let s = gather();
        assert!(s.os.starts_with("Windows "), "{s:?}");
        assert!(s.kernel.starts_with("build ") && s.kernel.contains('.'), "{s:?}");
        assert!(!s.cpu.is_empty(), "{s:?}");
        assert!(s.cores >= 1, "{s:?}");
        assert!(s.memory_kb >= 1024 * 1024, "{s:?}");
        assert!(s.session.is_empty(), "{s:?}");
        for gpu in &s.gpus {
            assert!(!gpu.name.is_empty() && gpu.driver == gpu.vendor, "{s:?}");
        }
        // What the logs print for it.
        let text = crate::pages::settings::logs::format_system_info(&s);
        assert!(text.contains(&format!("OS: {} {} (windows)", s.os, s.kernel)), "{text}");
        assert!(!text.contains("none detected") || s.gpus.is_empty(), "{text}");
    }

    #[test]
    fn pci_ids_lookup() {
        let ids = "# comment\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]\n\t\t1002 0b36  subsystem\n\t73df  Navi 22\n10de  NVIDIA Corporation\n\t2684  AD102 [GeForce RTX 4090]\n";
        assert_eq!(pci_names(ids, "10de", "2684"), Some(("NVIDIA Corporation".into(), "AD102 [GeForce RTX 4090]".into())));
        assert_eq!(pci_names(ids, "1002", "73df").map(|x| x.1).as_deref(), Some("Navi 22"));
        assert_eq!(pci_names(ids, "1002", "ffff"), None);
        assert_eq!(pci_names(ids, "8086", "0001"), None);
    }
}
