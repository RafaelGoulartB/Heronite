//! How the extension screens word things: every error of the engine and of
//! the lists as a sentence (the engine's text stays as a detail for the
//! author), where a source came from and what that guarantees, and how old
//! a list's copy is.
//!
//! The trust words say only what the code checks: an entry of a list is
//! installed when its files are the ones the list names (SHA-256); nobody
//! reviews a file the user picks, and Heronite knows nothing of a list the
//! user added.

use std::time::SystemTime;

use heronite_i18n::{tr, trn};
use heronite_metadata::SourceKind;
use heronite_metadata_ext::index::IndexError;
use heronite_metadata_ext::lists::{host_of, is_local, ListError, Origin, HERONITE_LIST};
use heronite_metadata_ext::ExtensionError;

/// What went wrong with a package, in a sentence.
pub fn error(e: &ExtensionError) -> String {
    match e {
        ExtensionError::NotAPackage(_) => tr!("This is not an extension. Choose its .zip, or the source.toml in its folder.").into(),
        ExtensionError::TooLarge(_) => {
            tr!("The extension is larger than an extension may be (2 MB for the zip, 512 KB for its script).").into()
        }
        ExtensionError::UnexpectedFile(_) => {
            tr!("The package holds a file an extension may not have. Only source.toml, source.js, README.md and an icon may be in it.")
                .into()
        }
        ExtensionError::UnsafePath(_) => tr!("A file of the package points outside of it, so the package was refused.").into(),
        ExtensionError::MissingFile(file) => tr!("The package has no {file}.", file),
        ExtensionError::Manifest(_) => tr!("Its source.toml has a mistake, so it cannot be installed.").into(),
        ExtensionError::NewerApi(_) => tr!("It was made for a newer Heronite. Update Heronite to install it.").into(),
        ExtensionError::IdTaken(_) => tr!("Its id is the id of a source Heronite has, so it cannot be installed.").into(),
        ExtensionError::Script(_) => tr!("Its script does not load, so it cannot be installed.").into(),
        ExtensionError::NotInstalled(_) => tr!("This extension is not installed any more.").into(),
        ExtensionError::Io(_) => tr!("It could not be saved in the data folder.").into(),
        ExtensionError::Download(_) => tr!("Its files could not be downloaded. Check your connection and try again.").into(),
        ExtensionError::Checksum(_) => tr!("A file of this source is not the one the list names, so nothing was installed.").into(),
        ExtensionError::NotAsListed(_) => tr!(
            "It is not what the list says it is (its id, version, kind, fields, hosts or adult content differ), so nothing was installed."
        )
        .into(),
        ExtensionError::Changed => tr!("The package changed after it was shown, so nothing was installed. Open it again.").into(),
    }
}

/// The engine's own words, for the details under [`error`] (English, for
/// the extension's author).
pub fn detail(e: &ExtensionError) -> Option<String> {
    match e {
        ExtensionError::NewerApi(_) | ExtensionError::NotInstalled(_) => None,
        other => Some(other.to_string()),
    }
}

/// Why a list could not be read or added, in a sentence.
pub fn list_error(e: &ListError) -> String {
    match e {
        ListError::NotHttps => tr!("Only https addresses can be added.").into(),
        ListError::Unreachable(_) => tr!("Could not reach the list. Check your connection.").into(),
        ListError::Status(404) => tr!("There is no list at this address.").into(),
        ListError::Status(status) => tr!("The list's server answered with an error ({status}).", status),
        ListError::TooLarge => tr!("The list is larger than a list may be (1 MB).").into(),
        ListError::Invalid(IndexError::NewerFormat(_)) => tr!("This list was made for a newer Heronite.").into(),
        ListError::Invalid(IndexError::NotJson(_)) => tr!("This address does not hold a list of sources.").into(),
        ListError::Invalid(IndexError::WrongType(_)) => tr!("This list is not a list of metadata sources.").into(),
        ListError::Invalid(IndexError::Invalid(_)) => tr!("The list has mistakes, so it is not used.").into(),
        ListError::AlreadyAdded => tr!("This list is in use already.").into(),
        ListError::Io(_) => tr!("The list could not be saved.").into(),
        ListError::NoFile(_) => tr!("The list's file could not be opened.").into(),
    }
}

/// The technical words of a list's error worth showing (a list's mistake).
pub fn list_detail(e: &ListError) -> Option<String> {
    match e {
        ListError::Invalid(IndexError::Invalid(m)) | ListError::NoFile(m) => Some(m.clone()),
        _ => None,
    }
}

/// Where a source comes from, in a few words ("From the Heronite list").
pub fn origin(o: &Origin) -> String {
    match o {
        Origin::Heronite => tr!("From the Heronite list").into(),
        Origin::List(url) => tr!("From {host}", host = host_of(url)),
        Origin::GoneList(url) => tr!("From {host}, a list you removed", host = host_of(url)),
        Origin::File => tr!("Installed from a file: nobody checked this source").into(),
        Origin::Local(_) => tr!("From a list on this computer").into(),
    }
}

