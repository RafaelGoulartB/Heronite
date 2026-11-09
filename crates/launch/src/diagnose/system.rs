//! The facts about the computer the analysis needs, and the commands that
//! install what is missing on each family of distributions.

use crate::gpu::Vendor;
use crate::probe::Probe;

/// What Valve's SteamOS, Fedora, Ubuntu (24.04+) and Arch set: many games
/// (with Easy Anti-Cheat or many memory maps) crash below it.
pub const MAX_MAP_COUNT: u64 = 1_048_576;

/// Keeps [`MAX_MAP_COUNT`] after a restart.
pub const SYSCTL_FILE: &str = "/etc/sysctl.d/80-heronite-max-map-count.conf";

/// The family of the distribution, by the package manager it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Distro {
    /// pacman (Arch, CachyOS, Manjaro, EndeavourOS...).
    Arch,
    /// apt (Debian, Ubuntu, Mint, Pop!_OS...).
    Debian,
    /// dnf (Fedora, Nobara...).
    Fedora,
    /// zypper (openSUSE).
    Suse,
    /// Anything else, and image-based systems (Bazzite, SteamOS, Silverblue)
    /// where packages are not installed this way.
    #[default]
    Other,
}

impl Distro {
    /// From `/etc/os-release` (`ID` and `ID_LIKE`); `image_based`: the
    /// system boots from an image (`/run/ostree-booted`).
    pub fn parse(os_release: &str, image_based: bool) -> Self {
        if image_based {
            return Distro::Other;
        }
        let value = |key: &str| {
            os_release
                .lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                .map(|v| v.trim().trim_matches('"').to_ascii_lowercase())
                .unwrap_or_default()
        };
        let id = value("ID");
        if id == "steamos" {
            return Distro::Other;
        }
        let like = value("ID_LIKE");
        let ids: Vec<&str> = std::iter::once(id.as_str()).chain(like.split_whitespace()).collect();
        let is = |names: &[&str]| ids.iter().any(|i| names.contains(i));
        if is(&["arch", "archlinux", "manjaro", "endeavouros", "cachyos", "garuda"]) {
            Distro::Arch
        } else if is(&["debian", "ubuntu"]) {
            Distro::Debian
        } else if is(&["fedora", "rhel", "centos"]) {
            Distro::Fedora
        } else if ids.iter().any(|i| i.contains("suse")) {
            Distro::Suse
        } else {
            Distro::Other
        }
    }

    /// A command that names the packages holding `libs` (installing them
    /// needs the package's name). With no library named and `bits32`, the
    /// 32-bit half of Wine where the distribution splits it.
    pub fn find_packages(self, libs: &[String], bits32: bool) -> Option<String> {
        let first = libs.first();
        match (self, first) {
            (Distro::Debian, None) if bits32 => {
                Some("sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install wine32".into())
            }
            (_, None) => None,
            (Distro::Arch, _) => Some(format!("sudo pacman -Fy && pacman -F {}", libs.join(" "))),
            (Distro::Debian, Some(lib)) => Some(format!("sudo apt install apt-file && sudo apt-file update && apt-file search {lib}")),
            (Distro::Fedora, _) => Some(format!("dnf provides {}", libs.iter().map(|l| format!("'*/{l}'")).collect::<Vec<_>>().join(" "))),
            (Distro::Suse, Some(lib)) => Some(format!("zypper search --provides {lib}")),
            (Distro::Other, _) => None,
        }
    }

