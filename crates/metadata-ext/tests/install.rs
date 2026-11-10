//! Installing, updating, removing and listing extensions, and loading them
//! at startup, with what the user set kept.

mod support;

use std::io::Write;
use std::path::Path;

use heronite_data::paths::DataPaths;
use heronite_metadata::sources::{OptionValue, SourceStatus};
use heronite_metadata::MetadataService;
use heronite_metadata_ext::{ExtensionError, ExtensionState, ExtensionsExt, InstallAction, InstallOutcome};
use support::{folder, manifest};

const SCRIPT: &str = "export function find() { return []; }\nexport function fetch() { return null; }\n";

fn body() -> String {
    "[[settings]]\nkey = \"region\"\nlabel = \"Region\"\ntype = \"select\"\noptions = [\"eu\", \"us\"]\n\n[network]\nhosts = [\"api.example.org\"]".into()
}

fn package(parent: &Path, version: &str) -> std::path::PathBuf {
    folder(parent, &format!("pkg-{version}"), &manifest("mydb", version, "details", &["description", "genres"], &body()), SCRIPT)
}

fn zipped(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.with_file_name(name);
    let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for file in ["source.toml", "source.js"] {
        w.start_file(format!("mydb/{file}"), zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(&std::fs::read(dir.join(file)).unwrap()).unwrap();
    }
    w.finish().unwrap();
    path
}

fn region(svc: &MetadataService) -> OptionValue {
    let rows = svc.source_rows(&svc.metadata_settings());
    rows.into_iter().find(|r| r.descriptor.id == "mydb").unwrap().options[0].value.clone()
}

#[test]
fn inspect_install_update_and_remove() {
    let data = tempfile::tempdir().unwrap();
    let pkgs = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(data.path());
    let svc = MetadataService::new(paths.clone());
    let v1 = package(pkgs.path(), "1.0.0");

    // Inspect says what it is and installs nothing.
    let info = svc.inspect_extension(&v1).unwrap();
    assert_eq!((info.id.as_str(), info.version.as_str(), info.action), ("mydb", "1.0.0", InstallAction::New));
    assert_eq!(info.hosts, ["api.example.org"]);
    assert_eq!(info.rate, "1/s");
    assert_eq!(info.provides, ["description", "genres"]);
    assert_eq!(info.settings.len(), 1);
    assert!(!info.adult && info.sha256.len() == 64 && info.installed.is_none());
    assert!(svc.sources().get("mydb").is_none() && svc.installed_extensions().is_empty());

    // Installed: on, listed, registered, its files under the data folder.
    let out = svc.install_extension(&zipped(&v1, "mydb-1.zip"), false).unwrap();
    let InstallOutcome::Installed(ext) = out else { panic!("{out:?}") };
    assert_eq!((ext.version.as_str(), &ext.state), ("1.0.0", &ExtensionState::Loaded));
    assert!(ext.from.ends_with("mydb-1.zip") && !ext.installed_at.is_empty());
    let dir = paths.metadata_extensions_dir().join("mydb");
    assert!(dir.join("source.toml").is_file() && dir.join("source.js").is_file() && dir.join("install.json").is_file());
    assert!(svc.source_on("mydb"));
    assert_eq!(svc.source_status(&svc.metadata_settings(), "mydb"), Some(SourceStatus::On));
    svc.set_source_option("mydb", "region", OptionValue::Text("us".into())).unwrap();
    svc.move_source("mydb", 0).unwrap();

    // The same version again, or an older one, asks first.
    let out = svc.install_extension(&v1, false).unwrap();
    assert_eq!(out, InstallOutcome::NeedsConfirmation { installed: "1.0.0".into(), offered: "1.0.0".into(), downgrade: false });
    assert_eq!(svc.inspect_extension(&v1).unwrap().action, InstallAction::Reinstall);
    let v2 = package(pkgs.path(), "2.0.0");
    assert_eq!(svc.inspect_extension(&v2).unwrap().action, InstallAction::Update);
    let out = svc.install_extension(&v2, false).unwrap();
    assert!(
        matches!(out, InstallOutcome::Updated { ref from, ref extension } if from == "1.0.0" && extension.version == "2.0.0"),
        "{out:?}"
    );
    assert_eq!(region(&svc), OptionValue::Text("us".into()), "an update keeps what the user set");
    assert_eq!(svc.sources().get("mydb").unwrap().extension.as_ref().unwrap().version, "2.0.0");
    assert_eq!(svc.sources().descriptors().filter(|d| d.id == "mydb").count(), 1);
    let old = package(pkgs.path(), "1.5.0");
    assert_eq!(svc.inspect_extension(&old).unwrap().action, InstallAction::Downgrade);
    assert!(matches!(svc.install_extension(&old, false).unwrap(), InstallOutcome::NeedsConfirmation { downgrade: true, .. }));
    assert!(matches!(svc.install_extension(&old, true).unwrap(), InstallOutcome::Updated { .. }));
    assert_eq!(svc.installed_extensions()[0].version, "1.5.0");

    // A new service (the next start) loads it.
    let next = MetadataService::new(paths.clone());
    assert!(next.sources().get("mydb").is_none());
    let loaded = next.load_extensions();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].state, ExtensionState::Loaded);
    assert_eq!(region(&next), OptionValue::Text("us".into()));

    // Removed: gone from the registry and the disk; the settings stay.
    svc.remove_extension("mydb", false).unwrap();
    assert!(svc.sources().get("mydb").is_none() && !dir.exists());
    assert!(svc.installed_extensions().is_empty());
    assert_eq!(svc.remove_extension("mydb", false), Err(ExtensionError::NotInstalled("mydb".into())));
    assert!(svc.metadata_settings().source_order.iter().any(|s| s == "mydb"));
    svc.install_extension(&v1, false).unwrap();
    assert_eq!(region(&svc), OptionValue::Text("us".into()), "back with its settings");
    // Unless asked to forget them.
    svc.remove_extension("mydb", true).unwrap();
    let m = svc.metadata_settings();
    assert!(!m.sources.contains_key("mydb") && !m.source_order.iter().any(|s| s == "mydb"));
}

