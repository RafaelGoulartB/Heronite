//! Epic's build manifests: what files a build has and which chunk bytes make
//! each one. Binary manifests (`0x44BEC00C`, zlib, optionally encrypted) and
//! the old JSON ones ([`json`]) read into one model; [`Manifest::to_bytes`]
//! writes the binary form (the stored copy of an install, cloud save
//! manifests). [`plan`] turns a manifest (and the installed one) into the work
//! of an install, update or repair.
//!
//! The layout follows legendary's `models/manifest.py` (the same format the
//! Epic Games Launcher reads).

pub mod json;
pub mod plan;

use std::io::{Read, Write};

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha1_smol::Sha1;

use crate::bin_io::{Reader, Writer};
use crate::crypt::{self, Secrets};

pub use plan::{InstallPlan, PlanOptions, PlannedFile, PlannedPart};

const MAGIC: u32 = 0x44BE_C00C;
/// Version a manifest is written with unless its features need a newer one
/// (legendary's `default_serialisation_version`).
const DEFAULT_SERIALISATION: u32 = 17;
const STORED_COMPRESSED: u8 = 0x1;
const STORED_ENCRYPTED: u8 = 0x2;

/// A 128-bit id: four little-endian `u32`s (chunks, secrets).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub [u32; 4]);

impl Guid {
    /// legendary's `guid_num`: the four numbers as one, first one highest.
    pub fn num(&self) -> u128 {
        let g = self.0;
        (g[0] as u128) << 96 | (g[1] as u128) << 64 | (g[2] as u128) << 32 | g[3] as u128
    }

    /// `aaaaaaaa-bbbbbbbb-cccccccc-dddddddd` (legendary's `guid_str`).
    pub fn text(&self) -> String {
        self.0.iter().map(|g| format!("{g:08x}")).collect::<Vec<_>>().join("-")
    }

    pub fn le_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        for (i, g) in self.0.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&g.to_le_bytes());
        }
        out
    }

    /// A random id (new cloud save chunks).
    pub fn random() -> Self {
        let b = *uuid::Uuid::new_v4().as_bytes();
        let n = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Guid([n(0), n(4), n(8), n(12)])
    }

    /// The folder number of a chunk without one stored: CRC-32 of the id's
    /// bytes, modulo 100.
    pub fn group_num(&self) -> u8 {
        (crc32(&self.le_bytes()) % 100) as u8
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

/// Build information: the app, its version, how it starts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub data_version: u8,
    /// The manifest's feature level; it picks the chunk folder.
    pub feature_level: u32,
    pub is_file_data: bool,
    pub app_id: u32,
    pub app_name: String,
    pub build_version: String,
    pub launch_exe: String,
    pub launch_command: String,
    pub prereq_ids: Vec<String>,
    pub prereq_name: String,
    pub prereq_path: String,
    pub prereq_args: String,
    /// Stored from data version 1; computed otherwise (see [`Meta::build_id`]).
    stored_build_id: String,
    pub uninstall_action_path: String,
    pub uninstall_action_args: String,
}

impl Meta {
    /// The build id (delta manifests are named by it). Old manifests do not
    /// store one: it is then the URL-safe base64 of a SHA-1 over the app id,
    /// name, version, program and command line.
    pub fn build_id(&self) -> String {
        if !self.stored_build_id.is_empty() {
            return self.stored_build_id.clone();
        }
        let mut s = Sha1::new();
        s.update(&self.app_id.to_le_bytes());
        s.update(self.app_name.as_bytes());
        s.update(self.build_version.as_bytes());
        s.update(self.launch_exe.as_bytes());
        s.update(self.launch_command.as_bytes());
        URL_SAFE_NO_PAD.encode(s.digest().bytes())
    }

    fn read(r: &mut Reader) -> Result<Self> {
        let start = r.pos();
        let size = r.u32()? as usize;
        let mut m = Meta {
            data_version: r.u8()?,
            feature_level: r.u32()?,
            is_file_data: r.u8()? == 1,
            app_id: r.u32()?,
            app_name: r.fstring()?,
            build_version: r.fstring()?,
            launch_exe: r.fstring()?,
            launch_command: r.fstring()?,
            ..Default::default()
        };
        let n = r.u32()?;
        for _ in 0..n {
            m.prereq_ids.push(r.fstring()?);
        }
        m.prereq_name = r.fstring()?;
        m.prereq_path = r.fstring()?;
        m.prereq_args = r.fstring()?;
        if m.data_version >= 1 {
            m.stored_build_id = r.fstring()?;
        }
        if m.data_version >= 2 {
            m.uninstall_action_path = r.fstring()?;
            m.uninstall_action_args = r.fstring()?;
        }
        if r.pos() - start != size {
            log::debug!("manifest metadata: read {} of {size} bytes", r.pos() - start);
            r.seek(start + size);
            m.data_version = 0;
        }
        Ok(m)
    }

