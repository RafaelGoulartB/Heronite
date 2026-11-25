//! A thin layer over Windows' HID API, for the battery drivers
//! (`pad_battery`): the HID collections present, and one opened for reports.
//!
//! Report buffers follow hidapi's conventions, as the protocols the drivers
//! come from are written with them:
//! - [`Device::write`], [`Device::set_feature`]: `data[0]` is the report id
//!   (0 when the collection does not number its reports), and the buffer is
//!   padded with zeros to the report's length;
//! - [`Device::read`]: a leading 0 report id is dropped, so `report[0]` is
//!   the first byte of an unnumbered report, the id of a numbered one.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::Duration;

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidD_FlushQueue, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid, HidD_GetInputReport,
    HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps, HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS,
};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};

/// A HID collection present now: one top-level usage of a device, with the
/// path that opens it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collection {
    /// NUL-terminated, for CreateFileW.
    path: Vec<u16>,
    pub vendor: u16,
    pub product: u16,
    /// The USB interface (`MI_xx` in the path), when the path says.
    pub interface: Option<u8>,
    pub usage_page: u16,
    pub usage: u16,
    /// Report lengths in bytes, the report id included.
    pub input_len: usize,
    pub output_len: usize,
    pub feature_len: usize,
}

impl Collection {
    /// A collection with these ids and lengths and no path, for tests of
    /// what drivers pick.
    pub fn new(vendor: u16, product: u16, usage_page: u16, usage: u16, lens: (usize, usize, usize)) -> Self {
        let (input_len, output_len, feature_len) = lens;
        Self { vendor, product, usage_page, usage, input_len, output_len, feature_len, ..Default::default() }
    }

    /// The device path, for logs.
    pub fn path(&self) -> String {
        String::from_utf16_lossy(self.path.strip_suffix(&[0]).unwrap_or(&self.path))
    }

    /// Whether the controller's game input runs through XInput (its path
    /// has `IG_`): XInput names it then, see `gamepad::product`.
    pub fn is_xinput(&self) -> bool {
        self.path().to_ascii_uppercase().contains("&IG_")
    }
}

/// Every HID collection present. Each is opened without access to read its
/// ids and capabilities: nothing is sent to the devices.
pub fn collections() -> io::Result<Vec<Collection>> {
    Ok(paths()?.into_iter().filter_map(describe).collect())
}

/// The device paths of every HID collection present.
fn paths() -> io::Result<Vec<Vec<u16>>> {
    let mut guid = windows_sys::core::GUID::from_u128(0);
    // SAFETY: `guid` is writable.
    unsafe { HidD_GetHidGuid(&mut guid) };
    // SAFETY: the GUID outlives the call; the list is destroyed below.
    let set = unsafe { SetupDiGetClassDevsW(&guid, std::ptr::null(), std::ptr::null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE) };
    if set == INVALID_HANDLE_VALUE as isize {
        return Err(io::Error::last_os_error());
    }
    let mut paths = Vec::new();
    for index in 0.. {
        let mut iface = SP_DEVICE_INTERFACE_DATA { cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        // SAFETY: the set is open; `iface` is writable with its size set.
        if unsafe { SetupDiEnumDeviceInterfaces(set, std::ptr::null(), &guid, index, &mut iface) } == 0 {
            break;
        }
        let mut needed = 0;
        // SAFETY: a size query (no buffer); `needed` is writable.
        unsafe { SetupDiGetDeviceInterfaceDetailW(set, &iface, std::ptr::null_mut(), 0, &mut needed, std::ptr::null_mut()) };
        if (needed as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
            continue;
        }
        // u32 words keep the structure aligned.
        let mut buf = vec![0u32; (needed as usize).div_ceil(4)];
        let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        // SAFETY: `buf` holds `needed` bytes, aligned for the structure, whose
        // size field is set as Windows asks.
        let ok = unsafe {
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            SetupDiGetDeviceInterfaceDetailW(set, &iface, detail, needed, std::ptr::null_mut(), std::ptr::null_mut())
        };
        if ok == 0 {
            continue;
        }
        // SAFETY: Windows wrote a NUL-terminated path inside `buf`, from DevicePath on.
        let path = unsafe {
            let start = std::ptr::addr_of!((*detail).DevicePath).cast::<u16>();
            let max = (needed as usize - std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath)) / 2;
            let len = (0..max).position(|i| *start.add(i) == 0).unwrap_or(max);
            let mut path = std::slice::from_raw_parts(start, len).to_vec();
            path.push(0);
            path
        };
        paths.push(path);
    }
    // SAFETY: the set came from SetupDiGetClassDevsW and is destroyed once.
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    Ok(paths)
}

