//! AES-GCM decryption of encrypted manifests and chunks (manifest version 22
//! and later). The keys come with the manifest's download answer
//! (`secrets`: secret GUID → hex key).

use std::collections::BTreeMap;

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce, Tag};
use anyhow::{anyhow, bail, Result};

/// Secret GUID (upper-case hex of its four numbers) → key (hex).
pub type Secrets = BTreeMap<String, String>;

/// The key name of a secret GUID: `{:08X}` of each number, joined.
pub fn secret_name(guid: &[u32; 4]) -> String {
    guid.iter().map(|g| format!("{g:08X}")).collect()
}

/// Whether a GUID names no secret (all zero).
pub fn is_plain(guid: &[u32; 4]) -> bool {
    guid.iter().all(|g| *g == 0)
}

/// Decrypts `data` with the key of `secret` from `secrets`.
pub fn decrypt(secrets: &Secrets, secret: &[u32; 4], nonce: &[u8], tag: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    let name = secret_name(secret);
    let key_hex = secrets.get(&name).ok_or_else(|| anyhow!("the key {name} needed to decrypt this download is missing"))?;
    let key = hex::decode(key_hex).map_err(|e| anyhow!("invalid key {name}: {e}"))?;
    if nonce.len() != 12 {
        bail!("unsupported nonce length {}", nonce.len());
    }
    if tag.len() != 16 {
        bail!("unsupported tag length {}", tag.len());
    }
    let mut buf = data.to_vec();
    let nonce = Nonce::from_slice(nonce);
    let tag = Tag::from_slice(tag);
    let result = match key.len() {
        16 => Aes128Gcm::new_from_slice(&key).expect("16 bytes").decrypt_in_place_detached(nonce, b"", &mut buf, tag),
        32 => Aes256Gcm::new_from_slice(&key).expect("32 bytes").decrypt_in_place_detached(nonce, b"", &mut buf, tag),
        n => bail!("unsupported key length {n}"),
    };
    result.map_err(|_| anyhow!("decryption failed: wrong key or damaged data"))?;
    Ok(buf)
}

/// Encrypts like Epic does (for tests: the reverse of [`decrypt`]).
#[cfg(test)]
pub(crate) fn encrypt(key: &[u8], nonce: &[u8], data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut buf = data.to_vec();
    let tag = Aes256Gcm::new_from_slice(key).unwrap().encrypt_in_place_detached(Nonce::from_slice(nonce), b"", &mut buf).unwrap();
    (buf, tag.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_with_the_named_key() {
        let key = [7u8; 32];
        let nonce = [1u8; 12];
        let (cipher, tag) = encrypt(&key, &nonce, b"secret data");
        let guid = [0xA, 0xB, 0xC, 0xD];
        let mut secrets = Secrets::new();
        secrets.insert(secret_name(&guid), hex::encode(key));
        assert_eq!(secret_name(&guid), "0000000A0000000B0000000C0000000D");
        assert_eq!(decrypt(&secrets, &guid, &nonce, &tag, &cipher).unwrap(), b"secret data");
        assert!(decrypt(&Secrets::new(), &guid, &nonce, &tag, &cipher).is_err());
        let mut bad = cipher.clone();
        bad[0] ^= 1;
        assert!(decrypt(&secrets, &guid, &nonce, &tag, &bad).is_err());
    }
}