    fn write(&self, w: &mut Writer) {
        let start = w.len();
        w.u32(0);
        w.u8(self.data_version);
        w.u32(self.feature_level);
        w.u8(self.is_file_data as u8);
        w.u32(self.app_id);
        w.fstring(&self.app_name);
        w.fstring(&self.build_version);
        w.fstring(&self.launch_exe);
        w.fstring(&self.launch_command);
        w.u32(self.prereq_ids.len() as u32);
        for id in &self.prereq_ids {
            w.fstring(id);
        }
        w.fstring(&self.prereq_name);
        w.fstring(&self.prereq_path);
        w.fstring(&self.prereq_args);
        if self.data_version >= 1 {
            w.fstring(&self.build_id());
        }
        if self.data_version >= 2 {
            w.fstring(&self.uninstall_action_path);
            w.fstring(&self.uninstall_action_args);
        }
        let size = (w.len() - start) as u32;
        w.patch_u32(start, size);
    }
}

/// One chunk of the build: up to 1 MiB of data, downloaded compressed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChunkInfo {
    pub guid: Guid,
    /// Epic's rolling hash of the data.
    pub hash: u64,
    /// SHA-1 of the uncompressed data.
    pub sha_hash: [u8; 20],
    pub group_num: u8,
    /// Uncompressed size, bytes.
    pub window_size: u32,
    /// Download size (the chunk file), bytes.
    pub file_size: u64,
    /// Zero when the chunk is not encrypted.
    pub secret_guid: Guid,
    pub window_size_compressed: u32,
    pub encryption_tag: [u8; 16],
}

impl ChunkInfo {
    /// The chunk's path below a base URL, for a manifest of `feature_level`.
    pub fn path(&self, feature_level: u32) -> String {
        let dir = chunk_dir(feature_level);
        if feature_level >= 22 {
            let secret = if crypt::is_plain(&self.secret_guid.0) {
                "plain".to_string()
            } else {
                URL_SAFE_NO_PAD.encode(self.secret_guid.le_bytes())
            };
            let hash = URL_SAFE_NO_PAD.encode(self.hash.to_le_bytes());
            let guid = URL_SAFE_NO_PAD.encode(self.guid.le_bytes());
            return format!("{dir}/{secret}/{:02}/{hash}_{guid}.chunk", self.group_num);
        }
        let guid: String = self.guid.0.iter().map(|g| format!("{g:08X}")).collect();
        format!("{dir}/{:02}/{:016X}_{guid}.chunk", self.group_num, self.hash)
    }
}

/// legendary's `get_chunk_dir`.
pub fn chunk_dir(feature_level: u32) -> &'static str {
    match feature_level {
        22.. => "ChunksV5",
        15..=21 => "ChunksV4",
        6..=14 => "ChunksV3",
        3..=5 => "ChunksV2",
        _ => "Chunks",
    }
}

/// A piece of a file: `size` bytes at `offset` of a chunk's data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChunkPart {
    pub guid: Guid,
    pub offset: u32,
    pub size: u32,
}

/// One file of the build.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileManifest {
    /// Relative path with `/`.
    pub filename: String,
    pub symlink_target: String,
    /// SHA-1 of the whole file.
    pub hash: [u8; 20],
    pub flags: u8,
    pub install_tags: Vec<String>,
    pub chunk_parts: Vec<ChunkPart>,
    pub hash_md5: Option<[u8; 16]>,
    pub mime_type: String,
    pub hash_sha256: Option<[u8; 32]>,
}

impl FileManifest {
    pub fn size(&self) -> u64 {
        self.chunk_parts.iter().map(|c| c.size as u64).sum()
    }

    pub fn read_only(&self) -> bool {
        self.flags & 0x1 != 0
    }

    pub fn executable(&self) -> bool {
        self.flags & 0x4 != 0
    }

    /// Whether the file belongs to an install with `tags` selected: untagged
    /// files go with the `""` tag (legendary's selection rule).
    pub fn selected_by(&self, tags: &[String]) -> bool {
        tags.iter().any(|t| self.install_tags.contains(t) || (t.is_empty() && self.install_tags.is_empty()))
    }
}

