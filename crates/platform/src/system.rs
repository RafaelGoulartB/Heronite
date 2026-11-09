//! What the computer is, for the logs (Windows).
//!
//! The processor, the memory, the graphics cards and the Windows version, for
//! Settings → About and the header of the logs. They come from the registry
//! and `GlobalMemoryStatusEx`; nothing is spawned. A value Windows does not
//! give is `None` (or an empty list) and the caller shows what it has.

use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use crate::registry::{self, Hive};

const VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const PROCESSOR_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
/// The display adapters' class: one numbered key (`0000`, `0001`...) per adapter, next to
/// others (`Properties`, `Configuration`) that describe no adapter.
const DISPLAY_CLASS_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

/// The first build of Windows 11. Windows 11 still says "Windows 10" in `ProductName`.
const WINDOWS_11_BUILD: u32 = 22000;

/// The processor's name as the maker words it ("AMD Ryzen 5 7600 6-Core Processor"), the
/// padding Windows leaves in it removed.
pub fn processor_name() -> Option<String> {
    let name = registry::string(Hive::LocalMachine, PROCESSOR_KEY, "ProcessorNameString")?;
    Some(name.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|n| !n.is_empty())
}

/// The physical memory Windows can use, in bytes. A little less than what is installed:
/// the firmware and the integrated graphics keep a part.
pub fn total_memory_bytes() -> Option<u64> {
    // SAFETY: all zeros is a valid `MEMORYSTATUSEX`.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is writable and its length is set, as the call requires. 0 is a failure.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) } != 0;
    ok.then_some(status.ullTotalPhys)
}

/// A display adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Card {
    /// The maker, short: "NVIDIA", "AMD", "Intel". Empty when the driver names none.
    pub vendor: String,
    /// The model without the maker: "GeForce RTX 5070", "Radeon Graphics", "UHD Graphics 770".
    pub name: String,
    /// The driver's version. NVIDIA's own number ("616.92") instead of the one Windows keeps
    /// ("32.0.16.1692"); the other makers' as Windows has it. Empty when unknown.
    pub driver_version: String,
}

/// The graphics cards, integrated ones included, in the order Windows lists them.
///
/// Software and virtual adapters ("Microsoft Basic Display Adapter", remote desktop's) are
/// left out, as are cards listed twice. A card that was removed without uninstalling its
/// driver may still be listed: the registry does not say whether the card is plugged in.
pub fn graphics_cards() -> Vec<Card> {
    let mut cards: Vec<Card> = Vec::new();
    for index in registry::subkeys(Hive::LocalMachine, DISPLAY_CLASS_KEY) {
        let key = format!(r"{DISPLAY_CLASS_KEY}\{index}");
        let text = |name| registry::string(Hive::LocalMachine, &key, name).map(|v| v.trim().to_string()).unwrap_or_default();
        let description = text("DriverDesc");
        if description.is_empty() || is_software_adapter(&description) {
            continue;
        }
        let card = card(&description, &text("ProviderName"), &text("DriverVersion"));
        if !cards.contains(&card) {
            cards.push(card);
        }
    }
    cards
}

/// A [`Card`] out of the registry's `DriverDesc`, `ProviderName` and `DriverVersion`.
fn card(description: &str, provider: &str, driver_version: &str) -> Card {
    let vendor = maker(provider);
    let driver_version = match vendor.as_str() {
        "NVIDIA" => nvidia_release(driver_version).unwrap_or_else(|| driver_version.to_string()),
        _ => driver_version.to_string(),
    };
    Card { name: model(description, &vendor), vendor, driver_version }
}

/// Adapters that draw nothing on a screen by themselves: Windows' fallback driver, remote
/// desktop's and the virtual displays some apps add.
fn is_software_adapter(description: &str) -> bool {
    const MARKERS: [&str; 6] = ["basic display", "basic render", "remote display", "virtual display", "indirect display", "mirror driver"];
    let description = description.to_lowercase();
    MARKERS.iter().any(|m| description.contains(m))
}

