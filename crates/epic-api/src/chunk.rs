//! Chunk files (`0xB1FE3AA2`): up to 1 MiB of build data, zlib-compressed,
//! encrypted from header version 4 (legendary's `models/chunk.py`).

use std::io::{Read, Write};
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use sha1_smol::Sha1;

use crate::bin_io::{Reader, Writer};
use crate::crypt::{self, Secrets};
use crate::manifest::{ChunkInfo, Guid};

const MAGIC: u32 = 0xB1FE_3AA2;
const STORED_COMPRESSED: u8 = 0x1;
const STORED_ENCRYPTED: u8 = 0x2;
const HASH_ROLLING: u8 = 0x1;
const HASH_SHA1: u8 = 0x2;
/// Every chunk holds this much data (new ones are padded to it).
pub const CHUNK_SIZE: usize = 1024 * 1024;

/// What a chunk file holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub guid: Guid,
    pub hash: u64,
    pub sha_hash: Option<[u8; 20]>,
    pub data: Vec<u8>,
}

/// Reads a chunk file: decrypts it with `secrets` when it is encrypted,
/// inflates it and checks the hashes the header carries.
pub fn decode(raw: &[u8], secrets: &Secrets) -> Result<Chunk> {
    let mut r = Reader::new(raw);
    if r.u32()? != MAGIC {
        bail!("not an Epic chunk (no header magic)");
    }
    let header_version = r.u32()?;
    let header_size = r.u32()? as usize;
    let compressed_size = r.u32()? as usize;
    let guid = Guid(r.guid()?);
    let hash = r.u64()?;
    let stored_as = r.u8()?;
    let (mut sha, mut hash_type) = (None, 0);
    if header_version >= 2 {
        sha = Some(r.array::<20>()?);
        hash_type = r.u8()?;
    }
    if header_version >= 3 {
        let _uncompressed = r.u32()?;
    }
    let mut secret = [0u32; 4];
    let mut tag = [0u8; 16];
    if header_version >= 4 {
        secret = r.guid()?;
        tag = r.array()?;
    }
    if r.pos() != header_size {
        bail!("the chunk header is {} bytes, not {header_size}", r.pos());
    }
    let payload = r.rest();
    let payload = if compressed_size > 0 && compressed_size <= payload.len() { &payload[..compressed_size] } else { payload };
    let mut data = if stored_as & STORED_ENCRYPTED != 0 {
        let nonce = sha.map(|s| s[..12].to_vec()).unwrap_or_default();
        crypt::decrypt(secrets, &secret, &nonce, &tag, payload).context("decrypting the chunk")?
    } else {
        payload.to_vec()
    };
    if stored_as & STORED_COMPRESSED != 0 {
        let mut out = Vec::with_capacity(CHUNK_SIZE);
        flate2::read::ZlibDecoder::new(&data[..]).read_to_end(&mut out).context("decompressing the chunk")?;
        data = out;
    }
    if let (true, Some(expected)) = (hash_type & HASH_SHA1 != 0, sha) {
        if Sha1::from(&data).digest().bytes() != expected {
            bail!("the chunk {} is damaged (SHA-1 mismatch)", guid.text());
        }
    } else if hash_type & HASH_ROLLING != 0 && rolling_hash(&data) != hash {
        bail!("the chunk {} is damaged (hash mismatch)", guid.text());
    }
    Ok(Chunk { guid, hash, sha_hash: sha, data })
}

/// [`decode`] for a chunk of a manifest: it must be that chunk, with the
/// manifest's SHA-1.
pub fn decode_for(raw: &[u8], info: &ChunkInfo, secrets: &Secrets) -> Result<Vec<u8>> {
    let chunk = decode(raw, secrets)?;
    if chunk.guid != info.guid {
        bail!("got the chunk {} instead of {}", chunk.guid.text(), info.guid.text());
    }
    if info.sha_hash != [0; 20] && Sha1::from(&chunk.data).digest().bytes() != info.sha_hash {
        bail!("the chunk {} is damaged (SHA-1 mismatch)", info.guid.text());
    }
    Ok(chunk.data)
}

