//! The disks things live on and the space files take on them, for
//! Settings → Storage.
//!
//! Linux reads its mounts from `/proc` (`heronite_storage::disks`); Windows
//! has volumes ([`volumes`]): each mounted as a drive letter, as a folder of
//! another volume, or both. Sizes are what the disk gives away for a file
//! (whole clusters, less for a compressed or sparse file), links are never
//! followed: a symlink, a junction or a mounted folder points somewhere else.

use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};

use crate::fs::allocated_bytes;
#[cfg(unix)]
use crate::fs::hard_link_id;

/// An entry of a folder with the space it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizedEntry {
    pub path: PathBuf,
    /// A folder to walk into (never a link to one).
    pub dir: bool,
    /// Bytes on the disk (a link: its own few bytes, or none).
    pub bytes: u64,
    /// Set for a file with other names too (hard links, Unix only), to count
    /// it once.
    pub link_id: Option<(u64, u64)>,
}

/// The entries of `dir`, links not followed. Entries that cannot be read
/// are left out.
pub fn sized_entries(dir: &Path) -> io::Result<Vec<SizedEntry>> {
    #[cfg(unix)]
    {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)?.flatten() {
            // Not followed: a link takes its own few bytes.
            let Ok(meta) = entry.metadata() else { continue };
            out.push(SizedEntry { path: entry.path(), dir: meta.is_dir(), bytes: allocated_bytes(&meta), link_id: hard_link_id(&meta) });
        }
        Ok(out)
    }
    #[cfg(windows)]
    {
        win::sized_entries(dir)
    }
}

/// Bytes the file `path` takes on the disk (`meta`: its own metadata, the
/// link's for a link).
pub fn allocated_size(path: &Path, meta: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        let _ = path;
        allocated_bytes(meta)
    }
    #[cfg(windows)]
    {
        win::file_size_on_disk(path, meta).unwrap_or_else(|| allocated_bytes(meta))
    }
}

/// `path` with its links resolved, as other paths are written: without
/// Windows' `\\?\` prefix (`C:\Games`, `\\server\share`).
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let real = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    let real = win::without_verbatim(real);
    Ok(real)
}

/// Whether `path` is on a removable drive (a USB stick, a memory card),
/// which has no Recycle Bin. Always `false` on Unix.
pub fn on_removable_drive(path: &Path) -> bool {
    #[cfg(unix)]
    {
        let _ = path;
        false
    }
    #[cfg(windows)]
    {
        win::on_removable_drive(path)
    }
}

/// Whether Windows hides the file (the hidden or the system attribute:
/// `$RECYCLE.BIN`, `System Volume Information`). Unix hides by a leading
/// dot in the name instead: always `false` there.
pub fn hidden(meta: &Metadata) -> bool {
    #[cfg(unix)]
    {
        let _ = meta;
        false
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
        meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
    }
}

/// A volume of a Windows PC: a fixed or removable disk with a file system.
#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// `\\?\Volume{…}\`: the same wherever it is mounted.
    pub id: String,
    /// Where it is mounted: `D:\`, a folder such as `C:\Games\SSD\`, or both.
    pub mount_points: Vec<PathBuf>,
    /// The name it was given (`Windows`, `Games`); may be empty.
    pub label: String,
    /// `NTFS`, `exFAT`, `ReFS`...
    pub fs: String,
    /// A USB stick or a memory card (no Recycle Bin).
    pub removable: bool,
}

/// The fixed and removable volumes that are mounted somewhere and hold a
/// medium. Network drives (no volumes of this PC) and optical drives are
/// left out.
#[cfg(windows)]
pub fn volumes() -> Vec<Volume> {
    win::volumes()
}

