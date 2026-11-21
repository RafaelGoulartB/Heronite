//! A manifest legendary stored for a real install (a DLC with one file),
//! read like legendary reads it.

use heronite_epic_api::manifest::Manifest;

#[test]
fn a_manifest_stored_by_legendary_reads_the_same() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/small.manifest")).unwrap();
    let m = Manifest::parse(&bytes).unwrap();
    // What legendary 0.21 reads from this file.
    assert_eq!(m.meta.app_name, "KingletAztec");
    assert_eq!(m.meta.build_version, "1.0.12.951533_nebraska_update1_KingletAztec");
    assert_eq!(m.meta.build_id(), "Qm4WtUpzqZebQnH-pySO8HBUYcE");
    assert_eq!(m.meta.feature_level, 17);
    assert_eq!(m.files.len(), 1);
    let f = &m.files[0];
    assert_eq!(f.filename, "DLC/Aztec_Montezuma/AztecOwnership.modinfo");
    assert_eq!(hex::encode(f.hash), "ec5260d086ad7ce05c79765830dd4b4ad7d3a1fc");
    assert_eq!(f.size(), 233);
    assert_eq!(f.chunk_parts[0].guid.text(), "4d5d560f-4428bc5e-5a135cb6-14ddd4bb");
    let c = &m.chunks[0];
    assert_eq!(c.hash, 0xd43705cc3add444c);
    assert_eq!(hex::encode(c.sha_hash), "330c9121e68cdc94c1a334fa0be51b5b4214bc34");
    assert_eq!((c.group_num, c.window_size, c.file_size), (69, 1048576, 1287));
    assert_eq!(m.chunk_path(c), "ChunksV4/69/D43705CC3ADD444C_4D5D560F4428BC5E5A135CB614DDD4BB.chunk");
    assert_eq!((m.disk_size(), m.download_size()), (233, 1287));
    // Written again, the body is byte for byte legendary's.
    assert_eq!(body(&m.to_bytes().unwrap()), body(&bytes));
}

/// The decompressed body of a binary manifest (after its 41-byte header).
fn body(file: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&file[41..]), &mut out).unwrap();
    out
}
