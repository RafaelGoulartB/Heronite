//! The filesystems things live on, from `/proc/self/mountinfo` and
//! `statvfs`. Mounts of one device (btrfs subvolumes such as `/` and
//! `/home`, bind mounts) are one disk: they share its space.

use std::fs;
use std::path::{Path, PathBuf};

/// A mounted filesystem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub point: PathBuf,
    pub fs: String,
    /// `/dev/nvme0n1p2`, `/dev/mapper/root`...
    pub source: String,
}

/// A disk (one device) and its space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// The shortest mount point of the device (`/` before `/home`).
    pub mount: PathBuf,
    pub fs: String,
    pub source: String,
    pub total: u64,
    /// Free for an ordinary user (`f_bavail`).
    pub free: u64,
}

impl Disk {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

/// Filesystems that never hold games.
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
    fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default()
}

/// `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let point = left.split(' ').nth(4)?;
            let mut right = right.split(' ');
            let fs = right.next()?;
            let source = right.next()?;
            let pseudo = PSEUDO.contains(&fs) || fs.starts_with("fuse.") || fs == "squashfs";
            (!pseudo).then(|| Mount { point: PathBuf::from(unescape(point)), fs: fs.to_string(), source: unescape(source) })
        })
        .collect()
}

/// mountinfo writes spaces, tabs, newlines and backslashes as `\040`-style octal.
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
    let real = path.ancestors().find_map(|p| fs::canonicalize(p).ok())?;
    mounts.iter().filter(|m| real.starts_with(&m.point)).max_by_key(|m| m.point.as_os_str().len())
}

/// Index of `path`'s disk in `disks`, adding the disk the first time.
pub fn locate(mounts: &[Mount], path: &Path, disks: &mut Vec<Disk>) -> Option<usize> {
    let mount = mount_of(mounts, path)?;
    if let Some(i) = disks.iter().position(|d| d.source == mount.source && d.fs == mount.fs) {
        return Some(i);
    }
    // The device's shortest mount names it.
    let point = mounts
        .iter()
        .filter(|m| m.source == mount.source && m.fs == mount.fs)
        .map(|m| m.point.clone())
        .min_by_key(|p| p.as_os_str().len())
        .unwrap_or_else(|| mount.point.clone());
    let (total, free) = space(&mount.point)?;
    disks.push(Disk { mount: point, fs: mount.fs.clone(), source: mount.source.clone(), total, free });
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

    // Linux disks and folders; Storage is off on Windows (`Feature::Storage`).
    #[cfg(unix)]
    #[test]
    fn a_device_mounted_twice_is_one_disk() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let mounts = vec![
            Mount { point: PathBuf::from("/"), fs: "ext4".into(), source: "/dev/x".into() },
            Mount { point: root.join("a"), fs: "btrfs".into(), source: "/dev/y".into() },
            Mount { point: root.join("b"), fs: "btrfs".into(), source: "/dev/y".into() },
        ];
        let mut disks = Vec::new();
        let ia = locate(&mounts, &a.join("not/made/yet"), &mut disks).unwrap();
        let ib = locate(&mounts, &b, &mut disks).unwrap();
        assert_eq!(ia, ib);
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].mount, root.join("a"));
        let other = locate(&mounts, Path::new("/usr"), &mut disks).unwrap();
        assert_ne!(other, ia);
    }
}
