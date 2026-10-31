//! The filesystems things live on, from `/proc/self/mountinfo` and
//! `statvfs`. Mounts of one device (btrfs subvolumes such as `/` and
//! `/home`, bind mounts) are one disk: they share its space.
//!
//! On Windows, from its volumes (`heronite_platform::disks::volumes`): a
//! drive letter, or a folder another volume is mounted on (`C:\Games\SSD`),
//! is a mount; a volume mounted at both is one disk, named by its letter.

use std::path::{Path, PathBuf};

/// A mounted filesystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub point: PathBuf,
    pub fs: String,
    /// `/dev/nvme0n1p2`, `/dev/mapper/root`... (Windows: `\\?\Volume{…}\`).
    pub source: String,
    /// The name it was given (Windows: `Windows`, `Games`); empty on Linux.
    pub label: String,
}

/// A disk (one device) and its space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// The shortest mount point of the device (`/` before `/home`).
    pub mount: PathBuf,
    pub fs: String,
    pub source: String,
    /// See [`Mount::label`].
    pub label: String,
    pub total: u64,
    /// Free for an ordinary user (`f_bavail`).
    pub free: u64,
}

impl Disk {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// What it is called: its mount point (`/`, `/mnt/games`); on Windows
    /// as File Explorer names a drive, by its label: `Games (D:)`. The
    /// system drive without one is `Windows (C:)`, another one just `E:`.
    pub fn name(&self) -> String {
        let point = self.mount.display().to_string();
        if !cfg!(windows) {
            return point;
        }
        // `C:\` is `C:`; a mounted folder keeps its path.
        let point = if point.len() == 3 && point.ends_with(":\\") { point[..2].to_string() } else { point };
        let system = std::env::var("SystemDrive").is_ok_and(|s| s.eq_ignore_ascii_case(&point));
        match self.label.as_str() {
            "" if system => format!("Windows ({point})"),
            "" => point,
            label => format!("{label} ({point})"),
        }
    }
}

/// Filesystems that never hold games.
#[cfg_attr(windows, allow(dead_code))]
const PSEUDO: [&str; 22] = [
    "proc",
    "sysfs",
    "tmpfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "mqueue",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "efivarfs",
    "bpf",
    "configfs",
    "fusectl",
    "hugetlbfs",
    "autofs",
    "binfmt_misc",
    "ramfs",
    "nsfs",
    "overlay",
];

/// The real filesystems mounted now.
pub fn mounts() -> Vec<Mount> {
    #[cfg(windows)]
    {
        heronite_platform::disks::volumes()
            .into_iter()
            .flat_map(|v| {
                let mounts = v.mount_points.iter();
                mounts
                    .map(|point| Mount { point: point.clone(), fs: v.fs.clone(), source: v.id.clone(), label: v.label.clone() })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default()
    }
}

/// `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
#[cfg_attr(windows, allow(dead_code))]
fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let point = left.split(' ').nth(4)?;
            let mut right = right.split(' ');
            let fs = right.next()?;
            let source = right.next()?;
            let pseudo = PSEUDO.contains(&fs) || fs.starts_with("fuse.") || fs == "squashfs";
            (!pseudo).then(|| Mount {
                point: PathBuf::from(unescape(point)),
                fs: fs.to_string(),
                source: unescape(source),
                label: String::new(),
            })
        })
        .collect()
}

