//! A list on this computer (a debug build's own list, or one
//! `HERONITE_EXTENSIONS_INDEX` names): read and checked like a downloaded
//! one without the network, its entries' files read beside it and checked
//! against it, nothing outside its folder read, and the repository's own
//! `extensions/metadata/index.json` installing every example.

mod support;

use std::path::{Path, PathBuf};

use heronite_data::paths::DataPaths;
use heronite_metadata::{Endpoints, MetadataService};
use heronite_metadata_ext::index::{self, IndexError};
use heronite_metadata_ext::lists::{self, ListError, ListLocation, Lists, Origin};
use heronite_metadata_ext::{ExtensionError, ExtensionsExt, InstallOutcome};
use support::example;

fn repo_index() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/metadata/index.json").canonicalize().unwrap()
}

/// A service whose network answers nothing: the entries' `url`s (on
/// GitHub) must never be needed.
fn offline(data: &Path) -> MetadataService {
    let e = Endpoints { origins: vec![("https://raw.githubusercontent.com".into(), "http://127.0.0.1:9".into())], ..Default::default() };
    MetadataService::with_endpoints(DataPaths::at(data), e)
}

/// A copy of the repository's `extensions/metadata/` (the list and its
/// folders) in `dir`; returns the list's path.
fn copy_of_the_repository_list(dir: &Path) -> PathBuf {
    for id in ["wikipedia", "vndb"] {
        std::fs::create_dir_all(dir.join(id)).unwrap();
        for file in ["source.toml", "source.js"] {
            std::fs::copy(example(id).join(file), dir.join(id).join(file)).unwrap();
        }
    }
    std::fs::copy(repo_index(), dir.join("index.json")).unwrap();
    dir.join("index.json")
}

#[test]
fn a_list_is_written_as_an_address_or_a_path() {
    let at = |t: &str| ListLocation::of(t);
    assert_eq!(at("https://lists.example.org/index.json"), ListLocation::Address("https://lists.example.org/index.json".into()));
    assert_eq!(at("http://lists.example.org/index.json"), ListLocation::Address("http://lists.example.org/index.json".into()));
    assert_eq!(
        at("/home/me/heronite/extensions/metadata/index.json"),
        ListLocation::File("/home/me/heronite/extensions/metadata/index.json".into())
    );
    // A file address is an absolute path of this system: a drive on Windows.
    #[cfg(unix)]
    assert_eq!(at(" file:///home/me/list/index.json "), ListLocation::File("/home/me/list/index.json".into()));
    #[cfg(windows)]
    assert_eq!(at(" file:///C:/me/list/index.json "), ListLocation::File(r"C:\me\list\index.json".into()));
    let file = ListLocation::File("/home/me/list/index.json".into());
    assert_eq!(ListLocation::of(&file.key()), file, "the key reads back as the same list");
    assert!(lists::is_local("/x/index.json") && !lists::is_local(lists::HERONITE_LIST));
}