/// The maker's short name out of a driver's `ProviderName`
/// ("Advanced Micro Devices, Inc." → "AMD", "Intel Corporation" → "Intel").
fn maker(provider: &str) -> String {
    const SUFFIXES: [&str; 11] =
        ["inc.", "inc", "corporation", "corp.", "corp", "ltd.", "ltd", "limited", "co.", "technologies", "technology"];
    // "Qualcomm Technologies, Inc." is cut at the comma; "ATI Technologies Inc." loses its suffixes.
    let mut name = tidy(provider.split(',').next().unwrap_or_default());
    while let Some(shorter) = SUFFIXES.iter().find_map(|s| strip_suffix_ignore_case(&name, s)) {
        name = shorter.trim_end().to_string();
    }
    match name.to_lowercase().as_str() {
        "advanced micro devices" | "ati" | "amd" => "AMD".to_string(),
        _ => name,
    }
}

/// The model out of a `DriverDesc` ("Intel(R) UHD Graphics 770" → "UHD Graphics 770"): the
/// maker's name and the (R) / (TM) marks go, as in the names of Linux's PCI ids.
fn model(description: &str, vendor: &str) -> String {
    let description = tidy(description);
    // AMD's cards say "AMD" or the older "ATI" in front of "Radeon".
    let makers: &[&str] = if vendor == "AMD" { &["AMD", "ATI"] } else { &[vendor] };
    let rest = makers
        .iter()
        .filter(|m| !m.is_empty())
        .find_map(|m| strip_prefix_ignore_case(&description, m).filter(|r| r.is_empty() || r.starts_with(' ')))
        .map(str::trim)
        .filter(|r| !r.is_empty());
    rest.map_or(description.clone(), str::to_string)
}