/// mountinfo writes spaces, tabs, newlines and backslashes as `\040`-style octal.
#[cfg_attr(windows, allow(dead_code))]
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1..i + 4).is_some_and(|d| d.iter().all(|c| (b'0'..=b'7').contains(c))) {
            let v = (b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0');
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The mount `path` is on: the longest mount point it starts with, looking
/// at the path with its links resolved (or its nearest existing parent).
fn mount_of<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    let real = path.ancestors().find_map(|p| heronite_platform::disks::canonical(p).ok())?;
    mounts.iter().filter(|m| real.starts_with(&m.point)).max_by_key(|m| m.point.as_os_str().len())
}

/// Index of `path`'s disk in `disks`, adding the disk the first time.
pub fn locate(mounts: &[Mount], path: &Path, disks: &mut Vec<Disk>) -> Option<usize> {
    let mount = mount_of(mounts, path)?;
    if let Some(i) = disks.iter().position(|d| d.source == mount.source && d.fs == mount.fs) {
        return Some(i);
    }
    // The device's shortest mount names it (a drive letter before a folder).
    let point = mounts
        .iter()
        .filter(|m| m.source == mount.source && m.fs == mount.fs)
        .map(|m| m.point.clone())
        .min_by_key(|p| p.as_os_str().len())
        .unwrap_or_else(|| mount.point.clone());
    let (total, free) = space(&mount.point)?;
    disks.push(Disk { mount: point, fs: mount.fs.clone(), source: mount.source.clone(), label: mount.label.clone(), total, free });
    Some(disks.len() - 1)
}

/// Total and free bytes of the filesystem at `path`.
fn space(path: &Path) -> Option<(u64, u64)> {
    heronite_platform::fs::disk_space(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "\
32 2 0:29 /@ / rw,noatime shared:1 - btrfs /dev/mapper/root rw,subvol=/@
58 32 0:29 /@home /home rw,noatime shared:195 - btrfs /dev/mapper/root rw,subvol=/@home
24 32 0:22 / /proc rw - proc proc rw
249 32 259:8 / /mnt/Game\\040Drive rw,noatime shared:213 - ext4 /dev/nvme1n1p1 rw
402 383 0:83 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
";

    #[test]
    fn reads_real_mounts_and_unescapes() {
        let m = parse_mountinfo(INFO);
        let points: Vec<&str> = m.iter().map(|m| m.point.to_str().unwrap()).collect();
        assert_eq!(points, ["/", "/home", "/mnt/Game Drive"]);
        assert_eq!(m[2].source, "/dev/nvme1n1p1");
        assert_eq!(unescape("a\\134b"), "a\\b");
    }

    fn mount(point: PathBuf, source: &str) -> Mount {
        Mount { point, fs: "btrfs".into(), source: source.into(), label: String::new() }
    }

    /// Also a Windows volume mounted at a drive letter and at a folder.
    #[test]
    fn a_device_mounted_twice_is_one_disk() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let root = heronite_platform::disks::canonical(dir.path()).unwrap();
        let top = root.ancestors().last().unwrap().to_path_buf();
        let mounts = vec![mount(top.clone(), "x"), mount(root.join("a"), "y"), mount(root.join("b"), "y")];
        let mut disks = Vec::new();
        let ia = locate(&mounts, &a.join("not/made/yet"), &mut disks).unwrap();
        let ib = locate(&mounts, &b, &mut disks).unwrap();
        assert_eq!(ia, ib);
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].mount, root.join("a"));
        let other = locate(&mounts, &top, &mut disks).unwrap();
        assert_ne!(other, ia);
    }

    /// A folder linked elsewhere is on the disk it links to.
    #[cfg(windows)]
    #[test]
    fn a_junction_is_on_the_disk_it_points_to() {
        let dir = tempfile::tempdir().unwrap();
        let (games, other) = (dir.path().join("games"), dir.path().join("other"));
        std::fs::create_dir_all(&games).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let link = dir.path().join("link");
        let out = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&link).arg(&other).output().unwrap();
        assert!(out.status.success());
        let root = heronite_platform::disks::canonical(dir.path()).unwrap();
        let mounts = vec![mount(root.ancestors().last().unwrap().to_path_buf(), "c"), mount(root.join("other"), "d")];
        let mut disks = Vec::new();
        let i = locate(&mounts, &link.join("Game"), &mut disks).unwrap();
        assert_eq!(disks[i].source, "d");
        let j = locate(&mounts, &games, &mut disks).unwrap();
        assert_eq!(disks[j].source, "c");
    }

    /// The real drives: the temp folder is on one, with its space, named
    /// like File Explorer names it.
    #[cfg(windows)]
    #[test]
    fn windows_drives_have_space_and_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut disks = Vec::new();
        let i = locate(&mounts(), dir.path(), &mut disks).unwrap();
        let d = &disks[i];
        assert!(d.total > 0 && d.free <= d.total, "{d:?}");
        assert!(d.name().contains(':'), "{d:?}");
        let named = |label: &str, mount: &str| Disk {
            mount: mount.into(),
            label: label.into(),
            fs: "NTFS".into(),
            source: String::new(),
            total: 1,
            free: 0,
        };
        assert_eq!(named("Windows", r"C:\").name(), "Windows (C:)");
        assert_eq!(named("Games", r"D:\").name(), "Games (D:)");
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        assert_eq!(named("", &format!("{system}\\")).name(), format!("Windows ({system})"));
        let other = if system.eq_ignore_ascii_case("Z:") { r"Y:\" } else { r"Z:\" };
        assert_eq!(named("", other).name(), other[..2]);
        assert_eq!(named("SSD", r"C:\Games\SSD\").name(), r"SSD (C:\Games\SSD\)");
    }
}
