//! Data only this Windows user can read back: DPAPI (`CryptProtectData`,
//! bound to the user's logon credentials). Other systems have no such store
//! here: protecting fails, so nothing secret is written in the clear.

use std::io;

/// Purpose text mixed into the protection, so a blob made for something
/// else does not open here.
#[cfg(windows)]
const ENTROPY: &[u8] = b"Heronite secret v1";

#[cfg(windows)]
mod imp {
    use super::ENTROPY;
    use std::io;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI filled `out` with a LocalAlloc'd buffer of cbData bytes.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: freed once, with the allocator DPAPI used.
        unsafe { LocalFree(out.pbData as _) };
        bytes
    }

    pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
        let (input, entropy) = (blob(data), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: the blobs point at live slices for the call; `out` is written by it.
        let ok = unsafe {
            CryptProtectData(&input, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
        let (input, entropy) = (blob(data), blob(ENTROPY));
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: as in `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    pub fn protect(_: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no protected storage on this system"))
    }

    pub fn unprotect(_: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no protected storage on this system"))
    }
}

/// `data` sealed for this Windows user.
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    imp::protect(data)
}

/// What [`protect`] sealed, when this user sealed it on this PC.
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    imp::unprotect(data)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses_what_it_did_not_seal() {
        let sealed = protect(b"refresh token").unwrap();
        assert!(!sealed.windows(7).any(|w| w == b"refresh"), "not in the clear");
        assert_eq!(unprotect(&sealed).unwrap(), b"refresh token");
        assert!(unprotect(b"not a blob").is_err());
        let mut broken = sealed.clone();
        let last = broken.len() - 1;
        broken[last] ^= 0xFF;
        assert!(unprotect(&broken).is_err());
    }
}