#[test]
fn a_list_on_this_computer_is_read_and_checked_without_the_network_or_a_copy() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let list = copy_of_the_repository_list(folder.path());
    let key = ListLocation::File(list.clone()).key();
    let svc = offline(data.path());
    let own = Lists::new(key.clone());

    let copies = own.refresh(&svc);
    assert_eq!(copies.len(), 1);
    let copy = &copies[0];
    assert!(copy.builtin && copy.error.is_none() && copy.read_at.is_some(), "{copy:?}");
    assert_eq!(copy.url, key);
    let ids: Vec<&str> = copy.index.as_ref().unwrap().entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["vndb", "wikipedia"]);
    // Read now when the copies kept are asked for too, and no copy is kept.
    assert_eq!(own.cached(&svc)[0].index, copy.index);
    let kept = std::fs::read_dir(svc.paths.extension_lists_cache_dir()).map(|d| d.count()).unwrap_or(0);
    assert_eq!(kept, 0, "nothing is kept of a list on this computer");

    // As strict as a downloaded list: one wrong entry refuses it all.
    let text = std::fs::read_to_string(&list).unwrap();
    std::fs::write(&list, text.replacen("https://raw.githubusercontent.com", "http://raw.githubusercontent.com", 1)).unwrap();
    let copy = own.read(&svc, &key);
    assert!(copy.index.is_none() && matches!(copy.error, Some(ListError::Invalid(IndexError::Invalid(_)))), "{copy:?}");
    std::fs::write(&list, r#"{"index": 2, "extensions": []}"#).unwrap();
    assert_eq!(lists::read_list_file(&list).unwrap_err(), ListError::Invalid(IndexError::NewerFormat(2)));
    std::fs::write(&list, r#"{"index": 1, "type": "themes", "extensions": []}"#).unwrap();
    assert_eq!(lists::read_list_file(&list).unwrap_err(), ListError::Invalid(IndexError::WrongType("themes".into())));
    std::fs::write(&list, r#"{"index": 1, "extensions": []}"#).unwrap();
    assert!(lists::read_list_file(&list).is_ok(), "a hand-written list without a type is a list of metadata extensions");
    std::fs::write(&list, vec![b' '; index::MAX_INDEX_BYTES as usize + 1]).unwrap();
    assert_eq!(lists::read_list_file(&list).unwrap_err(), ListError::TooLarge);

    // Gone: nothing to show, and why.
    std::fs::remove_file(&list).unwrap();
    let copy = own.read(&svc, &key);
    assert!(copy.index.is_none() && copy.read_at.is_none() && matches!(copy.error, Some(ListError::NoFile(_))), "{copy:?}");
    // Not a list one can add: only addresses are.
    assert_eq!(Lists::default().add(&svc, &key), Err(ListError::NotHttps));
}

#[test]
fn an_entry_is_read_beside_the_list_and_checked_against_it() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let list = copy_of_the_repository_list(folder.path());
    let key = ListLocation::File(list).key();
    let svc = offline(data.path());
    let own = Lists::new(key.clone());
    let entry = || own.read(&svc, &key).index.unwrap().get("wikipedia").cloned().unwrap();

    // A source.js that is not the one the list names: refused.
    let js = folder.path().join("wikipedia/source.js");
    let original = std::fs::read(&js).unwrap();
    let mut tampered = original.clone();
    tampered.extend(b"\n// more\n");
    std::fs::write(&js, &tampered).unwrap();
    assert_eq!(lists::prepare(&svc, &key, &entry()).unwrap_err(), ExtensionError::Checksum("source.js".into()));
    // The same size, other bytes.
    let mut same_size = original.clone();
    let last = same_size.len() - 2;
    same_size[last] ^= 1;
    std::fs::write(&js, &same_size).unwrap();
    assert_eq!(lists::prepare(&svc, &key, &entry()).unwrap_err(), ExtensionError::Checksum("source.js".into()));
    // No file.
    std::fs::remove_file(&js).unwrap();
    assert_eq!(lists::prepare(&svc, &key, &entry()).unwrap_err(), ExtensionError::MissingFile("source.js".into()));
    assert!(svc.installed_extensions().is_empty());

    // The files as listed: installed as from a downloaded list, the list
    // recorded as its path. The entry's `url`s were never asked for (the
    // network answers nothing).
    std::fs::write(&js, &original).unwrap();
    let prepared = lists::prepare(&svc, &key, &entry()).unwrap();
    assert_eq!((prepared.info.id.as_str(), prepared.info.version.as_str()), ("wikipedia", "1.0.0"));
    let InstallOutcome::Installed(ext) = lists::install_prepared(&svc, &prepared, false).unwrap() else { panic!() };
    assert_eq!(ext.list.as_deref(), Some(key.as_str()));
    assert_eq!(PathBuf::from(&ext.from), folder.path().join("wikipedia/source.toml"));
    assert_eq!(own.origin(&svc, &ext), Origin::Local(key.clone()), "never \"From the Heronite list\"");
    // Still from this computer once the app's own list is another one.
    assert_eq!(Lists::default().origin(&svc, &ext), Origin::Local(key));
}

#[test]
fn nothing_outside_the_lists_folder_is_read() {
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let list = copy_of_the_repository_list(&folder.path().join("list"));
    let key = ListLocation::File(list).key();
    let svc = offline(data.path());
    let entry = Lists::new(key.clone()).read(&svc, &key).index.unwrap().get("wikipedia").cloned().unwrap();

    // An id that would leave the folder (the parser refuses it; a caller
    // could still build one).
    for id in ["../wikipedia", "/wikipedia", "wikipedia/..", ".."] {
        let wrong = index::IndexEntry { id: id.into(), ..entry.clone() };
        assert!(matches!(lists::prepare(&svc, &key, &wrong), Err(ExtensionError::UnsafePath(_))), "{id}");
    }

    // A link to a file outside, even the very file the list names.
    let outside = folder.path().join("source.js");
    let js = folder.path().join("list/wikipedia/source.js");
    std::fs::rename(&js, &outside).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, &js).unwrap();
        assert!(matches!(lists::prepare(&svc, &key, &entry), Err(ExtensionError::UnsafePath(_))));
    }
    assert!(svc.installed_extensions().is_empty());
}

#[test]
fn the_repository_list_read_from_this_checkout_installs_every_example() {
    let data = tempfile::tempdir().unwrap();
    let svc = offline(data.path());
    let key = ListLocation::File(repo_index()).key();
    let own = Lists::new(key.clone());
    let copy = own.refresh(&svc).remove(0);
    assert!(copy.error.is_none(), "{:?}", copy.error);
    for entry in &copy.index.unwrap().entries {
        let prepared = lists::prepare(&svc, &key, entry).unwrap_or_else(|e| panic!("{}: {e}", entry.id));
        let out = lists::install_prepared(&svc, &prepared, false).unwrap();
        assert!(matches!(out, InstallOutcome::Installed(_)), "{}: {out:?}", entry.id);
    }
    let installed = svc.installed_extensions();
    let ids: Vec<&str> = installed.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["vndb", "wikipedia"]);
    assert!(installed.iter().all(|e| e.list.as_deref() == Some(key.as_str()) && own.origin(&svc, e) == Origin::Local(key.clone())));
    assert!(svc.sources().get("wikipedia").is_some() && svc.sources().get("vndb").is_some(), "registered and asked");
    // Nothing updates while the list offers the versions installed.
    assert!(lists::updates(&own.refresh(&svc), &installed).is_empty());
}
