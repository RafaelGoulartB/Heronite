//! DuckStation keeps the RetroAchievements token encrypted in `settings.ini`
//! (`Achievements::EncryptLoginToken` in `src/core/achievements.cpp`): AES-128-CBC
//! with a key made from this PC's machine id and the username, so the file
//! is useless on another PC. We write the same format.
//!
//! Key: SHA-256 of `machine id ++ username`, then 100 more rounds of SHA-256
//! on the digest; the first 16 bytes are the key, the last 16 the IV. The
//! token is zero-padded to 16 bytes and stored as standard base64.

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::Engine;
use sha2::{Digest, Sha256};

const EXTRA_ROUNDS: u32 = 100;

type Enc = cbc::Encryptor<aes::Aes128>;
type Dec = cbc::Decryptor<aes::Aes128>;

/// `/etc/machine-id` as DuckStation reads it: the whole file, newline included.
pub fn machine_key() -> Option<String> {
    std::fs::read_to_string("/etc/machine-id").ok().filter(|s| !s.is_empty())
}

fn key_and_iv(machine_key: &str, username: &str) -> ([u8; 16], [u8; 16]) {
    let mut hasher = Sha256::new();
    hasher.update(machine_key.as_bytes());
    hasher.update(username.as_bytes());
    let mut digest = hasher.finalize();
    for _ in 0..EXTRA_ROUNDS {
        digest = Sha256::digest(digest);
    }
    let mut key = [0u8; 16];
    let mut iv = [0u8; 16];
    key.copy_from_slice(&digest[..16]);
    iv.copy_from_slice(&digest[16..]);
    (key, iv)
}

pub fn encrypt(token: &str, username: &str, machine_key: &str) -> String {
    let (key, iv) = key_and_iv(machine_key, username);
    let mut data = token.as_bytes().to_vec();
    data.resize(data.len().div_ceil(16) * 16, 0);
    let len = data.len();
    let encrypted = Enc::new(&key.into(), &iv.into())
        .encrypt_padded_mut::<NoPadding>(&mut data, len)
        .expect("the buffer is a whole number of blocks")
        .to_vec();
    base64::engine::general_purpose::STANDARD.encode(encrypted)
}

pub fn decrypt(encrypted: &str, username: &str, machine_key: &str) -> Option<String> {
    let mut data = base64::engine::general_purpose::STANDARD.decode(encrypted.trim()).ok()?;
    if data.is_empty() || data.len() % 16 != 0 {
        return None;
    }
    let (key, iv) = key_and_iv(machine_key, username);
    let plain = Dec::new(&key.into(), &iv.into()).decrypt_padded_mut::<NoPadding>(&mut data).ok()?;
    let end = plain.iter().position(|b| *b == 0).unwrap_or(plain.len());
    String::from_utf8(plain[..end].to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let enc = encrypt("AbCdEfGhIj", "me", "0123456789abcdef\n");
        assert_eq!(enc.len(), 24);
        assert_eq!(decrypt(&enc, "me", "0123456789abcdef\n").as_deref(), Some("AbCdEfGhIj"));
        let long = "x".repeat(32);
        assert_eq!(decrypt(&encrypt(&long, "me", "k"), "me", "k").as_deref(), Some(long.as_str()));
    }

    #[test]
    fn another_user_or_pc_does_not_decrypt() {
        let enc = encrypt("AbCdEfGhIj", "me", "pc1");
        assert_ne!(decrypt(&enc, "you", "pc1").as_deref(), Some("AbCdEfGhIj"));
        assert_ne!(decrypt(&enc, "me", "pc2").as_deref(), Some("AbCdEfGhIj"));
        assert_eq!(decrypt("AbCdEfGhIj", "me", "pc1"), None);
    }
}