/// The encrypted part of a manifest (feature level 24+): program, command
/// line, prerequisites and every file name, until [`Manifest::decrypt`].
#[derive(Debug, Clone, Default, PartialEq)]
struct EncryptedData {
    version: u8,
    header_version: u32,
    header_stored_as: u8,
    data_uncompressed: u32,
    data_compressed: u32,
    iv: Vec<u8>,
    ciphertext: Vec<u8>,
}

impl EncryptedData {
    fn read(r: &mut Reader) -> Result<Self> {
        let start = r.pos();
        let size = r.u32()? as usize;
        let version = r.u8()?;
        let cipher_size = r.u32()? as usize;
        let mut inner = Reader::new(r.bytes(cipher_size)?);
        let head_start = inner.pos();
        let head_size = inner.u32()? as usize;
        let mut e = EncryptedData {
            version,
            header_version: inner.u32()?,
            header_stored_as: inner.u8()?,
            data_uncompressed: inner.u32()?,
            data_compressed: inner.u32()?,
            ..Default::default()
        };
        let iv_len = inner.u32()? as usize;
        e.iv = inner.bytes(iv_len)?.to_vec();
        inner.seek(head_start + head_size);
        let len = inner.u32()? as usize;
        e.ciphertext = inner.bytes(len)?.to_vec();
        r.seek(start + size);
        Ok(e)
    }
}

/// A parsed manifest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    /// The header's serialisation version.
    pub version: u32,
    stored_as: u8,
    secret_guid: Guid,
    encryption_tag: [u8; 16],
    pub meta: Meta,
    pub cdl_version: u8,
    pub chunks: Vec<ChunkInfo>,
    pub fml_version: u8,
    pub files: Vec<FileManifest>,
    pub custom_fields_version: u8,
    /// In file order.
    pub custom_fields: Vec<(String, String)>,
    encrypted: Option<EncryptedData>,
    /// Read from a JSON manifest.
    pub is_json: bool,
}