/// What that origin guarantees, for the installer and the gallery.
pub fn guarantee(o: &Origin) -> String {
    match o {
        Origin::Heronite => tr!("Its files are checked against the Heronite list (SHA-256) before anything is installed.").into(),
        Origin::List(_) | Origin::GoneList(_) => tr!(
            "Its files are checked against this list (SHA-256), but Heronite does not know who keeps the list. Install only what you trust."
        )
        .into(),
        Origin::File => tr!("Install it only if you trust where the file comes from.").into(),
        Origin::Local(_) => {
            tr!("Its files are read from the folder of this list and checked against it (SHA-256). Nobody else checked them: install only what you trust.")
                .into()
        }
    }
}

/// Whether the origin deserves the caution colour.
pub fn cautious(o: &Origin) -> bool {
    !matches!(o, Origin::Heronite)
}

/// The origin of a list's entry. The app's own list is the Heronite list
/// only at its address: another one set by `HERONITE_EXTENSIONS_INDEX` is
/// a list like one the user added.
pub fn list_origin(url: &str, builtin: bool) -> Origin {
    if is_local(url) {
        Origin::Local(url.to_owned())
    } else if builtin && url == HERONITE_LIST {
        Origin::Heronite
    } else {
        Origin::List(url.to_owned())
    }
}

/// What a list is called: "The Heronite list", its host, or, for a list
/// on this computer, "This copy of the repository" (`checkout`, a debug
/// build's own list) or "A list on this computer".
pub fn list_name(url: &str, builtin: bool, checkout: bool) -> String {
    match list_origin(url, builtin) {
        Origin::Local(_) if checkout => tr!("This copy of the repository").into(),
        Origin::Local(_) => tr!("A list on this computer").into(),
        Origin::Heronite => tr!("The Heronite list").into(),
        _ => host_of(url),
    }
}

/// Where the entries of one list come from, for the gallery's footer
/// ("From the Heronite list", "From this copy of the repository").
pub fn list_from(url: &str, builtin: bool, checkout: bool) -> String {
    if checkout && is_local(url) {
        tr!("From this copy of the repository").into()
    } else {
        origin(&list_origin(url, builtin))
    }
}

/// "read today", "read yesterday", "read 3 days ago".
pub fn read_ago(at: SystemTime, now: SystemTime) -> String {
    let days = now.duration_since(at).map_or(0, |d| d.as_secs() / 86_400);
    match days {
        0 => tr!("read today").into(),
        1 => tr!("read yesterday").into(),
        n => trn!(n as usize, "read {n} day ago", "read {n} days ago"),
    }
}

