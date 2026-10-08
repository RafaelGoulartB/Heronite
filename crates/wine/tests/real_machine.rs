//! Read-only check against the machine's real Heroic/Steam folders. Skipped
//! when they do not exist (CI). Never writes.

use heronite_data::HeroicPaths;

#[test]
fn real_machine_detection() {
    let heroic = HeroicPaths::detect();
    if !heroic.tools_dir().join("proton").exists() {
        eprintln!("skipped: no real Heroic tools folder");
        return;
    }
    let list = heronite_wine::list_installations(&heroic);
    for w in &list {
        eprintln!("{:8} {:32} {}", w.kind, w.name, w.bin);
    }
    assert!(list.iter().any(|w| w.name == "GE-Proton-latest" && w.kind == "proton"));
    if let Some(home) = dirs_home() {
        if home.join(".local/share/Steam/compatibilitytools.d/Proton-CachyOS Latest/proton").exists() {
            assert!(list.iter().any(|w| w.name == "Proton-CachyOS Latest"));
        }
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(Into::into)
}