impl Manifest {
    /// Reads a binary or JSON manifest.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.first() == Some(&b'{') {
            return json::parse(data);
        }
        let mut r = Reader::new(data);
        if r.u32()? != MAGIC {
            bail!("not an Epic manifest (no header magic)");
        }
        let header_size = r.u32()? as usize;
        let size_uncompressed = r.u32()? as usize;
        let _size_compressed = r.u32()?;
        let sha: [u8; 20] = r.array()?;
        let stored_as = r.u8()?;
        let version = r.u32()?;
        let mut m = Manifest { version, stored_as, ..Default::default() };
        if version >= 22 {
            m.secret_guid = Guid(r.guid()?);
            m.encryption_tag = r.array()?;
        }
        if r.pos() != header_size {
            log::debug!("manifest header: read {} of {header_size} bytes", r.pos());
            r.seek(header_size);
        }
        let body = if stored_as & STORED_COMPRESSED != 0 {
            let mut out = Vec::with_capacity(size_uncompressed);
            flate2::read::ZlibDecoder::new(r.rest()).read_to_end(&mut out).context("decompressing the manifest")?;
            if Sha1::from(&out).digest().bytes() != sha {
                bail!("the manifest is damaged (hash mismatch)");
            }
            out
        } else {
            r.rest().to_vec()
        };
        let mut b = Reader::new(&body);
        m.meta = Meta::read(&mut b)?;
        m.read_cdl(&mut b)?;
        m.read_fml(&mut b)?;
        m.read_custom_fields(&mut b)?;
        if m.is_encrypted() && m.meta.feature_level >= 24 {
            m.encrypted = Some(EncryptedData::read(&mut b)?);
        }
        if !b.rest().is_empty() {
            log::debug!("manifest: {} bytes left unread", b.rest().len());
        }
        Ok(m)
    }

    pub fn is_encrypted(&self) -> bool {
        self.stored_as & STORED_ENCRYPTED != 0
    }

    /// The file was zlib-compressed.
    pub fn is_compressed(&self) -> bool {
        self.stored_as & STORED_COMPRESSED != 0
    }

    fn read_cdl(&mut self, r: &mut Reader) -> Result<()> {
        let start = r.pos();
        let size = r.u32()? as usize;
        self.cdl_version = r.u8()?;
        let count = r.u32()? as usize;
        // Columns, not rows: every field of every chunk in turn.
        let mut chunks = vec![ChunkInfo::default(); count];
        for c in chunks.iter_mut() {
            c.guid = Guid(r.guid()?);
        }
        for c in chunks.iter_mut() {
            c.hash = r.u64()?;
        }
        for c in chunks.iter_mut() {
            c.sha_hash = r.array()?;
        }
        for c in chunks.iter_mut() {
            c.group_num = r.u8()?;
        }
        for c in chunks.iter_mut() {
            c.window_size = r.u32()?;
        }
        for c in chunks.iter_mut() {
            c.file_size = r.i64()?.max(0) as u64;
        }
        if self.meta.feature_level >= 22 {
            for c in chunks.iter_mut() {
                c.secret_guid = Guid(r.guid()?);
            }
            for c in chunks.iter_mut() {
                c.window_size_compressed = r.u32()?;
            }
            for c in chunks.iter_mut() {
                c.encryption_tag = r.array()?;
            }
        }
        if r.pos() - start != size {
            log::debug!("chunk list: read {} of {size} bytes", r.pos() - start);
            r.seek(start + size);
            self.cdl_version = 0;
        }
        self.chunks = chunks;
        Ok(())
    }

    fn read_fml(&mut self, r: &mut Reader) -> Result<()> {
        let start = r.pos();
        let size = r.u32()? as usize;
        self.fml_version = r.u8()?;
        let count = r.u32()? as usize;
        let mut files = vec![FileManifest::default(); count];
        for f in files.iter_mut() {
            f.filename = r.fstring()?;
        }
        for f in files.iter_mut() {
            f.symlink_target = r.fstring()?;
        }
        for f in files.iter_mut() {
            f.hash = r.array()?;
        }
        for f in files.iter_mut() {
            f.flags = r.u8()?;
        }
        for f in files.iter_mut() {
            let n = r.u32()?;
            for _ in 0..n {
                f.install_tags.push(r.fstring()?);
            }
        }
        for f in files.iter_mut() {
            let n = r.u32()? as usize;
            f.chunk_parts.reserve(n);
            for _ in 0..n {
                let part_start = r.pos();
                let part_size = r.u32()? as usize;
                let part = ChunkPart { guid: Guid(r.guid()?), offset: r.u32()?, size: r.u32()? };
                if part_size > r.pos() - part_start {
                    r.seek(part_start + part_size);
                }
                f.chunk_parts.push(part);
            }
        }
        if self.fml_version >= 1 {
            for f in files.iter_mut() {
                if r.u32()? != 0 {
                    f.hash_md5 = Some(r.array()?);
                }
            }
            for f in files.iter_mut() {
                f.mime_type = r.fstring()?;
            }
        }
        if self.fml_version >= 2 {
            for f in files.iter_mut() {
                f.hash_sha256 = Some(r.array()?);
            }
        }
        if r.pos() - start != size {
            log::debug!("file list: read {} of {size} bytes", r.pos() - start);
            r.seek(start + size);
            self.fml_version = 0;
        }
        self.files = files;
        Ok(())
    }

    fn read_custom_fields(&mut self, r: &mut Reader) -> Result<()> {
        let start = r.pos();
        let size = r.u32()? as usize;
        self.custom_fields_version = r.u8()?;
        let count = r.u32()? as usize;
        let keys = (0..count).map(|_| r.fstring()).collect::<Result<Vec<_>>>()?;
        let values = (0..count).map(|_| r.fstring()).collect::<Result<Vec<_>>>()?;
        self.custom_fields = keys.into_iter().zip(values).collect();
        if r.pos() - start != size {
            log::debug!("custom fields: read {} of {size} bytes", r.pos() - start);
            r.seek(start + size);
            self.custom_fields_version = 0;
        }
        Ok(())
    }

    /// Decrypts the program, prerequisites and file names of an encrypted
    /// manifest with the download's `secrets`. Nothing to do for a plain one.
    pub fn decrypt(&mut self, secrets: &Secrets) -> Result<()> {
        if !self.is_encrypted() {
            return Ok(());
        }
        let enc = self.encrypted.take().ok_or_else(|| anyhow!("the encrypted manifest has no encrypted data"))?;
        let plain = crypt::decrypt(secrets, &self.secret_guid.0, &enc.iv, &self.encryption_tag, &enc.ciphertext)?;
        let plain = if enc.header_stored_as & STORED_COMPRESSED != 0 {
            let mut out = Vec::with_capacity(enc.data_uncompressed as usize);
            flate2::read::ZlibDecoder::new(&plain[..]).read_to_end(&mut out).context("decompressing the manifest's encrypted data")?;
            out
        } else {
            plain
        };
        let mut r = Reader::new(&plain);
        self.meta.launch_exe = r.fstring()?;
        self.meta.launch_command = r.fstring()?;
        let n = r.u32()?;
        self.meta.prereq_ids = (0..n).map(|_| r.fstring()).collect::<Result<_>>()?;
        self.meta.prereq_name = r.fstring()?;
        self.meta.prereq_path = r.fstring()?;
        self.meta.prereq_args = r.fstring()?;
        self.meta.uninstall_action_path = r.fstring()?;
        self.meta.uninstall_action_args = r.fstring()?;
        for f in self.files.iter_mut() {
            f.filename = r.fstring()?;
            f.symlink_target = r.fstring()?;
        }
        self.stored_as &= !STORED_ENCRYPTED;
        self.secret_guid = Guid::default();
        let _ = (enc.version, enc.header_version, enc.data_compressed);
        Ok(())
    }

    /// The custom field `key`, if set.
    pub fn custom_field(&self, key: &str) -> Option<&str> {
        self.custom_fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set_custom_field(&mut self, key: &str, value: &str) {
        match self.custom_fields.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value.to_string(),
            None => self.custom_fields.push((key.to_string(), value.to_string())),
        }
    }

    pub fn chunk(&self, guid: &Guid) -> Option<&ChunkInfo> {
        self.chunks.iter().find(|c| c.guid == *guid)
    }

    pub fn file(&self, path: &str) -> Option<&FileManifest> {
        self.files.iter().find(|f| f.filename == path)
    }

    /// Bytes on disk of every file.
    pub fn disk_size(&self) -> u64 {
        self.files.iter().map(FileManifest::size).sum()
    }

    /// Bytes to download for every chunk.
    pub fn download_size(&self) -> u64 {
        self.chunks.iter().map(|c| c.file_size).sum()
    }

    /// Every install tag of the build, sorted, with `""` (untagged files).
    pub fn install_tags(&self) -> Vec<String> {
        let mut tags: std::collections::BTreeSet<String> = self.files.iter().flat_map(|f| f.install_tags.iter().cloned()).collect();
        tags.insert(String::new());
        tags.into_iter().collect()
    }

    /// The version to serialise with: the newest needed by what the manifest
    /// holds (legendary's `Manifest.write`), at most 24.
    fn target_version(&self) -> u32 {
        let mut v = DEFAULT_SERIALISATION.max(self.meta.feature_level);
        if self.meta.data_version == 2 {
            v = v.max(21);
        } else if self.fml_version == 2 {
            v = v.max(20);
        } else if self.fml_version == 1 {
            v = v.max(19);
        } else if self.meta.data_version == 1 {
            v = v.max(18);
        }
        v.min(24)
    }

    /// The binary form, zlib-compressed. An encrypted manifest is written
    /// decrypted ([`Manifest::decrypt`] it first).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.is_encrypted() {
            bail!("decrypt the manifest before writing it");
        }
        let version = self.target_version();
        let mut meta = self.meta.clone();
        meta.feature_level = version;
        let mut w = Writer::default();
        meta.write(&mut w);
        self.write_cdl(&mut w, version);
        self.write_fml(&mut w);
        self.write_custom_fields(&mut w);
        let body = w.buf;
        let sha = Sha1::from(&body).digest().bytes();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&body)?;
        let compressed = z.finish()?;

        let mut out = Writer::default();
        out.u32(MAGIC);
        let header_size = if version >= 22 { 73 } else { 41 };
        out.u32(header_size);
        out.u32(body.len() as u32);
        out.u32(compressed.len() as u32);
        out.bytes(&sha);
        out.u8(STORED_COMPRESSED);
        out.u32(version);
        if version >= 22 {
            out.guid(&[0; 4]);
            out.bytes(&[0; 16]);
        }
        out.bytes(&compressed);
        Ok(out.buf)
    }

    fn write_cdl(&self, w: &mut Writer, version: u32) {
        let start = w.len();
        w.u32(0);
        w.u8(self.cdl_version);
        w.u32(self.chunks.len() as u32);
        for c in &self.chunks {
            w.guid(&c.guid.0);
        }
        for c in &self.chunks {
            w.u64(c.hash);
        }
        for c in &self.chunks {
            w.bytes(&c.sha_hash);
        }
        for c in &self.chunks {
            w.u8(c.group_num);
        }
        for c in &self.chunks {
            w.u32(c.window_size);
        }
        for c in &self.chunks {
            w.i64(c.file_size as i64);
        }
        if version >= 22 {
            for c in &self.chunks {
                w.guid(&c.secret_guid.0);
            }
            for c in &self.chunks {
                w.u32(c.window_size_compressed);
            }
            for c in &self.chunks {
                w.bytes(&c.encryption_tag);
            }
        }
        let size = (w.len() - start) as u32;
        w.patch_u32(start, size);
    }

    fn write_fml(&self, w: &mut Writer) {
        let start = w.len();
        w.u32(0);
        w.u8(self.fml_version);
        w.u32(self.files.len() as u32);
        for f in &self.files {
            w.fstring(&f.filename);
        }
        for f in &self.files {
            w.fstring(&f.symlink_target);
        }
        for f in &self.files {
            w.bytes(&f.hash);
        }
        for f in &self.files {
            w.u8(f.flags);
        }
        for f in &self.files {
            w.u32(f.install_tags.len() as u32);
            for t in &f.install_tags {
                w.fstring(t);
            }
        }
        for f in &self.files {
            w.u32(f.chunk_parts.len() as u32);
            for p in &f.chunk_parts {
                // 4 size + 16 guid + 4 offset + 4 size.
                w.u32(28);
                w.guid(&p.guid.0);
                w.u32(p.offset);
                w.u32(p.size);
            }
        }
        if self.fml_version >= 1 {
            for f in &self.files {
                match &f.hash_md5 {
                    Some(md5) => {
                        w.u32(1);
                        w.bytes(md5);
                    }
                    None => w.u32(0),
                }
            }
            for f in &self.files {
                w.fstring(&f.mime_type);
            }
        }
        if self.fml_version >= 2 {
            for f in &self.files {
                w.bytes(&f.hash_sha256.unwrap_or([0; 32]));
            }
        }
        let size = (w.len() - start) as u32;
        w.patch_u32(start, size);
    }

    fn write_custom_fields(&self, w: &mut Writer) {
        let start = w.len();
        w.u32(0);
        w.u8(self.custom_fields_version);
        w.u32(self.custom_fields.len() as u32);
        for (k, _) in &self.custom_fields {
            w.fstring(k);
        }
        for (_, v) in &self.custom_fields {
            w.fstring(v);
        }
        let size = (w.len() - start) as u32;
        w.patch_u32(start, size);
    }

    /// A new, empty manifest (cloud saves).
    pub fn new(app_name: &str, build_version: &str) -> Self {
        Manifest {
            version: 18,
            meta: Meta {
                feature_level: 18,
                app_name: app_name.to_string(),
                build_version: build_version.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// The feature level chunk paths are computed with.
    pub fn chunk_level(&self) -> u32 {
        if self.is_json {
            self.version
        } else {
            self.meta.feature_level
        }
    }

    /// The path of `chunk` below a base URL.
    pub fn chunk_path(&self, chunk: &ChunkInfo) -> String {
        chunk.path(self.chunk_level())
    }

    /// Takes in an optimized delta manifest (`Deltas/<new>/<old>.delta`): its
    /// files replace those of the same path, its chunks join the list
    /// (legendary's `apply_delta_manifest`).
    pub fn apply_delta(&mut self, delta: &Manifest) {
        let by_path: std::collections::HashMap<&str, &FileManifest> = delta.files.iter().map(|f| (f.filename.as_str(), f)).collect();
        let mut taken = std::collections::HashSet::new();
        for f in self.files.iter_mut() {
            if let Some(d) = by_path.get(f.filename.as_str()) {
                *f = (*d).clone();
                taken.insert(f.filename.clone());
            }
        }
        self.files.extend(delta.files.iter().filter(|f| !taken.contains(&f.filename)).cloned());
        let known: std::collections::HashSet<Guid> = self.chunks.iter().map(|c| c.guid).collect();
        self.chunks.extend(delta.chunks.iter().filter(|c| !known.contains(&c.guid)).cloned());
    }

    /// The files an uninstall deletes: those of the installed tags (all, when
    /// none was picked).
    pub fn installed_files(&self, install_tags: &[String]) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.install_tags.is_empty() || f.install_tags.iter().any(|t| install_tags.contains(t)))
            .map(|f| f.filename.clone())
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small manifest with two files sharing a chunk, and a tagged file.
    pub(crate) fn sample() -> Manifest {
        let c1 = ChunkInfo {
            guid: Guid([1, 2, 3, 4]),
            hash: 0x1122334455667788,
            sha_hash: [9; 20],
            group_num: 42,
            window_size: 1 << 20,
            file_size: 1000,
            ..Default::default()
        };
        let c2 = ChunkInfo {
            guid: Guid([5, 6, 7, 8]),
            hash: 7,
            sha_hash: [8; 20],
            group_num: 3,
            window_size: 1 << 20,
            file_size: 500,
            ..Default::default()
        };
        let mut m = Manifest::new("Calluna", "1.33");
        m.meta.launch_exe = "Control.exe".into();
        m.meta.launch_command = "-dx12".into();
        m.meta.prereq_ids = vec!["vcredist".into()];
        m.meta.prereq_path = "redist/vc.exe".into();
        m.meta.data_version = 2;
        m.meta.uninstall_action_path = "uninstall.exe".into();
        m.fml_version = 2;
        m.chunks = vec![c1.clone(), c2.clone()];
        m.files = vec![
            FileManifest {
                filename: "Control.exe".into(),
                hash: [1; 20],
                flags: 0x4,
                chunk_parts: vec![ChunkPart { guid: c1.guid, offset: 0, size: 100 }],
                hash_md5: Some([2; 16]),
                hash_sha256: Some([3; 32]),
                ..Default::default()
            },
            FileManifest {
                filename: "data/a.pak".into(),
                hash: [4; 20],
                chunk_parts: vec![ChunkPart { guid: c1.guid, offset: 100, size: 50 }, ChunkPart { guid: c2.guid, offset: 0, size: 70 }],
                hash_sha256: Some([5; 32]),
                ..Default::default()
            },
            FileManifest {
                filename: "lang/de.pak".into(),
                hash: [6; 20],
                install_tags: vec!["de".into()],
                chunk_parts: vec![ChunkPart { guid: c2.guid, offset: 70, size: 30 }],
                mime_type: "application/octet-stream".into(),
                hash_sha256: Some([7; 32]),
                ..Default::default()
            },
        ];
        m.custom_fields = vec![("CloudSaveFolder".into(), "{AppData}/Remedy".into())];
        m
    }

    #[test]
    fn binary_manifests_round_trip() {
        let m = sample();
        let bytes = m.to_bytes().unwrap();
        let back = Manifest::parse(&bytes).unwrap();
        assert_eq!(back.meta.app_name, "Calluna");
        assert_eq!(back.meta.build_version, "1.33");
        assert_eq!(back.meta.launch_exe, "Control.exe");
        assert_eq!(back.meta.prereq_ids, ["vcredist"]);
        assert_eq!(back.meta.uninstall_action_path, "uninstall.exe");
        assert_eq!(back.meta.feature_level, 21, "data version 2 needs version 21");
        assert_eq!(back.chunks, m.chunks);
        assert_eq!(back.files, m.files);
        assert_eq!(back.custom_field("CloudSaveFolder"), Some("{AppData}/Remedy"));
        assert_eq!(back.meta.build_id(), m.meta.build_id());
        // Writing what was read gives the same bytes.
        assert_eq!(back.to_bytes().unwrap(), bytes);
        assert!(Manifest::parse(b"nope").is_err());
    }

    #[test]
    fn sizes_and_tags() {
        let m = sample();
        assert_eq!(m.disk_size(), 100 + 120 + 30);
        assert_eq!(m.download_size(), 1500);
        assert_eq!(m.install_tags(), ["", "de"]);
        assert!(m.files[0].executable());
        assert!(m.files[0].selected_by(&["".into()]));
        assert!(!m.files[2].selected_by(&["".into()]));
        assert!(m.files[2].selected_by(&["de".into()]));
    }

    #[test]
    fn chunk_paths_follow_the_feature_level() {
        let c = &sample().chunks[0];
        assert_eq!(c.path(18), "ChunksV4/42/1122334455667788_00000001000000020000000300000004.chunk");
        assert_eq!(c.path(3), "ChunksV2/42/1122334455667788_00000001000000020000000300000004.chunk");
        assert_eq!(c.path(13), "ChunksV3/42/1122334455667788_00000001000000020000000300000004.chunk");
        assert_eq!(c.path(22), "ChunksV5/plain/42/iHdmVUQzIhE_AQAAAAIAAAADAAAABAAAAA.chunk");
        let mut secret = c.clone();
        secret.secret_guid = Guid([1, 0, 0, 0]);
        assert!(secret.path(23).starts_with("ChunksV5/AQAAAAAAAAAAAAAAAAAAAA/42/"));
    }

    #[test]
    fn computed_build_ids_match_legendary() {
        // legendary: sha1(pack('<I', 0) + b'Calluna' + b'1.33' + b'Control.exe' + b'-dx12'), urlsafe b64 without '='.
        let mut meta = Meta {
            app_name: "Calluna".into(),
            build_version: "1.33".into(),
            launch_exe: "Control.exe".into(),
            launch_command: "-dx12".into(),
            ..Default::default()
        };
        let mut s = Sha1::new();
        s.update(&[0, 0, 0, 0]);
        s.update(b"Calluna1.33Control.exe-dx12");
        assert_eq!(meta.build_id(), URL_SAFE_NO_PAD.encode(s.digest().bytes()));
        meta.stored_build_id = "stored".into();
        assert_eq!(meta.build_id(), "stored");
    }

    #[test]
    fn delta_manifests_replace_files_and_add_chunks() {
        let mut m = sample();
        let mut delta = Manifest::new("Calluna", "1.34");
        let mut changed = m.files[1].clone();
        changed.hash = [0xEE; 20];
        delta.files = vec![changed.clone(), FileManifest { filename: "new.txt".into(), ..Default::default() }];
        delta.chunks = vec![m.chunks[0].clone(), ChunkInfo { guid: Guid([7, 7, 7, 7]), ..Default::default() }];
        m.apply_delta(&delta);
        assert_eq!(m.files.len(), 4);
        assert_eq!(m.files[1], changed);
        assert_eq!(m.chunks.len(), 3);
        assert_eq!(m.installed_files(&[]), ["Control.exe", "data/a.pak", "new.txt"]);
        assert_eq!(m.installed_files(&["de".into()]).len(), 4);
    }

    #[test]
    fn guids() {
        let g = Guid([0xA, 0xB, 0xC, 0xD]);
        assert_eq!(g.text(), "0000000a-0000000b-0000000c-0000000d");
        assert_eq!(g.num(), 0xA << 96 | 0xB << 64 | 0xC << 32 | 0xD);
        assert!(Guid::random() != Guid::random());
        assert!(g.group_num() < 100);
    }

    /// An encrypted manifest of feature level 24, made like Epic's.
    #[test]
    fn encrypted_manifests_are_decrypted_with_their_secret() {
        let key = [3u8; 32];
        let iv = [4u8; 12];
        let secret = Guid([0x11, 0x22, 0x33, 0x44]);
        let mut plain = Writer::default();
        plain.fstring("Game.exe");
        plain.fstring("-epic");
        plain.u32(0);
        for _ in 0..5 {
            plain.fstring("");
        }
        plain.fstring("bin/Game.exe");
        plain.fstring("");
        let (cipher, tag) = crypt::encrypt(&key, &iv, &plain.buf);

        let mut m = Manifest::new("App", "1");
        m.files = vec![FileManifest { filename: String::new(), ..Default::default() }];
        m.meta.feature_level = 24;
        let mut body = Writer::default();
        m.meta.write(&mut body);
        m.write_cdl(&mut body, 24);
        m.write_fml(&mut body);
        m.write_custom_fields(&mut body);
        // The encrypted data block.
        let mut inner = Writer::default();
        inner.u32(4 + 4 + 1 + 4 + 4 + 4 + 12);
        inner.u32(1);
        inner.u8(0);
        inner.u32(plain.buf.len() as u32);
        inner.u32(plain.buf.len() as u32);
        inner.u32(12);
        inner.bytes(&iv);
        inner.u32(cipher.len() as u32);
        inner.bytes(&cipher);
        let block_start = body.len();
        body.u32(0);
        body.u8(0);
        body.u32(inner.buf.len() as u32);
        body.bytes(&inner.buf);
        let size = (body.len() - block_start) as u32;
        body.patch_u32(block_start, size);

        let mut out = Writer::default();
        out.u32(MAGIC);
        out.u32(73);
        out.u32(body.buf.len() as u32);
        out.u32(body.buf.len() as u32);
        out.bytes(&[0; 20]);
        out.u8(STORED_ENCRYPTED);
        out.u32(24);
        out.guid(&secret.0);
        out.bytes(&tag);
        out.bytes(&body.buf);

        let mut parsed = Manifest::parse(&out.buf).unwrap();
        assert!(parsed.is_encrypted());
        assert!(parsed.to_bytes().is_err());
        assert!(parsed.clone().decrypt(&Secrets::new()).is_err());
        let mut secrets = Secrets::new();
        secrets.insert(crypt::secret_name(&secret.0), hex::encode(key));
        parsed.decrypt(&secrets).unwrap();
        assert_eq!(parsed.meta.launch_exe, "Game.exe");
        assert_eq!(parsed.meta.launch_command, "-epic");
        assert_eq!(parsed.files[0].filename, "bin/Game.exe");
        assert!(!parsed.is_encrypted());
        assert!(Manifest::parse(&parsed.to_bytes().unwrap()).is_ok());
    }
}
