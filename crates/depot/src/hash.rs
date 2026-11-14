//! The hashes stores give their files: SHA-1 (Epic), MD5 (GOG), SHA-256.

use std::io::Read;

use serde::{Deserialize, Serialize};
use sha2::Digest;

/// A whole-file hash to check a written file against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Hash {
    Sha1([u8; 20]),
    Md5([u8; 16]),
    Sha256([u8; 32]),
}

impl Hash {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Hash::Sha1(b) => b,
            Hash::Md5(b) => b,
            Hash::Sha256(b) => b,
        }
    }

    pub fn sha1_hex(text: &str) -> Option<Self> {
        unhex(text).map(Hash::Sha1)
    }

    pub fn md5_hex(text: &str) -> Option<Self> {
        unhex(text).map(Hash::Md5)
    }

    pub fn sha256_hex(text: &str) -> Option<Self> {
        unhex(text).map(Hash::Sha256)
    }

    /// Lowercase hex.
    pub fn hex(&self) -> String {
        crate::plan::hex(self.bytes())
    }

    /// A hasher of the same kind.
    pub fn hasher(&self) -> Hasher {
        match self {
            Hash::Sha1(_) => Hasher::Sha1(sha1_smol::Sha1::new()),
            Hash::Md5(_) => Hasher::Md5(md5::Md5::new()),
            Hash::Sha256(_) => Hasher::Sha256(sha2::Sha256::new()),
        }
    }

    /// Hashes everything `reader` gives with a hasher of this kind.
    pub fn of_reader(&self, reader: &mut dyn Read) -> std::io::Result<Hash> {
        let mut hasher = self.hasher();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                return Ok(hasher.finish());
            }
            hasher.update(&buf[..n]);
        }
    }
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.trim().as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; N];
    for (i, pair) in text.chunks(2).enumerate() {
        out[i] = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// A running hash of one of the [`Hash`] kinds.
pub enum Hasher {
    Sha1(sha1_smol::Sha1),
    Md5(md5::Md5),
    Sha256(sha2::Sha256),
}

impl Hasher {
    pub fn update(&mut self, bytes: &[u8]) {
        match self {
            Hasher::Sha1(h) => h.update(bytes),
            Hasher::Md5(h) => h.update(bytes),
            Hasher::Sha256(h) => h.update(bytes),
        }
    }

    pub fn finish(self) -> Hash {
        match self {
            Hasher::Sha1(h) => Hash::Sha1(h.digest().bytes()),
            Hasher::Md5(h) => Hash::Md5(h.finalize().into()),
            Hasher::Sha256(h) => Hash::Sha256(h.finalize().into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let of = |h: Hash| h.of_reader(&mut &b"abc"[..]).unwrap().hex();
        assert_eq!(of(Hash::Sha1([0; 20])), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(of(Hash::Md5([0; 16])), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(of(Hash::Sha256([0; 32])), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn hex_round_trips() {
        let h = Hash::md5_hex("900150983CD24FB0D6963F7D28E17F72").unwrap();
        assert_eq!(h.hex(), "900150983cd24fb0d6963f7d28e17f72");
        assert!(Hash::md5_hex("90").is_none());
        assert!(Hash::sha1_hex("zz9993e364706816aba3e25717850c26c9cd0d89d").is_none());
    }
}
