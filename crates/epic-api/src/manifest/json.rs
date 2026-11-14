//! The JSON manifests of old builds (legendary's `JSONManifest`). Numbers are
//! "blobs": three decimal digits per byte, little-endian.

use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value};

use super::{ChunkInfo, ChunkPart, FileManifest, Guid, Manifest, Meta};

/// A blob number: `"013000000000"` is 13.
fn blob_to_num(text: &str) -> u128 {
    let mut num = 0u128;
    for (i, part) in text.as_bytes().chunks(3).enumerate() {
        let byte: u128 = std::str::from_utf8(part).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        if i < 16 {
            num += byte << (8 * i);
        }
    }
    num
}

fn blob(v: Option<&Value>) -> u128 {
    v.and_then(Value::as_str).map(blob_to_num).unwrap_or(0)
}

/// A GUID written as 32 hex digits, big-endian per number.
fn guid(text: &str) -> Result<Guid> {
    let bytes = hex::decode(text).map_err(|e| anyhow!("invalid GUID {text}: {e}"))?;
    if bytes.len() != 16 {
        return Err(anyhow!("invalid GUID {text}"));
    }
    let n = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Ok(Guid([n(0), n(4), n(8), n(12)]))
}

fn text(doc: &Map<String, Value>, key: &str) -> String {
    doc.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub(super) fn parse(data: &[u8]) -> Result<Manifest> {
    let doc: Value = serde_json::from_slice(data).context("reading the JSON manifest")?;
    let doc = doc.as_object().ok_or_else(|| anyhow!("the JSON manifest is not an object"))?;
    let version = doc.get("ManifestFileVersion").map(|v| blob(Some(v))).unwrap_or(13) as u32;
    let meta = Meta {
        feature_level: version,
        is_file_data: doc.get("bIsFileData").and_then(Value::as_bool).unwrap_or(false),
        app_id: blob(doc.get("AppID")) as u32,
        app_name: text(doc, "AppNameString"),
        build_version: text(doc, "BuildVersionString"),
        launch_exe: text(doc, "LaunchExeString"),
        launch_command: text(doc, "LaunchCommand"),
        prereq_ids: doc
            .get("PrereqIds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        prereq_name: text(doc, "PrereqName"),
        prereq_path: text(doc, "PrereqPath"),
        prereq_args: text(doc, "PrereqArgs"),
        ..Default::default()
    };

    let empty = Map::new();
    let section = |key: &str| doc.get(key).and_then(Value::as_object).unwrap_or(&empty);
    let (sizes, hashes, shas, groups) =
        (section("ChunkFilesizeList"), section("ChunkHashList"), section("ChunkShaList"), section("DataGroupList"));
    let mut chunks = Vec::with_capacity(sizes.len());
    for (id, size) in sizes {
        let mut sha = [0u8; 20];
        if let Some(h) = shas.get(id).and_then(Value::as_str).and_then(|h| hex::decode(h).ok()).filter(|b| b.len() == 20) {
            sha.copy_from_slice(&h);
        }
        chunks.push(ChunkInfo {
            guid: guid(id)?,
            file_size: blob(Some(size)) as u64,
            hash: blob(hashes.get(id)) as u64,
            sha_hash: sha,
            group_num: blob(groups.get(id)) as u8,
            window_size: 1024 * 1024,
            ..Default::default()
        });
    }

    let mut files = Vec::new();
    for f in doc.get("FileManifestList").and_then(Value::as_array).into_iter().flatten() {
        let Some(f) = f.as_object() else { continue };
        let flag = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(false) as u8;
        // A 160-bit blob, too wide for a number: each digit triple is one byte.
        let mut hash = [0u8; 20];
        let hash_text = f.get("FileHash").and_then(Value::as_str).unwrap_or_default();
        for (byte, part) in hash.iter_mut().zip(hash_text.as_bytes().chunks(3)) {
            *byte = std::str::from_utf8(part).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
        let mut parts = Vec::new();
        for p in f.get("FileChunkParts").and_then(Value::as_array).into_iter().flatten() {
            parts.push(ChunkPart {
                guid: guid(p.get("Guid").and_then(Value::as_str).unwrap_or_default())?,
                offset: blob(p.get("Offset")) as u32,
                size: blob(p.get("Size")) as u32,
            });
        }
        files.push(FileManifest {
            filename: text(f, "Filename"),
            hash,
            flags: flag("bIsReadOnly") | flag("bIsCompressed") << 1 | flag("bIsUnixExecutable") << 2,
            install_tags: f
                .get("InstallTags")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            chunk_parts: parts,
            ..Default::default()
        });
    }
    let custom_fields = section("CustomFields").iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect();
    Ok(Manifest { version, meta, chunks, files, custom_fields, is_json: true, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_are_little_endian_digit_triples() {
        assert_eq!(blob_to_num("013000000000"), 13);
        assert_eq!(blob_to_num("000001"), 256);
        assert_eq!(blob_to_num(""), 0);
    }

    #[test]
    fn reads_an_old_json_manifest() {
        let doc = r#"{
            "ManifestFileVersion": "013000000000", "bIsFileData": false, "AppID": "000000000000",
            "AppNameString": "Old", "BuildVersionString": "1.0", "LaunchExeString": "Old.exe", "LaunchCommand": "",
            "PrereqIds": [], "PrereqName": "", "PrereqPath": "", "PrereqArgs": "",
            "FileManifestList": [{"Filename": "Old.exe", "FileHash": "001002003004005006007008009010011012013014015016017018019020",
                "bIsUnixExecutable": true, "FileChunkParts": [{"Guid": "0000000100000002000000030000000A", "Offset": "000000000000", "Size": "100000000000"}]}],
            "ChunkHashList": {"0000000100000002000000030000000A": "001000000000000000"},
            "ChunkShaList": {"0000000100000002000000030000000A": "0909090909090909090909090909090909090909"},
            "DataGroupList": {"0000000100000002000000030000000A": "007"},
            "ChunkFilesizeList": {"0000000100000002000000030000000A": "232003000000"},
            "CustomFields": {"CloudSaveFolder": "{AppData}/Old"}
        }"#;
        let m = Manifest::parse(doc.as_bytes()).unwrap();
        assert!(m.is_json);
        assert_eq!(m.chunk_level(), 13);
        assert_eq!(m.meta.app_name, "Old");
        assert_eq!(m.files[0].hash, core::array::from_fn::<u8, 20, _>(|i| i as u8 + 1));
        assert!(m.files[0].executable());
        assert_eq!(m.files[0].size(), 100);
        assert_eq!(m.chunks[0].guid, Guid([1, 2, 3, 10]));
        assert_eq!(m.chunks[0].file_size, 1000);
        assert_eq!(m.chunks[0].hash, 1);
        assert_eq!(m.chunks[0].group_num, 7);
        assert_eq!(m.chunk_path(&m.chunks[0]), "ChunksV3/07/0000000000000001_0000000100000002000000030000000A.chunk");
        assert_eq!(m.custom_field("CloudSaveFolder"), Some("{AppData}/Old"));
    }
}
