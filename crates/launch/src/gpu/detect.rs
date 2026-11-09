//! What this computer has for the GPU settings: its graphics cards (from
//! sysfs, named by `vulkaninfo` when it is installed, else by `pci.ids`),
//! the driver layers some settings need, and what a Proton build offers.
//!
//! [`Machine::detect`] blocks (it reads files and runs `vulkaninfo`, about
//! half a second): call it off the UI thread.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Vendor, AMD, INTEL, NVIDIA};
use crate::probe::Probe;

/// One graphics card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// PCI address, like `0000:01:00.0`.
    pub slot: String,
    pub vendor_id: u16,
    pub device_id: u16,
    /// Like `NVIDIA GeForce RTX 5070`.
    pub name: String,
    /// The kernel driver (`nvidia`, `amdgpu`, `i915`, `xe`, `nouveau`...).
    pub driver: String,
    /// The card the firmware started the screens on (`boot_vga`).
    pub primary: bool,
    /// From Vulkan; `None`: unknown.
    pub integrated: Option<bool>,
}

impl Card {
    pub fn vendor(&self) -> Option<Vendor> {
        Vendor::of_pci(self.vendor_id)
    }

    /// Whether the NVIDIA driver runs it (Smooth Motion, Reflex, its shader
    /// cache); nouveau / NVK cards are Mesa's.
    pub fn nvidia_driver(&self) -> bool {
        self.vendor_id == NVIDIA && self.driver == "nvidia"
    }

    /// Whether Mesa runs it (Anti-Lag, glthread).
    pub fn mesa(&self) -> bool {
        !self.nvidia_driver()
    }

    /// Whether NVIDIA lists Smooth Motion for it: GeForce RTX 40 and newer.
    /// `None`: the name does not say.
    pub fn smooth_motion_capable(&self) -> Option<bool> {
        let series = rtx_series(&self.name)?;
        Some(series >= 40)
    }
}

/// `40` for "GeForce RTX 4070", `50` for "RTX 5090 Laptop GPU".
fn rtx_series(name: &str) -> Option<u32> {
    let rest = &name[name.find("RTX ")? + 4..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() == 4 {
        digits[..2].parse().ok()
    } else {
        None
    }
}

/// The cards and the layers of this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Machine {
    /// Primary card first, then by PCI address.
    pub cards: Vec<Card>,
    /// The NVIDIA driver's present layer, which has Smooth Motion (575+).
    pub smooth_motion: bool,
    /// Mesa's Anti-Lag layer.
    pub anti_lag: bool,
    /// Version of the loaded `nvidia` kernel module, like `580.82.07`.
    pub nvidia_driver: Option<String>,
}

impl Machine {
    pub fn detect(probe: &dyn Probe) -> Self {
        let vulkan = vulkan_devices();
        let ids = std::fs::read_to_string(PCI_IDS.iter().find(|p| Path::new(p).is_file()).copied().unwrap_or("")).unwrap_or_default();
        let mut cards: Vec<Card> = drm_slots()
            .into_iter()
            .filter_map(|slot| {
                let c = card_at(&slot, probe)?;
                let vk = vulkan.iter().find(|d| d.vendor_id == c.vendor_id && d.device_id == c.device_id);
                let driver = std::fs::read_link(format!("/sys/bus/pci/devices/{slot}/driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                Some(Card {
                    name: display_name(
                        c.vendor_id,
                        c.device_id,
                        pci_name(&ids, c.vendor_id, c.device_id).as_deref(),
                        vk.map(|d| d.name.as_str()),
                    ),
                    integrated: vk.and_then(|d| d.integrated),
                    slot,
                    vendor_id: c.vendor_id,
                    device_id: c.device_id,
                    driver,
                    primary: c.primary,
                })
            })
            .collect();
        cards.sort_by(|a, b| b.primary.cmp(&a.primary).then_with(|| a.slot.cmp(&b.slot)));
        let dirs = crate::probe::implicit_layer_dirs(probe);
        let manifest_has = |file: &str, text: &str| {
            dirs.iter()
                .any(|d| probe.read_head(&format!("{d}/{file}"), 1 << 16).is_some_and(|b| String::from_utf8_lossy(&b).contains(text)))
        };
        Self {
            smooth_motion: manifest_has("nvidia_layers.json", "NVPRESENT_ENABLE_SMOOTH_MOTION"),
            anti_lag: dirs.iter().any(|d| probe.exists(&format!("{d}/VkLayer_MESA_anti_lag.json"))),
            nvidia_driver: probe
                .read_head("/sys/module/nvidia/version", 64)
                .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                .filter(|v| !v.is_empty()),
            cards,
        }
    }