    /// The command that installs Vulkan for 64- and 32-bit programs with
    /// the driver of `gpu` (`nvidia`: NVIDIA's own driver runs it).
    pub fn vulkan_packages(self, gpu: Option<Vendor>, nvidia: bool) -> Option<String> {
        let driver = match (gpu, nvidia) {
            (Some(Vendor::Nvidia), true) => "nvidia",
            (Some(Vendor::Nvidia), false) => "nouveau",
            (Some(Vendor::Intel), _) => "intel",
            (Some(Vendor::Amd), _) => "radeon",
            (None, _) => "",
        };
        Some(match self {
            Distro::Arch => {
                let pkgs = match driver {
                    "nvidia" => "nvidia-utils lib32-nvidia-utils",
                    "nouveau" => "vulkan-nouveau lib32-vulkan-nouveau",
                    "intel" => "vulkan-intel lib32-vulkan-intel",
                    "radeon" => "vulkan-radeon lib32-vulkan-radeon",
                    _ => "",
                };
                format!("sudo pacman -S --needed vulkan-icd-loader lib32-vulkan-icd-loader {pkgs}").trim_end().to_string()
            }
            Distro::Debian => {
                let mesa = if driver == "nvidia" { "" } else { " mesa-vulkan-drivers mesa-vulkan-drivers:i386" };
                format!("sudo dpkg --add-architecture i386 && sudo apt update && sudo apt install libvulkan1 libvulkan1:i386{mesa}")
            }
            Distro::Fedora => {
                let extra =
                    if driver == "nvidia" { "xorg-x11-drv-nvidia-libs.i686" } else { "mesa-vulkan-drivers mesa-vulkan-drivers.i686" };
                format!("sudo dnf install vulkan-loader vulkan-loader.i686 {extra}")
            }
            Distro::Suse => {
                let extra = match driver {
                    "intel" => " libvulkan_intel libvulkan_intel-32bit",
                    "radeon" => " libvulkan_radeon libvulkan_radeon-32bit",
                    _ => "",
                };
                format!("sudo zypper install libvulkan1 libvulkan1-32bit{extra}")
            }
            Distro::Other => return None,
        })
    }
}

/// What [`super::diagnose`] knows of the computer.
#[derive(Debug, Clone, Default)]
pub struct System {
    /// `/proc/sys/vm/max_map_count` (`None`: unreadable).
    pub max_map_count: Option<u64>,
    pub distro: Distro,
    /// The vendor of the card the screens start on.
    pub gpu: Option<Vendor>,
    /// NVIDIA's own driver is loaded.
    pub nvidia_driver: bool,
    /// Folders of the 64-bit libraries (the 32-bit ones live elsewhere).
    pub lib64_dirs: Vec<String>,
}

impl System {
    /// Reads the facts (a few small files).
    pub fn read(probe: &dyn Probe) -> Self {
        let text = |p: &str| probe.read_head(p, 1 << 14).map(|b| String::from_utf8_lossy(&b).into_owned());
        let os_release = text("/etc/os-release").or_else(|| text("/usr/lib/os-release")).unwrap_or_default();
        let lib64_dirs = ["/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/lib64"]
            .into_iter()
            .chain((!probe.exists("/usr/lib64")).then_some("/usr/lib"))
            .filter(|d| probe.exists(d))
            .map(String::from)
            .collect();
        System {
            max_map_count: text("/proc/sys/vm/max_map_count").and_then(|t| t.trim().parse().ok()),
            distro: Distro::parse(&os_release, probe.exists("/run/ostree-booted")),
            gpu: crate::gpu::detect::primary_card(probe).and_then(|c| Vendor::of_pci(c.vendor_id)),
            nvidia_driver: probe.exists("/sys/module/nvidia"),
            lib64_dirs,
        }
    }

    /// Whether the 64-bit copy of `lib` is installed (so a 32-bit program
    /// that cannot find it misses the 32-bit one).
    pub fn has_64bit_lib(&self, lib: &str) -> bool {
        !lib.contains('/') && self.lib64_dirs.iter().any(|d| std::path::Path::new(d).join(lib).exists())
    }
}

/// The shell script (run as root through `pkexec /bin/sh -c`) that raises
/// `vm.max_map_count` now and keeps it in [`SYSCTL_FILE`].
pub fn max_map_count_script() -> String {
    format!("printf 'vm.max_map_count = {MAX_MAP_COUNT}\\n' > {SYSCTL_FILE} && sysctl -q -w vm.max_map_count={MAX_MAP_COUNT}")
}
