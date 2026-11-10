use std::io::Write;

use zip::write::SimpleFileOptions;

use super::*;

const MANIFEST_TEXT: &str = r#"id = "tiny"
name = "Tiny"
version = "1.0.0"
api = 1
author = "me"
kind = "details"
description = "d"
[provides]
fields = ["description"]
"#;
const SCRIPT_TEXT: &str = "export function find() { return []; }\nexport function fetch() { return null; }\n";

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        w.start_file(*name, SimpleFileOptions::default()).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn good() -> Vec<(&'static str, &'static [u8])> {
    vec![(MANIFEST, MANIFEST_TEXT.as_bytes()), (SCRIPT, SCRIPT_TEXT.as_bytes())]
}

#[test]
fn a_zip_with_the_files_at_the_top_or_in_one_folder() {
    let bytes = zip_of(&good());
    let pkg = read_zip(&bytes).unwrap();
    assert_eq!(pkg.manifest.id, "tiny");
    assert_eq!(pkg.sha256, hex(&Sha256::digest(&bytes)), "the zip file's own digest");
    assert_eq!(pkg.sha256.len(), 64);
    let nested = zip_of(&[
        ("tiny/source.toml", MANIFEST_TEXT.as_bytes()),
        ("tiny/source.js", SCRIPT_TEXT.as_bytes()),
        ("tiny/README.md", b"# Tiny"),
        ("tiny/icon.svg", b"<svg/>"),
        ("__MACOSX/tiny/._source.js", b"junk"),
        ("tiny/.DS_Store", b"junk"),
    ]);
    let pkg = read_zip(&nested).unwrap();
    assert_eq!(pkg.readme.as_deref(), Some("# Tiny"));
    assert_eq!(pkg.icon.as_ref().map(|(n, _)| n.as_str()), Some("icon.svg"));
    assert_eq!(pkg.files().len(), 4);
}

#[test]
fn paths_that_leave_the_package_are_refused() {
    for evil in ["../source.js", "/etc/source.js", "tiny/../../source.js", "C:/source.js", "tiny\\..\\source.js", "./source.js"] {
        let mut entries = good();
        entries.push((evil, b"x"));
        assert!(matches!(read_zip(&zip_of(&entries)), Err(ExtensionError::UnsafePath(_))), "{evil}");
    }
}

#[test]
fn only_the_expected_files_are_accepted() {
    for (extra, why) in [("evil.so", "another file"), ("lib/helper.js", "a folder"), ("icon.jpg", "another icon")] {
        let mut entries = good();
        entries.push((extra, b"x"));
        assert!(matches!(read_zip(&zip_of(&entries)), Err(ExtensionError::UnexpectedFile(_))), "{why}");
    }
    let both_icons = [good(), vec![("icon.png", b"x".as_slice()), ("icon.svg", b"<svg/>".as_slice())]].concat();
    assert!(matches!(read_zip(&zip_of(&both_icons)), Err(ExtensionError::UnexpectedFile(_))));
    assert_eq!(read_zip(&zip_of(&[(MANIFEST, MANIFEST_TEXT.as_bytes())])).unwrap_err(), ExtensionError::MissingFile(SCRIPT.into()));
    assert_eq!(read_zip(&zip_of(&[(SCRIPT, SCRIPT_TEXT.as_bytes())])).unwrap_err(), ExtensionError::MissingFile(MANIFEST.into()));
    assert!(matches!(read_zip(b"not a zip"), Err(ExtensionError::NotAPackage(_))));
    let not_utf8 = [(MANIFEST, MANIFEST_TEXT.as_bytes()), (SCRIPT, b"\xff\xfe".as_slice())];
    assert!(matches!(read_zip(&zip_of(&not_utf8)), Err(ExtensionError::NotAPackage(_))));
}

#[test]
fn links_in_a_zip_are_refused() {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in good() {
        w.start_file(name, SimpleFileOptions::default()).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.add_symlink("README.md", "/etc/passwd", SimpleFileOptions::default()).unwrap();
    let bytes = w.finish().unwrap().into_inner();
    assert!(matches!(read_zip(&bytes), Err(ExtensionError::UnexpectedFile(m)) if m.contains("link")));
}

#[test]
fn sizes_are_capped() {
    let big_script = vec![b' '; 512 * 1024 + 1];
    let entries = [(MANIFEST, MANIFEST_TEXT.as_bytes()), (SCRIPT, big_script.as_slice())];
    assert!(matches!(read_zip(&zip_of(&entries)), Err(ExtensionError::TooLarge(m)) if m.contains("source.js")));
    // A small zip that unpacks to a lot (a zip bomb) stops at the cap.
    let bomb = vec![b'a'; 64 * 1024 * 1024];
    let entries = [(MANIFEST, MANIFEST_TEXT.as_bytes()), (SCRIPT, bomb.as_slice())];
    let bytes = zip_of(&entries);
    assert!(bytes.len() < 1024 * 1024);
    assert!(matches!(read_zip(&bytes), Err(ExtensionError::TooLarge(_))));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.zip");
    std::fs::write(&path, vec![0u8; MAX_ARCHIVE as usize + 1]).unwrap();
    assert!(matches!(read(&path), Err(ExtensionError::TooLarge(_))));
}

#[test]
fn a_folder_is_read_like_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MANIFEST), MANIFEST_TEXT).unwrap();
    std::fs::write(dir.path().join(SCRIPT), SCRIPT_TEXT).unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let pkg = read(dir.path()).unwrap();
    assert_eq!(pkg.manifest.name, "Tiny");
    // The digest of the content is the same whatever the folder is called.
    let other = tempfile::tempdir().unwrap();
    for (name, bytes) in pkg.files() {
        std::fs::write(other.path().join(name), bytes).unwrap();
    }
    assert_eq!(read(other.path()).unwrap().sha256, pkg.sha256);
    std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
    assert!(matches!(read(dir.path()), Err(ExtensionError::UnexpectedFile(_))));
    std::fs::remove_file(dir.path().join("notes.txt")).unwrap();
    std::fs::create_dir(dir.path().join("lib")).unwrap();
    assert!(matches!(read(dir.path()), Err(ExtensionError::UnexpectedFile(m)) if m.contains("folder")));
    std::fs::remove_dir(dir.path().join("lib")).unwrap();
    std::fs::write(dir.path().join(RECORD), "{}").unwrap();
    assert!(matches!(read(dir.path()), Err(ExtensionError::UnexpectedFile(_))), "install.json is the host's");
    assert!(read_installed(dir.path()).is_ok());
    assert!(matches!(read(&dir.path().join("missing")), Err(ExtensionError::NotAPackage(_))));
}

#[cfg(unix)]
#[test]
fn links_in_a_folder_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MANIFEST), MANIFEST_TEXT).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.path().join(SCRIPT)).unwrap();
    assert!(matches!(read(dir.path()), Err(ExtensionError::UnexpectedFile(m)) if m.contains("link")));
}