/// A kind of source, as the filters and the panel name it.
pub fn kind(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Details => tr!("Game details"),
        SourceKind::Pictures => tr!("Pictures"),
        SourceKind::Extras => tr!("Extras"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        cautious, error, guarantee, list_detail, list_error, list_from, list_name, list_origin, origin, read_ago, ExtensionError,
        IndexError, ListError, Origin, SystemTime, HERONITE_LIST,
    };

    /// Every variant, once. The `match` has no `_`: a new variant fails to
    /// compile here until it has a sentence and is listed.
    fn every_error() -> Vec<ExtensionError> {
        let all = vec![
            ExtensionError::NotAPackage("x".into()),
            ExtensionError::TooLarge("x".into()),
            ExtensionError::UnexpectedFile("x".into()),
            ExtensionError::UnsafePath("x".into()),
            ExtensionError::MissingFile("source.js".into()),
            ExtensionError::Manifest("x".into()),
            ExtensionError::NewerApi(2),
            ExtensionError::IdTaken("igdb".into()),
            ExtensionError::Script("x".into()),
            ExtensionError::NotInstalled("x".into()),
            ExtensionError::Io("x".into()),
            ExtensionError::Download("x".into()),
            ExtensionError::Checksum("source.js".into()),
            ExtensionError::NotAsListed("x".into()),
            ExtensionError::Changed,
        ];
        for e in &all {
            match e {
                ExtensionError::NotAPackage(_)
                | ExtensionError::TooLarge(_)
                | ExtensionError::UnexpectedFile(_)
                | ExtensionError::UnsafePath(_)
                | ExtensionError::MissingFile(_)
                | ExtensionError::Manifest(_)
                | ExtensionError::NewerApi(_)
                | ExtensionError::IdTaken(_)
                | ExtensionError::Script(_)
                | ExtensionError::NotInstalled(_)
                | ExtensionError::Io(_)
                | ExtensionError::Download(_)
                | ExtensionError::Checksum(_)
                | ExtensionError::NotAsListed(_)
                | ExtensionError::Changed => {}
            }
        }
        all
    }

    #[test]
    fn every_engine_error_is_a_sentence_of_its_own() {
        let all = every_error();
        let sentences: Vec<String> = all.iter().map(error).collect();
        for (e, s) in all.iter().zip(&sentences) {
            assert!(s.ends_with('.') && s.len() > 20, "{e:?}: {s}");
            assert_ne!(*s, e.to_string(), "{e:?}: the engine's text is a detail, not the sentence");
            assert!(!s.contains("ExtensionError") && !s.contains("(\""), "{e:?}: {s}");
        }
        let mut unique = sentences.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), sentences.len(), "each error says something different");
    }

    #[test]
    fn not_as_listed_names_what_the_check_compares() {
        // `check_as_listed`: id, version, kind, adult, fields and hosts; never the name.
        let s = error(&ExtensionError::NotAsListed("x".into()));
        for what in ["id", "version", "kind", "fields", "hosts", "adult"] {
            assert!(s.contains(what), "{what}: {s}");
        }
        assert!(!s.contains("name"), "{s}");
    }

    #[test]
    fn every_list_error_is_a_sentence() {
        let all = [
            ListError::NotHttps,
            ListError::Unreachable("x".into()),
            ListError::Status(404),
            ListError::Status(500),
            ListError::TooLarge,
            ListError::Invalid(IndexError::NewerFormat(2)),
            ListError::Invalid(IndexError::NotJson("x".into())),
            ListError::Invalid(IndexError::Invalid("wikipedia: no files".into())),
            ListError::Invalid(IndexError::WrongType("themes".into())),
            ListError::AlreadyAdded,
            ListError::Io("x".into()),
            ListError::NoFile("/x/index.json: No such file or directory".into()),
        ];
        for e in &all {
            match e {
                ListError::NotHttps
                | ListError::Unreachable(_)
                | ListError::Status(_)
                | ListError::TooLarge
                | ListError::Invalid(_)
                | ListError::AlreadyAdded
                | ListError::Io(_)
                | ListError::NoFile(_) => {}
            }
            let s = list_error(e);
            assert!(s.ends_with('.') && s.len() > 15, "{e:?}: {s}");
        }
        assert_eq!(list_detail(&all[7]).as_deref(), Some("wikipedia: no files"));
        assert!(list_error(&ListError::Status(500)).contains("500"));
        assert_eq!(list_detail(&all[11]).as_deref(), Some("/x/index.json: No such file or directory"), "which file, and why");
        assert_eq!(list_error(&all[8]), "This list is not a list of metadata sources.");
    }

    #[test]
    fn the_trust_words_follow_where_it_came_from() {
        assert_eq!(origin(&Origin::Heronite), "From the Heronite list");
        assert_eq!(origin(&Origin::List("https://lists.example.org/a/index.json".into())), "From lists.example.org");
        assert!(origin(&Origin::GoneList("https://lists.example.org/x.json".into())).contains("a list you removed"));
        assert_eq!(origin(&Origin::File), "Installed from a file: nobody checked this source");
        assert!(!cautious(&Origin::Heronite) && cautious(&Origin::File) && cautious(&Origin::List(String::new())));
        assert_eq!(list_origin(HERONITE_LIST, true), Origin::Heronite);
    }

    #[test]
    fn another_address_for_the_apps_own_list_is_not_the_heronite_list() {
        // `HERONITE_EXTENSIONS_INDEX=https://…`: the app's own list, at
        // another address.
        let branch = "https://raw.githubusercontent.com/someone/Heronite/branch/extensions/metadata/index.json";
        let origin = list_origin(branch, true);
        assert_eq!(origin, Origin::List(branch.into()));
        assert!(cautious(&origin) && guarantee(&origin) != guarantee(&Origin::Heronite));
        assert_eq!(list_name(branch, true, false), "raw.githubusercontent.com");
        assert_eq!(list_from(branch, true, false), "From raw.githubusercontent.com");
        assert_eq!(list_name(HERONITE_LIST, true, false), "The Heronite list");
    }

    #[test]
    fn a_list_on_this_computer_is_never_the_heronite_list() {
        let path = "/src/heronite/extensions/metadata/index.json";
        // Even as the app's own list (a debug build's).
        assert_eq!(list_origin(path, true), Origin::Local(path.into()));
        let local = Origin::Local(path.into());
        assert_eq!(origin(&local), "From a list on this computer");
        assert!(cautious(&local) && guarantee(&local) != guarantee(&Origin::Heronite));
        assert_eq!(list_name(path, true, true), "This copy of the repository");
        assert_eq!(list_name(path, true, false), "A list on this computer");
        assert_eq!(list_name(HERONITE_LIST, true, false), "The Heronite list");
        assert_eq!(list_name("https://a.example.org/x.json", false, false), "a.example.org");
        assert_eq!(list_from(path, true, true), "From this copy of the repository");
        assert_eq!(list_from(path, true, false), "From a list on this computer");
        assert_eq!(list_from(HERONITE_LIST, true, true), "From the Heronite list");
    }

    #[test]
    fn a_copy_says_how_old_it_is() {
        let now = SystemTime::now();
        assert_eq!(read_ago(now, now), "read today");
        assert_eq!(read_ago(now - Duration::from_secs(86_400 + 60), now), "read yesterday");
        assert_eq!(read_ago(now - Duration::from_secs(3 * 86_400), now), "read 3 days ago");
    }
}