    pub fn card(&self, slot: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.slot == slot)
    }

    pub fn has_nvidia_driver(&self) -> bool {
        self.cards.iter().any(Card::nvidia_driver)
    }

    pub fn has_mesa(&self) -> bool {
        self.cards.iter().any(Card::mesa)
    }

    pub fn has_vendor(&self, v: Vendor) -> bool {
        self.cards.iter().any(|c| c.vendor() == Some(v))
    }
}

/// The ids of the card at a PCI address, read at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardIds {
    pub vendor_id: u16,
    pub device_id: u16,
    pub primary: bool,
}

/// The card at `slot` (`None`: no display card there).
pub fn card_at(slot: &str, probe: &dyn Probe) -> Option<CardIds> {
    let read = |file: &str| {
        probe.read_head(&format!("/sys/bus/pci/devices/{slot}/{file}"), 32).map(|b| String::from_utf8_lossy(&b).trim().to_string())
    };
    let hex = |file: &str| read(file).and_then(|t| u16::from_str_radix(t.trim_start_matches("0x"), 16).ok());
    Some(CardIds { vendor_id: hex("vendor")?, device_id: hex("device")?, primary: read("boot_vga").as_deref() == Some("1") })
}

/// The card the screens start on (`boot_vga`), or the only display card.
pub fn primary_card(probe: &dyn Probe) -> Option<CardIds> {
    let root = "/sys/bus/pci/devices";
    let display: Vec<CardIds> = probe
        .list_dir(root)
        .into_iter()
        .filter(|slot| {
            // PCI class 0x03xxxx: display controllers.
            probe.read_head(&format!("{root}/{slot}/class"), 16).is_some_and(|b| String::from_utf8_lossy(&b).trim().starts_with("0x03"))
        })
        .filter_map(|slot| card_at(&slot, probe))
        .collect();
    display.iter().find(|c| c.primary).or(if display.len() == 1 { display.first() } else { None }).copied()
}

/// PCI addresses of the cards with a DRM node (`/sys/class/drm/card<N>`).
fn drm_slots() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out: Vec<String> = dir
        .flatten()
        .filter(|e| {
            e.file_name().to_string_lossy().strip_prefix("card").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .filter_map(|e| std::fs::canonicalize(e.path().join("device")).ok())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|slot| slot.matches(':').count() == 2)
        .collect();
    out.sort();
    out.dedup();
    out
}

const PCI_IDS: [&str; 3] = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"];

/// A device's name in `pci.ids` (`GB205 [GeForce RTX 5070]`).
pub fn pci_name(ids: &str, vendor: u16, device: u16) -> Option<String> {
    let vendor_line = format!("{vendor:04x}  ");
    let device_line = format!("\t{device:04x}  ");
    let mut in_vendor = false;
    for line in ids.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if in_vendor {
                return None;
            }
            in_vendor = line.starts_with(&vendor_line);
        } else if in_vendor {
            if let Some(name) = line.strip_prefix(&device_line) {
                return Some(name.trim().to_string());
            }
        }
    }
    None
}

/// The name shown for a card: Vulkan's (without the driver's tag), else the
/// marketing name of `pci.ids`, with the maker in front.
pub fn display_name(vendor: u16, device: u16, pci: Option<&str>, vulkan: Option<&str>) -> String {
    let brand = match vendor {
        NVIDIA => "NVIDIA",
        AMD => "AMD",
        INTEL => "Intel",
        _ => "",
    };
    let with_brand = |name: &str| {
        if brand.is_empty() || name.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase()) {
            name.to_string()
        } else {
            format!("{brand} {name}")
        }
    };
    if let Some(name) = vulkan.map(clean_vulkan_name).filter(|n| !n.is_empty()) {
        return with_brand(&name);
    }
    if let Some(pci) = pci {
        let marketing = pci.find('[').and_then(|a| pci[a + 1..].find(']').map(|b| &pci[a + 1..a + 1 + b])).unwrap_or(pci);
        return with_brand(marketing.trim());
    }
    with_brand(&format!("{device:04x}"))
}

