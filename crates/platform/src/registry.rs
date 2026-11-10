//! Reading and writing values of the Windows registry.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegDeleteTreeW, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, REG_DWORD, REG_EXPAND_SZ, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
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

/// A value of the registry, as [`values`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// `REG_SZ` and `REG_EXPAND_SZ` (its variables not expanded).
    Str(String),
    Dword(u32),
    /// Binary, multi-string, 64-bit numbers and the other types: not read.
    Other,
}

/// A key opened with `RegOpenKeyExW`, closed when dropped.
struct Open(HKEY);

impl Open {
    fn new(hive: Hive, key: &str, access: u32) -> Option<Self> {
        let key = wide(key);
        let mut open: HKEY = std::ptr::null_mut();
        // SAFETY: `key` is NUL-terminated; `open` receives the handle on success.
        (unsafe { RegOpenKeyExW(root(hive), key.as_ptr(), 0, access, &mut open) } == 0).then_some(Self(open))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: the key was opened by `Open::new` and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
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

/// The binary value `name` (a `REG_BINARY`) of the key `key`: a monitor's
/// `EDID` under `SYSTEM\CurrentControlSet\Enum\DISPLAY\..\Device Parameters`.
pub fn binary(hive: Hive, key: &str, name: &str) -> Option<Vec<u8>> {
    let root = root(hive);
    let (key, name) = (wide(key), wide(name));
    let mut buf = vec![0u8; 512];
    loop {
        let mut bytes = buf.len() as u32;
        // SAFETY: the strings are NUL-terminated; `buf` holds `bytes` bytes.
        let r = unsafe {
            RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_BINARY, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut bytes)
        };
        match r {
            0 => {
                buf.truncate(bytes as usize);
                return Some(buf);
            }
            // ERROR_MORE_DATA: `bytes` now says how much it needs.
            234 if bytes as usize > buf.len() => buf.resize(bytes as usize, 0),
            _ => return None,
        }
    }
}

/// The names of the keys directly under `key`, in the registry's order; empty
/// when `key` is missing or cannot be read.
pub fn subkeys(hive: Hive, key: &str) -> Vec<String> {
    let key = wide(key);
    let mut open: HKEY = std::ptr::null_mut();
    // SAFETY: `key` is NUL-terminated; `open` receives the handle on success.
    if unsafe { RegOpenKeyExW(root(hive), key.as_ptr(), 0, KEY_ENUMERATE_SUB_KEYS, &mut open) } != 0 {
        return Vec::new();
    }
    let mut names = Vec::new();
    // A key name has at most 255 characters, plus the NUL.
    let mut buf = [0u16; 256];
    for index in 0.. {
        let mut len = buf.len() as u32;
        // SAFETY: `open` is an open key; `buf` holds `len` characters.
        let r = unsafe {
            RegEnumKeyExW(
                open,
                index,
                buf.as_mut_ptr(),
                &mut len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if r != 0 {
            break;
        }
        names.push(String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]));
    }
    // SAFETY: `open` was opened above and is not used afterwards.
    unsafe { RegCloseKey(open) };
    names
}

/// Every value of the key `key` with its name (`""` is the default value),
/// in the registry's order; `None` when `key` is missing or cannot be read.
pub fn values(hive: Hive, key: &str) -> Option<Vec<(String, Value)>> {
    let open = Open::new(hive, key, KEY_QUERY_VALUE)?;
    let mut out = Vec::new();
    // A value name has at most 16383 characters, plus the NUL.
    let mut name = vec![0u16; 16384];
    let mut data = vec![0u8; 1024];
    let mut index = 0;
    loop {
        let mut name_len = name.len() as u32;
        let mut data_len = data.len() as u32;
        let mut kind = 0u32;
        // SAFETY: `open` is an open key; `name` holds `name_len` characters and `data` `data_len` bytes.
        let r = unsafe {
            RegEnumValueW(open.0, index, name.as_mut_ptr(), &mut name_len, std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut data_len)
        };
        match r {
            0 => {}
            // ERROR_MORE_DATA: `data_len` now says how much the value needs.
            234 if data_len as usize > data.len() => {
                data.resize(data_len as usize, 0);
                continue;
            }
            // ERROR_NO_MORE_ITEMS, or the key went away meanwhile.
            _ => break,
        }
        let bytes = &data[..(data_len as usize).min(data.len())];
        let value = match kind {
            REG_SZ | REG_EXPAND_SZ => {
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
                let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
                Value::Str(String::from_utf16_lossy(&units[..end]))
            }
            REG_DWORD => bytes.first_chunk::<4>().map_or(Value::Other, |b| Value::Dword(u32::from_le_bytes(*b))),
            _ => Value::Other,
        };
        out.push((String::from_utf16_lossy(&name[..(name_len as usize).min(name.len())]), value));
        index += 1;
    }
    Some(out)
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

/// Removes the key `key` and every key and value below it; fine when it is
/// not there.
pub fn delete_tree(hive: Hive, key: &str) -> std::io::Result<()> {
    let key = wide(key);
    // SAFETY: `key` is NUL-terminated.
    match unsafe { RegDeleteTreeW(root(hive), key.as_ptr()) } {
        // ERROR_FILE_NOT_FOUND: no such key.
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
    fn reads_a_binary_value() {
        // A REG_BINARY every Windows user profile has; only read.
        let mask = binary(Hive::CurrentUser, r"Control Panel\Desktop", "UserPreferencesMask").unwrap();
        assert!(!mask.is_empty() && mask.len() < 512, "{mask:?}");
        assert_eq!(binary(Hive::CurrentUser, r"Software\Heronite\Missing", "Nothing"), None);
        // Another type is not binary.
        assert_eq!(binary(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "SystemRoot"), None);
    }

    #[test]
    fn lists_the_keys_under_a_key() {
        let versions = subkeys(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT");
        assert!(versions.iter().any(|k| k == "CurrentVersion"), "{versions:?}");
        assert!(subkeys(Hive::CurrentUser, r"Software\Heronite\Missing").is_empty());

        // Its own root: `writes_and_removes_a_value` clears `Software\Heronite` meanwhile.
        let key = format!(r"Software\HeroniteSubkeysTest\{}", std::process::id());
        for child in ["b", "a"] {
            set_string(Hive::CurrentUser, &format!(r"{key}\{child}"), "Value", "x").unwrap();
        }
        let mut found = subkeys(Hive::CurrentUser, &key);
        found.sort();
        assert_eq!(found, ["a", "b"]);
        // A key with values and no keys under it.
        assert!(subkeys(Hive::CurrentUser, &format!(r"{key}\a")).is_empty());
        // SAFETY: a test key of this process, under HKCU\Software\HeroniteSubkeysTest.
        let tree = wide(r"Software\HeroniteSubkeysTest");
        unsafe { windows_sys::Win32::System::Registry::RegDeleteTreeW(HKEY_CURRENT_USER, tree.as_ptr()) };
    }

    #[test]
    fn lists_the_values_of_a_key_with_their_types() {
        use windows_sys::Win32::System::Registry::REG_BINARY;
        // Its own root: `writes_and_removes_a_value` clears `Software\Heronite` meanwhile.
        let key = format!(r"Software\HeroniteValuesTest\{}", std::process::id());
        set_string(Hive::CurrentUser, &key, "", "default").unwrap();
        set_string(Hive::CurrentUser, &key, "InstallDir", "C:/Program Files (x86)/Ubisoft/").unwrap();
        // Longer than the first buffer.
        let long = "x".repeat(3000);
        set_string(Hive::CurrentUser, &key, "Long", &long).unwrap();
        let raw = |name: &str, kind: u32, bytes: &[u8]| {
            let (k, n) = (wide(&key), wide(name));
            // SAFETY: the strings are NUL-terminated; `bytes` holds the value's bytes.
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), kind, bytes.as_ptr().cast(), bytes.len() as u32) }
        };
        assert_eq!(raw("Size", REG_DWORD, &10u32.to_le_bytes()), 0);
        assert_eq!(raw("Blob", REG_BINARY, &[1, 2, 3]), 0);

        let mut found = values(Hive::CurrentUser, &key).unwrap();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            found,
            [
                (String::new(), Value::Str("default".into())),
                ("Blob".into(), Value::Other),
                ("InstallDir".into(), Value::Str("C:/Program Files (x86)/Ubisoft/".into())),
                ("Long".into(), Value::Str(long)),
                ("Size".into(), Value::Dword(10)),
            ]
        );
        assert_eq!(values(Hive::CurrentUser, &format!(r"{key}\Missing")), None);
        let current = values(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").unwrap();
        assert!(current.iter().any(|(n, v)| n == "SystemRoot" && matches!(v, Value::Str(_))), "{current:?}");
        delete_tree(Hive::CurrentUser, &key).unwrap();
        assert_eq!(values(Hive::CurrentUser, &key), None);
        // Already gone: still fine.
        delete_tree(Hive::CurrentUser, &key).unwrap();
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