/// The ids and capabilities of the collection at `path`; `None` when it
/// cannot be opened even without access (gone since).
fn describe(path: Vec<u16>) -> Option<Collection> {
    let handle = open(&path, 0, 0).ok()?;
    let raw = handle.as_raw_handle() as HANDLE;
    let mut attributes = HIDD_ATTRIBUTES { Size: size_of::<HIDD_ATTRIBUTES>() as u32, ..Default::default() };
    // SAFETY: the handle is open; `attributes` is writable with its size set.
    if !unsafe { HidD_GetAttributes(raw, &mut attributes) } {
        return None;
    }
    let mut data = 0;
    // SAFETY: the handle is open; `data` is freed below.
    if !unsafe { HidD_GetPreparsedData(raw, &mut data) } {
        return None;
    }
    let mut caps = HIDP_CAPS::default();
    // SAFETY: `data` is the collection's preparsed data, freed once after.
    let status = unsafe { HidP_GetCaps(data, &mut caps) };
    // SAFETY: as above.
    unsafe { HidD_FreePreparsedData(data) };
    if status != HIDP_STATUS_SUCCESS {
        return None;
    }
    let interface = interface_of(&String::from_utf16_lossy(&path));
    Some(Collection {
        path,
        vendor: attributes.VendorID,
        product: attributes.ProductID,
        interface,
        usage_page: caps.UsagePage,
        usage: caps.Usage,
        input_len: caps.InputReportByteLength.into(),
        output_len: caps.OutputReportByteLength.into(),
        feature_len: caps.FeatureReportByteLength.into(),
    })
}

/// The USB interface number in a HID path (`…&mi_01&col04…`).
fn interface_of(path: &str) -> Option<u8> {
    let lower = path.to_ascii_lowercase();
    let at = lower.find("&mi_")? + 4;
    u8::from_str_radix(lower.get(at..at + 2)?, 16).ok()
}