/// NVIDIA's release number out of the version Windows keeps: its last digit of the third part
/// and the fourth part, as hundredths ("32.0.16.1692" → "616.92").
fn nvidia_release(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    let [_, _, third, fourth] = parts[..] else { return None };
    if fourth.len() != 4 || !third.chars().chain(fourth.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits = format!("{}{fourth}", third.chars().last()?);
    Some(format!("{}.{}", &digits[..3], &digits[3..]))
}

/// `text` without the (R) / (TM) marks and with single spaces.
fn tidy(text: &str) -> String {
    let mut text = text.to_string();
    for mark in ["(R)", "(r)", "(TM)", "(tm)", "®", "™"] {
        text = text.replace(mark, "");
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

/// `text` without `suffix` when it ends with it as a word of its own.
fn strip_suffix_ignore_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let start = text.len().checked_sub(suffix.len())?;
    let (head, tail) = (text.get(..start)?, text.get(start..)?);
    (tail.eq_ignore_ascii_case(suffix) && head.ends_with(' ')).then_some(head)
}

/// The version of Windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsVersion {
    /// "Windows 11 Pro". Windows 11 says "Windows 10" in the registry; this is corrected.
    pub name: String,
    /// The feature update: "24H2". `None` when Windows does not say.
    pub release: Option<String>,
    /// "26100".
    pub build: u32,
    /// The monthly update level (`UBR`): the 4061 of 10.0.26100.4061.
    pub revision: Option<u32>,
}

impl WindowsVersion {
    /// The edition and feature update: "Windows 11 Pro 24H2".
    pub fn label(&self) -> String {
        match &self.release {
            Some(release) => format!("{} {release}", self.name),
            None => self.name.clone(),
        }
    }

    /// "26100.4061": the build and, when known, its update level.
    pub fn build_label(&self) -> String {
        match self.revision {
            Some(revision) => format!("{}.{revision}", self.build),
            None => self.build.to_string(),
        }
    }
}

/// The edition, feature update and build of this Windows ("Windows 11 Pro", "24H2", 26100.4061).
pub fn windows_version() -> Option<WindowsVersion> {
    let text = |name| registry::string(Hive::LocalMachine, VERSION_KEY, name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let build: u32 = text("CurrentBuildNumber")?.parse().ok()?;
    Some(WindowsVersion {
        name: product_name(&text("ProductName")?, build),
        // `DisplayVersion` came with 20H2; before it `ReleaseId` said "2004".
        release: text("DisplayVersion").or_else(|| text("ReleaseId")),
        build,
        revision: registry::dword(Hive::LocalMachine, VERSION_KEY, "UBR"),
    })
}

/// Windows 11 keeps "Windows 10 Pro" in `ProductName`; the build number tells them apart.
fn product_name(registry_name: &str, build: u32) -> String {
    match registry_name.strip_prefix("Windows 10") {
        Some(edition) if build >= WINDOWS_11_BUILD => format!("Windows 11{edition}"),
        _ => registry_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makers_and_models_read_like_the_pci_names_of_linux() {
        let c = card("NVIDIA GeForce RTX 5070", "NVIDIA", "32.0.16.1692");
        assert_eq!((c.vendor.as_str(), c.name.as_str(), c.driver_version.as_str()), ("NVIDIA", "GeForce RTX 5070", "616.92"));
        let c = card("AMD Radeon(TM) Graphics", "Advanced Micro Devices, Inc.", "31.0.24002.92");
        assert_eq!((c.vendor.as_str(), c.name.as_str(), c.driver_version.as_str()), ("AMD", "Radeon Graphics", "31.0.24002.92"));
        let c = card("Intel(R) Arc(TM) A770 Graphics", "Intel Corporation", "32.0.101.6078");
        assert_eq!((c.vendor.as_str(), c.name.as_str(), c.driver_version.as_str()), ("Intel", "Arc A770 Graphics", "32.0.101.6078"));
        // Old Radeon drivers: ATI as the provider and the description.
        let c = card("ATI Radeon HD 5770", "ATI Technologies Inc.", "8.970.100.9001");
        assert_eq!((c.vendor.as_str(), c.name.as_str()), ("AMD", "Radeon HD 5770"));
        // A maker that is not in the description stays out of it.
        let c = card("Radeon RX 580 Series", "Advanced Micro Devices, Inc.", "");
        assert_eq!((c.vendor.as_str(), c.name.as_str()), ("AMD", "Radeon RX 580 Series"));
        // "AMDGPU" is no "AMD" followed by a model.
        assert_eq!(model("AMDGPU Thing", "AMD"), "AMDGPU Thing");
        // No provider: the description as it is.
        let c = card("Some Display Adapter", "", "1.0");
        assert_eq!((c.vendor.as_str(), c.name.as_str()), ("", "Some Display Adapter"));
        // The description is only the maker.
        assert_eq!(model("Intel", "Intel"), "Intel");
    }

    #[test]
    fn maker_names_lose_their_legal_suffixes() {
        assert_eq!(maker("Advanced Micro Devices, Inc."), "AMD");
        assert_eq!(maker("Intel Corporation"), "Intel");
        assert_eq!(maker("NVIDIA"), "NVIDIA");
        assert_eq!(maker("Qualcomm Technologies, Inc."), "Qualcomm");
        assert_eq!(maker("Microsoft Corporation"), "Microsoft");
        assert_eq!(maker("VMware, Inc."), "VMware");
        assert_eq!(maker("  "), "");
        // A name that merely ends like a suffix is kept.
        assert_eq!(maker("Zinc"), "Zinc");
    }

    #[test]
    fn nvidia_release_numbers() {
        assert_eq!(nvidia_release("32.0.16.1692").as_deref(), Some("616.92"));
        assert_eq!(nvidia_release("32.0.15.7270").as_deref(), Some("572.70"));
        assert_eq!(nvidia_release("31.0.15.5222").as_deref(), Some("552.22"));
        assert_eq!(nvidia_release("27.21.14.6109").as_deref(), Some("461.09"));
        assert_eq!(nvidia_release("10.0.26100"), None);
        assert_eq!(nvidia_release("32.0.15.72"), None);
        assert_eq!(nvidia_release(""), None);
        // Kept as Windows has it when it is not a number NVIDIA would use.
        assert_eq!(card("NVIDIA Thing", "NVIDIA", "garbage").driver_version, "garbage");
    }

    #[test]
    fn software_adapters_are_not_cards() {
        assert!(is_software_adapter("Microsoft Basic Display Adapter"));
        assert!(is_software_adapter("Microsoft Remote Display Adapter"));
        assert!(is_software_adapter("Parsec Virtual Display Adapter"));
        assert!(is_software_adapter("Microsoft Basic Render Driver"));
        assert!(!is_software_adapter("NVIDIA GeForce RTX 5070"));
        assert!(!is_software_adapter("AMD Radeon(TM) Graphics"));
    }

    #[test]
    fn windows_11_is_told_apart_by_its_build() {
        assert_eq!(product_name("Windows 10 Pro", 26100), "Windows 11 Pro");
        assert_eq!(product_name("Windows 10 Home", 22000), "Windows 11 Home");
        assert_eq!(product_name("Windows 10 Enterprise N", 22631), "Windows 11 Enterprise N");
        assert_eq!(product_name("Windows 10 Pro", 19045), "Windows 10 Pro");
        assert_eq!(product_name("Windows 11 Pro", 26100), "Windows 11 Pro");
        assert_eq!(product_name("Windows Server 2022 Standard", 20348), "Windows Server 2022 Standard");
    }

    #[test]
    fn version_labels() {
        let version = |release: Option<&str>, revision| WindowsVersion {
            name: "Windows 11 Pro".into(),
            release: release.map(str::to_string),
            build: 26100,
            revision,
        };
        assert_eq!(version(Some("24H2"), Some(4061)).label(), "Windows 11 Pro 24H2");
        assert_eq!(version(None, None).label(), "Windows 11 Pro");
        assert_eq!(version(None, Some(4061)).build_label(), "26100.4061");
        assert_eq!(version(None, None).build_label(), "26100");
    }

    #[test]
    fn prefixes_and_suffixes_ignore_case() {
        assert_eq!(strip_prefix_ignore_case("NVIDIA GeForce", "nvidia"), Some(" GeForce"));
        assert_eq!(strip_prefix_ignore_case("NV", "nvidia"), None);
        assert_eq!(strip_suffix_ignore_case("Foo INC.", "inc."), Some("Foo "));
        assert_eq!(strip_suffix_ignore_case("Zinc", "inc"), None);
        // Not cut in the middle of a character.
        assert_eq!(strip_prefix_ignore_case("é", "e"), None);
    }

    #[test]
    fn this_computer_has_a_name_memory_and_a_windows() {
        let cpu = processor_name().expect("the registry has the processor's name");
        assert!(cpu.len() > 3 && cpu == cpu.trim() && !cpu.contains("  "), "{cpu:?}");

        let memory = total_memory_bytes().expect("GlobalMemoryStatusEx works");
        assert!((1 << 30..1 << 50).contains(&memory), "{memory} bytes");

        let version = windows_version().expect("the registry has the version");
        assert!(version.name.starts_with("Windows "), "{version:?}");
        assert!(version.build >= 10240, "{version:?}");
        assert!(version.revision.is_some(), "{version:?}");
        // The name and the build agree on which Windows this is (a server,
        // such as CI's "Windows Server 2025", has a name of its own).
        if version.name.starts_with("Windows 10") || version.name.starts_with("Windows 11") {
            assert_eq!(version.name.starts_with("Windows 11"), version.build >= WINDOWS_11_BUILD, "{version:?}");
        }
        assert!(version.release.as_deref().is_none_or(|r| !r.is_empty()), "{version:?}");
    }

    #[test]
    fn this_computers_cards_have_names() {
        // A virtual machine may have none; a physical PC has one or more.
        let cards = graphics_cards();
        for card in &cards {
            assert!(!card.name.is_empty(), "{cards:?}");
            assert!(!is_software_adapter(&card.name), "{cards:?}");
            assert!(!card.name.contains("(R)") && !card.name.contains("(TM)"), "{cards:?}");
        }
        let mut unique = cards.clone();
        unique.dedup();
        assert_eq!(unique, cards);
        eprintln!("{cards:#?}");
    }
}