#[test]
fn a_package_that_does_not_load_is_never_installed() {
    let data = tempfile::tempdir().unwrap();
    let pkgs = tempfile::tempdir().unwrap();
    let svc = MetadataService::new(DataPaths::at(data.path()));
    let bad_script = folder(pkgs.path(), "bad", &manifest("bad", "1.0.0", "details", &["description"], ""), "export function find( {");
    assert!(matches!(svc.inspect_extension(&bad_script), Err(ExtensionError::Script(_))));
    assert!(matches!(svc.install_extension(&bad_script, true), Err(ExtensionError::Script(_))));
    let newer =
        folder(pkgs.path(), "newer", &manifest("newer", "1.0.0", "details", &["description"], "").replace("api = 1", "api = 2"), SCRIPT);
    assert_eq!(svc.install_extension(&newer, true), Err(ExtensionError::NewerApi(2)));
    let taken = folder(pkgs.path(), "taken", &manifest("steam", "1.0.0", "details", &["description"], ""), SCRIPT);
    assert_eq!(svc.install_extension(&taken, true), Err(ExtensionError::IdTaken("steam".into())));
    assert!(svc.installed_extensions().is_empty());
    assert!(!DataPaths::at(data.path()).metadata_extensions_dir().join("bad").exists());
}

#[test]
fn an_installed_extension_that_no_longer_loads_shows_why() {
    let data = tempfile::tempdir().unwrap();
    let pkgs = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(data.path());
    let svc = MetadataService::new(paths.clone());
    svc.install_extension(&package(pkgs.path(), "1.0.0"), false).unwrap();
    // Its files changed on disk: a contract this build does not speak, and
    // another extension whose script broke.
    let dir = paths.metadata_extensions_dir().join("mydb");
    let text = std::fs::read_to_string(dir.join("source.toml")).unwrap();
    std::fs::write(dir.join("source.toml"), text.replace("api = 1", "api = 9")).unwrap();
    let other =
        folder(&paths.metadata_extensions_dir(), "other", &manifest("other", "1.0.0", "details", &["description"], ""), "syntax error (");
    assert!(other.is_dir());
    // A half-written install of a crash is swept.
    std::fs::create_dir_all(paths.metadata_extensions_dir().join(".tmp-x-1-2")).unwrap();

    let next = MetadataService::new(paths.clone());
    let loaded = next.load_extensions();
    assert_eq!(loaded.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["mydb", "other"]);
    assert!(loaded.iter().all(|e| matches!(e.state, ExtensionState::LoadFailed(_))), "{loaded:?}");
    let m = next.metadata_settings();
    assert_eq!(next.source_status(&m, "mydb"), Some(SourceStatus::LoadFailed));
    let row = next.source_rows(&m).into_iter().find(|r| r.descriptor.id == "other").unwrap();
    assert_eq!(row.status, SourceStatus::LoadFailed);
    assert_eq!(row.descriptor.name, "other", "the manifest still names it");
    assert!(row.problem.is_some_and(|p| p.contains("source.js")));
    assert!(!paths.metadata_extensions_dir().join(".tmp-x-1-2").exists());
    // Updating it fixes it.
    let v2 = package(pkgs.path(), "2.0.0");
    next.install_extension(&v2, false).unwrap();
    assert_eq!(next.source_status(&next.metadata_settings(), "mydb"), Some(SourceStatus::On));
}
