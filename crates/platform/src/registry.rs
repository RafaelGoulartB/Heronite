//! Reading the Windows registry.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

/// The text value `name` of the key `key` (`Software\Valve\Steam`).
pub fn string(hive: Hive, key: &str, name: &str) -> Option<String> {
    let root: HKEY = match hive {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    };
    let wide = |s: &str| OsStr::new(s).encode_wide().chain([0]).collect::<Vec<u16>>();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_value_every_windows_has() {
        let root = string(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "SystemRoot").unwrap();
        assert!(root.to_lowercase().ends_with("windows"), "{root}");
        assert_eq!(string(Hive::CurrentUser, r"Software\Heronite\Missing", "Nothing"), None);
    }
}