#[cfg(windows)]
mod win {
    use std::ffi::OsString;
    use std::fs::Metadata;
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        FileFullDirectoryInfo, FileStandardInfo, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetCompressedFileSizeW, GetDriveTypeW,
        GetFileInformationByHandleEx, GetVolumeInformationW, GetVolumePathNameW, GetVolumePathNamesForVolumeNameW,
        FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FULL_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, INVALID_FILE_SIZE,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{SetThreadErrorMode, SEM_FAILCRITICALERRORS};

    use super::{SizedEntry, Volume};

    // What `GetDriveTypeW` returns (windows-sys keeps the names in a feature this crate does not use).
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    /// The reparse tags of links (symlinks, junctions, mounted folders):
    /// `IsReparseTagNameSurrogate`.
    const NAME_SURROGATE: u32 = 0x2000_0000;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    /// Up to the first NUL.
    fn from_wide(buf: &[u16]) -> OsString {
        OsString::from_wide(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())])
    }

    /// No "There is no disk in the drive" box while drives are asked.
    struct QuietErrors(u32);

    impl QuietErrors {
        fn new() -> Self {
            let mut old = 0;
            // SAFETY: `old` is ours to write.
            unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut old) };
            Self(old)
        }
    }

    impl Drop for QuietErrors {
        fn drop(&mut self) {
            // SAFETY: restores the mode read in `new`.
            unsafe { SetThreadErrorMode(self.0, std::ptr::null_mut()) };
        }
    }

    pub(super) fn volumes() -> Vec<Volume> {
        let _quiet = QuietErrors::new();
        let mut out = Vec::new();
        // `\\?\Volume{GUID}\` is 49 characters.
        let mut name = [0u16; 64];
        // SAFETY: `name` is writable for its length.
        let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
        if find == INVALID_HANDLE_VALUE {
            return out;
        }
        loop {
            out.extend(volume(&name));
            // SAFETY: `find` is open, `name` writable for its length.
            if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
                break;
            }
        }
        // SAFETY: `find` came from FindFirstVolumeW and is closed once.
        unsafe { FindVolumeClose(find) };
        out.sort_by(|a, b| a.mount_points.cmp(&b.mount_points));
        out
    }

    /// The volume `name` (NUL-terminated) names, if it is a disk of this PC
    /// that is mounted and holds a medium.
    fn volume(name: &[u16]) -> Option<Volume> {
        // SAFETY: `name` is NUL-terminated.
        let kind = unsafe { GetDriveTypeW(name.as_ptr()) };
        if !matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE) {
            return None;
        }
        let mount_points = mount_points(name);
        if mount_points.is_empty() {
            return None;
        }
        let (mut label, mut fs) = ([0u16; 261], [0u16; 261]);
        // SAFETY: `name` is NUL-terminated; the buffers are writable for the lengths given; the other outputs may be null.
        let ok = unsafe {
            GetVolumeInformationW(
                name.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        // No medium (an empty card reader).
        if ok == 0 {
            return None;
        }
        Some(Volume {
            id: from_wide(name).to_string_lossy().into_owned(),
            mount_points,
            label: from_wide(&label).to_string_lossy().trim().to_string(),
            fs: from_wide(&fs).to_string_lossy().into_owned(),
            removable: kind == DRIVE_REMOVABLE,
        })
    }

    /// `D:\`, `C:\Games\SSD\`...: where the volume `name` is mounted.
    fn mount_points(name: &[u16]) -> Vec<PathBuf> {
        let mut buf = vec![0u16; 512];
        loop {
            let mut needed = 0u32;
            // SAFETY: `name` is NUL-terminated; `buf` is writable for its length.
            let ok = unsafe { GetVolumePathNamesForVolumeNameW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32, &mut needed) };
            if ok != 0 {
                break;
            }
            // SAFETY: no arguments.
            if unsafe { GetLastError() } != ERROR_MORE_DATA || needed as usize <= buf.len() {
                return Vec::new();
            }
            buf.resize(needed as usize, 0);
        }
        // A list of NUL-terminated paths, ended by an empty one.
        buf.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(|s| PathBuf::from(OsString::from_wide(s))).collect()
    }

    pub(super) fn on_removable_drive(path: &Path) -> bool {
        let _quiet = QuietErrors::new();
        let Some(real) = path.ancestors().find_map(|p| super::canonical(p).ok()) else { return false };
        let mut root = [0u16; 1024];
        // SAFETY: the path is NUL-terminated; `root` is writable for its length.
        if unsafe { GetVolumePathNameW(wide(&real).as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return false;
        }
        // SAFETY: GetVolumePathNameW wrote a NUL-terminated root.
        unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOVABLE }
    }

    pub(super) fn without_verbatim(path: PathBuf) -> PathBuf {
        let Some(s) = path.to_str() else { return path };
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        match s.strip_prefix(r"\\?\") {
            Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
            _ => path,
        }
    }

    /// The space a compressed, sparse or otherwise special file takes
    /// (`GetCompressedFileSizeW`: what NTFS or WOF compression left of it).
    fn compressed_size(path: &Path) -> Option<u64> {
        let mut high = 0u32;
        // SAFETY: the path is NUL-terminated; `high` is ours to write.
        let low = unsafe { GetCompressedFileSizeW(wide(path).as_ptr(), &mut high) };
        // SAFETY: no arguments.
        if low == INVALID_FILE_SIZE && unsafe { GetLastError() } != 0 {
            return None;
        }
        Some((high as u64) << 32 | low as u64)
    }

    /// Whether the size of a file with these attributes is asked to the
    /// file itself: compressed, sparse, or behind a reparse point (WOF
    /// compression, deduplication, cloud files).
    fn special(attrs: u32) -> bool {
        attrs & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE | FILE_ATTRIBUTE_REPARSE_POINT) != 0
    }

    pub(super) fn file_size_on_disk(path: &Path, meta: &Metadata) -> Option<u64> {
        if meta.file_type().is_symlink() {
            return Some(0);
        }
        if special(meta.file_attributes()) {
            return compressed_size(path);
        }
        let file = std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .ok()?;
        // SAFETY: zeroed is a valid FILE_STANDARD_INFO.
        let mut info: FILE_STANDARD_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: the handle is open for the call; `info` is writable for its size.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                (&mut info as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        };
        (ok != 0).then(|| info.AllocationSize.max(0) as u64)
    }

    /// The folder read in batches (`GetFileInformationByHandleEx`): its
    /// entries come with their allocated size, no file is opened (only the
    /// compressed and special ones are asked their size).
    pub(super) fn sized_entries(dir: &Path) -> io::Result<Vec<SizedEntry>> {
        let handle = std::fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)?;
        // 64 KiB, aligned for the records.
        let mut buf = vec![0u64; 8 * 1024];
        let bytes = (buf.len() * 8) as u32;
        let mut out = Vec::new();
        loop {
            // SAFETY: the handle is open for the call; `buf` is writable for `bytes`.
            let ok = unsafe { GetFileInformationByHandleEx(handle.as_raw_handle(), FileFullDirectoryInfo, buf.as_mut_ptr().cast(), bytes) };
            if ok == 0 {
                let err = io::Error::last_os_error();
                if matches!(err.raw_os_error().map(|e| e as u32), Some(ERROR_NO_MORE_FILES | ERROR_FILE_NOT_FOUND)) {
                    return Ok(out);
                }
                return Err(err);
            }
            let base = buf.as_ptr().cast::<u8>();
            let name_at = std::mem::offset_of!(FILE_FULL_DIR_INFO, FileName);
            let mut offset = 0usize;
            loop {
                // SAFETY: the call filled the buffer with records chained by
                // `NextEntryOffset`, each inside it; read unaligned to be safe.
                let info: FILE_FULL_DIR_INFO = unsafe { std::ptr::read_unaligned(base.add(offset).cast()) };
                let len = info.FileNameLength as usize / 2;
                let mut name = vec![0u16; len];
                // SAFETY: the record's name follows its header, `FileNameLength` bytes long, inside the buffer.
                unsafe { std::ptr::copy_nonoverlapping(base.add(offset + name_at).cast::<u16>(), name.as_mut_ptr(), len) };
                if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                    out.push(entry(dir, OsString::from_wide(&name), &info));
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                offset += info.NextEntryOffset as usize;
            }
        }
    }

    fn entry(dir: &Path, name: OsString, info: &FILE_FULL_DIR_INFO) -> SizedEntry {
        let attrs = info.FileAttributes;
        let reparse = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        // With a reparse point, `EaSize` holds its tag.
        let link = reparse && info.EaSize & NAME_SURROGATE != 0;
        let dir_flag = attrs & FILE_ATTRIBUTE_DIRECTORY != 0 && !link;
        let path = dir.join(name);
        let allocated = info.AllocationSize.max(0) as u64;
        let bytes = if link {
            0
        } else if !dir_flag && special(attrs) {
            compressed_size(&path).unwrap_or(allocated)
        } else {
            allocated
        };
        SizedEntry { path, dir: dir_flag, bytes, link_id: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_have_their_size_and_links_are_not_folders() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("big.bin"), vec![1u8; 100_000]).unwrap();
        let mut entries = sized_entries(d.path()).unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(!entries[0].dir && entries[0].bytes >= 100_000, "{entries:?}");
        assert!(entries[1].dir);
        let meta = std::fs::symlink_metadata(d.path().join("big.bin")).unwrap();
        assert!(allocated_size(&d.path().join("big.bin"), &meta) >= 100_000);
        assert!(sized_entries(&d.path().join("missing")).is_err());
    }

    #[test]
    fn canonical_paths_are_plain() {
        let d = tempfile::tempdir().unwrap();
        let real = canonical(d.path()).unwrap();
        assert!(real.is_absolute());
        assert!(!real.to_string_lossy().starts_with(r"\\?\"), "{}", real.display());
        assert!(!on_removable_drive(d.path()));
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefixes_go() {
        assert_eq!(win::without_verbatim(PathBuf::from(r"\\?\C:\Games")), PathBuf::from(r"C:\Games"));
        assert_eq!(win::without_verbatim(PathBuf::from(r"\\?\UNC\nas\share\x")), PathBuf::from(r"\\nas\share\x"));
        let odd = PathBuf::from(r"\\?\Volume{1234}\x");
        assert_eq!(win::without_verbatim(odd.clone()), odd);
    }

    /// The temp folder is on a volume of this PC, mounted at its drive.
    #[cfg(windows)]
    #[test]
    fn the_temp_folder_is_on_a_volume() {
        let d = tempfile::tempdir().unwrap();
        let real = canonical(d.path()).unwrap();
        let vols = volumes();
        let vol = vols.iter().find(|v| v.mount_points.iter().any(|m| real.starts_with(m))).unwrap_or_else(|| panic!("{vols:?}"));
        assert!(vol.id.starts_with(r"\\?\Volume{"), "{vol:?}");
        assert!(!vol.fs.is_empty(), "{vol:?}");
        assert!(vols.iter().all(|v| !v.mount_points.is_empty()));
    }

    /// A junction (no administrator needed) is a link: its size is none and
    /// it is not walked into.
    #[cfg(windows)]
    #[test]
    fn a_junction_is_a_link() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("x.bin"), vec![1u8; 50_000]).unwrap();
        let inside = d.path().join("inside");
        std::fs::create_dir(&inside).unwrap();
        junction(&target, &inside.join("link"));
        let entries = sized_entries(&inside).unwrap();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert!(!entries[0].dir && entries[0].bytes == 0, "{entries:?}");
    }

    #[cfg(windows)]
    pub(crate) fn junction(target: &Path, link: &Path) {
        let out = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