/// Packs `data` (at most 1 MiB, padded to it) into a new chunk file, header
/// version 3 like legendary writes. Returns the file and the chunk's entry
/// for a manifest.
pub fn encode(guid: Guid, data: &[u8]) -> Result<(Vec<u8>, ChunkInfo)> {
    if data.len() > CHUNK_SIZE {
        bail!("a chunk holds at most 1 MiB");
    }
    let mut padded = data.to_vec();
    padded.resize(CHUNK_SIZE, 0);
    let hash = rolling_hash(&padded);
    let sha = Sha1::from(&padded).digest().bytes();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&padded)?;
    let compressed = z.finish()?;

    let mut w = Writer::default();
    w.u32(MAGIC);
    w.u32(3);
    w.u32(66);
    w.u32(compressed.len() as u32);
    w.guid(&guid.0);
    w.u64(hash);
    w.u8(STORED_COMPRESSED);
    w.bytes(&sha);
    w.u8(HASH_ROLLING | HASH_SHA1);
    w.u32(CHUNK_SIZE as u32);
    w.bytes(&compressed);
    let info = ChunkInfo {
        guid,
        hash,
        sha_hash: sha,
        group_num: guid.group_num(),
        window_size: CHUNK_SIZE as u32,
        file_size: w.buf.len() as u64,
        ..Default::default()
    };
    Ok((w.buf, info))
}

/// Epic's rolling hash, a variant of CRC-64-ECMA.
pub fn rolling_hash(data: &[u8]) -> u64 {
    static TABLE: OnceLock<[u64; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        const POLY: u64 = 0xC96C_5795_D787_0F42;
        let mut t = [0u64; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut v = i as u64;
            for _ in 0..8 {
                v = if v & 1 != 0 { (v >> 1) ^ POLY } else { v >> 1 };
            }
            *slot = v;
        }
        t
    });
    data.iter().fold(0u64, |h, b| h.rotate_left(1) ^ table[*b as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_round_trip() {
        let guid = Guid([1, 2, 3, 4]);
        let (raw, info) = encode(guid, b"hello save").unwrap();
        let chunk = decode(&raw, &Secrets::new()).unwrap();
        assert_eq!(chunk.guid, guid);
        assert_eq!(&chunk.data[..10], b"hello save");
        assert_eq!(chunk.data.len(), CHUNK_SIZE);
        assert_eq!(info.file_size, raw.len() as u64);
        assert_eq!(decode_for(&raw, &info, &Secrets::new()).unwrap().len(), CHUNK_SIZE);
        let other = ChunkInfo { guid: Guid([9, 9, 9, 9]), ..info.clone() };
        assert!(decode_for(&raw, &other, &Secrets::new()).is_err());
        let mut damaged = raw.clone();
        let last = damaged.len() - 3;
        damaged[last] ^= 0xFF;
        assert!(decode(&damaged, &Secrets::new()).is_err());
    }

    #[test]
    fn rolling_hash_matches_legendarys() {
        // What legendary's `get_hash` gives.
        assert_eq!(rolling_hash(b""), 0);
        assert_eq!(rolling_hash(b"abc"), 0xbc9d3899098133a7);
        assert_eq!(rolling_hash("Heronite".repeat(100).as_bytes()), 0x533c5e67c7a8caf3);
    }

    #[test]
    fn encrypted_chunks_need_their_key() {
        let key = [5u8; 32];
        let data = vec![7u8; 64];
        let sha = Sha1::from(&data).digest().bytes();
        let (cipher, tag) = crypt::encrypt(&key, &sha[..12], &data);
        let secret = [1, 2, 3, 4];
        let mut w = Writer::default();
        w.u32(MAGIC);
        w.u32(4);
        w.u32(98);
        w.u32(cipher.len() as u32);
        w.guid(&[8, 8, 8, 8]);
        w.u64(0);
        w.u8(STORED_ENCRYPTED);
        w.bytes(&sha);
        w.u8(HASH_SHA1);
        w.u32(64);
        w.guid(&secret);
        w.bytes(&tag);
        w.bytes(&cipher);
        assert!(decode(&w.buf, &Secrets::new()).is_err());
        let mut secrets = Secrets::new();
        secrets.insert(crypt::secret_name(&secret), hex::encode(key));
        assert_eq!(decode(&w.buf, &secrets).unwrap().data, data);
    }
}