/// `Intel(R) Arc(tm) A770 Graphics (DG2)` -> `Intel Arc A770 Graphics`;
/// `AMD Radeon RX 7900 XTX (RADV NAVI31)` -> `AMD Radeon RX 7900 XTX`.
fn clean_vulkan_name(name: &str) -> String {
    let mut n = name.replace("(R)", "").replace("(r)", "").replace("(TM)", "").replace("(tm)", "");
    if n.trim_end().ends_with(')') {
        if let Some(i) = n.rfind(" (") {
            n.truncate(i);
        }
    }
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A device of `vulkaninfo --summary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VulkanDevice {
    pub vendor_id: u16,
    pub device_id: u16,
    pub name: String,
    pub integrated: Option<bool>,
}

/// The devices `vulkaninfo --summary` lists (nothing when it is missing
/// or does not answer within a few seconds).
fn vulkan_devices() -> Vec<VulkanDevice> {
    let mut cmd = Command::new("vulkaninfo");
    cmd.arg("--summary").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    // A console tool: without this Windows flashes a console window.
    heronite_platform::process::new_helper_group(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        return Vec::new();
    };
    // Read while waiting: a full pipe would stall it.
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout.as_mut() {
            std::io::Read::read_to_string(out, &mut text).ok();
        }
        text
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                child.kill().ok();
                child.wait().ok();
                break;
            }
        }
    }
    reader.join().map(|t| parse_vulkan_summary(&t)).unwrap_or_default()
}

/// The `GPU<n>:` blocks of `vulkaninfo --summary`.
pub fn parse_vulkan_summary(text: &str) -> Vec<VulkanDevice> {
    let mut out = Vec::new();
    let mut cur: Option<(Option<u16>, Option<u16>, String, Option<bool>)> = None;
    let mut push = |cur: &mut Option<(Option<u16>, Option<u16>, String, Option<bool>)>| {
        if let Some((Some(vendor_id), Some(device_id), name, integrated)) = cur.take() {
            out.push(VulkanDevice { vendor_id, device_id, name, integrated });
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("GPU") && t.ends_with(':') {
            push(&mut cur);
            cur = Some((None, None, String::new(), None));
            continue;
        }
        let (Some(c), Some((k, v))) = (cur.as_mut(), t.split_once('=')) else { continue };
        let v = v.trim();
        let hex = || u16::from_str_radix(v.trim_start_matches("0x"), 16).ok();
        match k.trim() {
            "vendorID" => c.0 = hex(),
            "deviceID" => c.1 = hex(),
            "deviceName" => c.2 = v.to_string(),
            "deviceType" => {
                c.3 = match v {
                    "PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU" => Some(true),
                    "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU" => Some(false),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    push(&mut cur);
    out
}

/// What a Proton build offers the GPU settings, read from its `proton`
/// script.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtonFeatures {
    /// `PROTON_DLSS_UPGRADE`, `PROTON_FSR4_UPGRADE`, `PROTON_XESS_UPGRADE`
    /// (GE-Proton, Proton-CachyOS).
    pub upgrades: bool,
    /// dxvk-nvapi's Vulkan Reflex layer (`DXVK_NVAPI_VKREFLEX`).
    pub reflex: bool,
}

/// `bin`: the Proton's `proton` script (a Wine install's `bin`). Reads up to
/// 1 MiB of it.
pub fn proton_features(bin: &str, probe: &dyn Probe) -> ProtonFeatures {
    let Some(bytes) = probe.read_head(bin, 1 << 20) else { return ProtonFeatures::default() };
    let text = String::from_utf8_lossy(&bytes);
    ProtonFeatures { upgrades: text.contains("PROTON_DLSS_UPGRADE"), reflex: text.contains("DXVK_NVAPI_VKREFLEX") }
}