fn open(path: &[u16], access: u32, flags: u32) -> io::Result<OwnedHandle> {
    // SAFETY: `path` is NUL-terminated; the handle is owned below.
    let raw = unsafe {
        CreateFileW(path.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, flags, std::ptr::null_mut())
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a handle CreateFileW just opened, owned once.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// An opened collection. Closing it (dropping) is what ends a stream some
/// requests start.
pub struct Device {
    handle: OwnedHandle,
    event: OwnedHandle,
    input_len: usize,
    output_len: usize,
    feature_len: usize,
}

impl Device {
    /// Opens `collection` to read and write reports.
    pub fn open(collection: &Collection) -> io::Result<Self> {
        Self::with(collection, GENERIC_READ | GENERIC_WRITE)
    }

    /// Opens `collection` to read reports only: for collections another
    /// program holds for writing, or a driver that only listens.
    pub fn open_read_only(collection: &Collection) -> io::Result<Self> {
        Self::with(collection, GENERIC_READ)
    }

    fn with(collection: &Collection, access: u32) -> io::Result<Self> {
        let handle = open(&collection.path, access, FILE_FLAG_OVERLAPPED)?;
        // SAFETY: a manual-reset event, owned below.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            // SAFETY: a handle CreateEventW just made, owned once.
            event: unsafe { OwnedHandle::from_raw_handle(event) },
            input_len: collection.input_len,
            output_len: collection.output_len,
            feature_len: collection.feature_len,
        })
    }

    fn raw(&self) -> HANDLE {
        self.handle.as_raw_handle() as HANDLE
    }

    /// Drops the input reports queued since it was opened (replies to an
    /// earlier request).
    pub fn flush(&self) {
        // SAFETY: the handle is open.
        unsafe { HidD_FlushQueue(self.raw()) };
    }

    /// Sends an output report: `data[0]` is its id, the rest is padded with
    /// zeros to the report's length. `false` when it did not go out in time.
    pub fn write(&self, data: &[u8], timeout: Duration) -> io::Result<bool> {
        if self.output_len == 0 || data.is_empty() {
            return Ok(false);
        }
        let mut report = vec![0u8; self.output_len];
        let n = data.len().min(report.len());
        report[..n].copy_from_slice(&data[..n]);
        let file = self.raw();
        // SAFETY: `report` and the OVERLAPPED of `run` outlive the operation,
        // which `run` waits for (or cancels and waits for) before returning.
        let moved = self.run(timeout, |ov| unsafe { WriteFile(file, report.as_ptr(), report.len() as u32, std::ptr::null_mut(), ov) })?;
        Ok(moved == Some(report.len()))
    }

    /// The next input report, waiting up to `timeout`: `None` when none came.
    /// A leading 0 report id is dropped (see the module).
    pub fn read(&self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        if self.input_len == 0 {
            return Ok(None);
        }
        let mut report = vec![0u8; self.input_len];
        let (file, ptr, len) = (self.raw(), report.as_mut_ptr(), report.len() as u32);
        // SAFETY: as in `write`; `report` is writable for its length.
        let Some(moved) = self.run(timeout, |ov| unsafe { ReadFile(file, ptr, len, std::ptr::null_mut(), ov) })? else {
            return Ok(None);
        };
        report.truncate(moved);
        if report.first() == Some(&0) {
            report.remove(0);
        }
        Ok((!report.is_empty()).then_some(report))
    }

    /// Reads feature report `id`: the report, `[0]` being its id.
    pub fn get_feature(&self, id: u8) -> io::Result<Vec<u8>> {
        let mut report = vec![0u8; self.feature_len.max(1)];
        report[0] = id;
        // SAFETY: the handle is open; `report` is writable for its length.
        if !unsafe { HidD_GetFeature(self.raw(), report.as_mut_ptr().cast(), report.len() as u32) } {
            return Err(io::Error::last_os_error());
        }
        Ok(report)
    }

    /// Sends feature report `data` (`data[0]` its id), padded to its length.
    pub fn set_feature(&self, data: &[u8]) -> io::Result<()> {
        let mut report = vec![0u8; self.feature_len.max(data.len())];
        report[..data.len()].copy_from_slice(data);
        // SAFETY: the handle is open; `report` is readable for its length.
        if !unsafe { HidD_SetFeature(self.raw(), report.as_ptr().cast(), report.len() as u32) } {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// The current input report `id` through the control pipe (without
    /// waiting for the device to send one): `[0]` is its id.
    pub fn get_input_report(&self, id: u8) -> io::Result<Vec<u8>> {
        let mut report = vec![0u8; self.input_len.max(1)];
        report[0] = id;
        // SAFETY: the handle is open; `report` is writable for its length.
        if !unsafe { HidD_GetInputReport(self.raw(), report.as_mut_ptr().cast(), report.len() as u32) } {
            return Err(io::Error::last_os_error());
        }
        Ok(report)
    }

    /// Starts an overlapped operation and waits for it: the bytes moved,
    /// `None` after `timeout` (it is then cancelled, and waited for).
    fn run(&self, timeout: Duration, start: impl FnOnce(*mut OVERLAPPED) -> i32) -> io::Result<Option<usize>> {
        let file = self.raw();
        let mut ov = OVERLAPPED { hEvent: self.event.as_raw_handle() as HANDLE, ..Default::default() };
        // SAFETY: GetLastError right after the call it describes.
        if start(&mut ov) == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
            return Err(io::Error::last_os_error());
        }
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the event belongs to the operation started above.
        let done = unsafe { WaitForSingleObject(ov.hEvent, ms) } == WAIT_OBJECT_0;
        if !done {
            // SAFETY: cancels this thread's operations on the handle.
            unsafe { CancelIo(file) };
        }
        let mut moved = 0;
        // SAFETY: `ov` is the operation's; waiting (TRUE) keeps `ov` and the
        // buffer alive until Windows is done with them, cancelled or not.
        let ok = unsafe { GetOverlappedResult(file, &ov, &mut moved, 1) } != 0;
        Ok((done && ok).then_some(moved as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::{interface_of, Collection};

    #[test]
    fn the_interface_comes_from_the_path() {
        assert_eq!(interface_of(r"\\?\hid#vid_3537&pid_10c6&mi_01&col04#9&1551f73d&0&0003#{4d1e55b2}"), Some(1));
        assert_eq!(interface_of(r"\\?\hid#vid_3537&pid_10c6&ig_01#a&113d5401&0&0000#{4d1e55b2}"), None);
    }

    #[test]
    fn xinput_collections_are_told_by_their_path() {
        let mut c = Collection::new(0x3537, 0x10C6, 1, 5, (15, 0, 0));
        c.path = r"\\?\hid#vid_3537&pid_10c6&ig_01#a&1#{4d1e55b2}".encode_utf16().chain([0]).collect();
        assert!(c.is_xinput());
        c.path = r"\\?\hid#vid_3537&pid_10c6&mi_01&col04#9&1#{4d1e55b2}".encode_utf16().chain([0]).collect();
        assert!(!c.is_xinput());
    }
}
