//! Reading and writing values of the Windows registry.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

fn root(hive: Hive) -> HKEY {
    match hive {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0]).collect()
}

/// The number value `name` (a `REG_DWORD`) of the key `key`.
pub fn dword(hive: Hive, key: &str, name: &str) -> Option<u32> {
    let (key, name) = (wide(key), wide(name));
    let mut value = 0u32;
    let mut bytes = std::mem::size_of::<u32>() as u32;
    // SAFETY: the strings are NUL-terminated; `value` holds `bytes` bytes.
    let r = unsafe {
        RegGetValueW(
            root(hive),
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        )
    };
    (r == 0).then_some(value)
}

/// The text value `name` of the key `key` (`Software\Valve\Steam`).
pub fn string(hive: Hive, key: &str, name: &str) -> Option<String> {
    let root = root(hive);
    let (key, name) = (wide(key), wide(name));
    let mut buf = vec![0u16; 1024];
    loop {
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: the strings are NUL-terminated; `buf` holds `bytes` bytes.
        let r = unsafe {
            RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
        };
        match r {
            0 => {
                let len = (bytes as usize / 2).min(buf.len());
                let text = &buf[..len];
                let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
                return Some(String::from_utf16_lossy(&text[..end]));
            }
            // ERROR_MORE_DATA: `bytes` now says how much it needs.
            234 if (bytes as usize) / 2 > buf.len() => buf.resize(bytes as usize / 2 + 1, 0),
            _ => return None,
        }
    }
}

/// Sets the text value `name` of the key `key`, creating the key if needed.
pub fn set_string(hive: Hive, key: &str, name: &str, value: &str) -> std::io::Result<()> {
    let (key, name, data) = (wide(key), wide(name), wide(value));
    let bytes = (data.len() * 2) as u32;
    // SAFETY: the strings are NUL-terminated; `data` holds `bytes` bytes, its NUL included.
    let r = unsafe { RegSetKeyValueW(root(hive), key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), bytes) };
    result(r)
}

/// Removes the value `name` of the key `key`; fine when it is not there.
pub fn delete_value(hive: Hive, key: &str, name: &str) -> std::io::Result<()> {
    let (key, name) = (wide(key), wide(name));
    // SAFETY: both strings are NUL-terminated.
    let r = unsafe { RegDeleteKeyValueW(root(hive), key.as_ptr(), name.as_ptr()) };
    match r {
        // ERROR_FILE_NOT_FOUND: no such key or value.
        2 => Ok(()),
        r => result(r),
    }
}

fn result(code: u32) -> std::io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(code as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_value_every_windows_has() {
        let root = string(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "SystemRoot").unwrap();
        assert!(root.to_lowercase().ends_with("windows"), "{root}");
        assert_eq!(string(Hive::CurrentUser, r"Software\Heronite\Missing", "Nothing"), None);
        let build = dword(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentMajorVersionNumber");
        assert!(build.is_some_and(|v| v >= 10), "{build:?}");
    }

    #[test]
    fn writes_and_removes_a_value() {
        let key = format!(r"Software\Heronite\Tests\{}", std::process::id());
        set_string(Hive::CurrentUser, &key, "Value", r#""C:\Program Files\x.exe" --flag"#).unwrap();
        assert_eq!(string(Hive::CurrentUser, &key, "Value").as_deref(), Some(r#""C:\Program Files\x.exe" --flag"#));
        delete_value(Hive::CurrentUser, &key, "Value").unwrap();
        assert_eq!(string(Hive::CurrentUser, &key, "Value"), None);
        // Already gone: still fine.
        delete_value(Hive::CurrentUser, &key, "Value").unwrap();
        // SAFETY: a test key of this process, under HKCU\Software\Heronite.
        let tree = wide(r"Software\Heronite");
        unsafe { windows_sys::Win32::System::Registry::RegDeleteTreeW(HKEY_CURRENT_USER, tree.as_ptr()) };
    }
}
