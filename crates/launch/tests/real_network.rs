//! Opt-in checks against the real network (GitHub, Lutris), downloading into
//! a temporary Heroic folder only. Run with
//! `cargo test -p heronite-launch --test real_network -- --ignored --nocapture`.

use heronite_data::DataPaths;
use heronite_launch::net::SystemNet;
use heronite_launch::runtimes::{self, Runtime};
use heronite_launch::tools::{self, Tool, Updated};

fn ls(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

#[test]
#[ignore = "downloads from the internet"]
fn tools_and_runtimes_land_in_heroics_layout() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heroic"));
    let net = SystemNet;
    let vulkan = tools::detect_vulkan();
    println!("vulkan: {vulkan:?}");
    for tool in Tool::ALL {
        let got = tools::install_or_update(&paths, &net, tool, vulkan, &|| false).unwrap();
        let Updated::Installed(version) = got else { panic!("{got:?}") };
        let release = tools::tool_dir(&paths, tool).join(&version);
        println!("{} {version}: {:?} / {:?}", tool.name(), ls(&release), ls(&release.join("x64")));
        assert!(release.join("x64").is_dir(), "{}", tool.name());
        assert!(release.join(tool.dir32()).is_dir(), "{}", tool.name());
        assert_eq!(tools::installed_version(&paths, tool), Some(version));
    }
    assert!(tools::download_winetricks(&paths, &net, &|| false).unwrap());
    for rt in [Runtime::Umu, Runtime::Eac, Runtime::BattlEye] {
        assert!(runtimes::needs_download(&paths, &net, rt));
        runtimes::download(&paths, &net, rt, &|| false).unwrap();
        println!("{}: {:?}", rt.name(), ls(&runtimes::runtimes_dir(&paths).join(rt.name())));
        assert!(!runtimes::needs_download(&paths, &net, rt));
    }
    assert!(runtimes::umu_script(&paths).exists());
}

#[test]
#[ignore = "downloads from the internet"]
fn the_startup_update_installs_every_tool_once() {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::at(dir.path().join("heroic"));
    let said = std::sync::Mutex::new(Vec::new());
    tools::update_all(&paths, &SystemNet, &|| false, &|m| said.lock().unwrap().push(m.to_string()));
    for tool in Tool::ALL {
        assert!(tools::installed_dir(&paths, tool).is_some(), "{}", tool.name());
    }
    assert!(paths.tools_dir().join("winetricks").exists());
    assert_eq!(said.lock().unwrap().len(), 3, "{:?}", said.lock().unwrap());
    // A second run finds everything current and says nothing.
    let again = std::sync::Mutex::new(0);
    tools::update_all(&paths, &SystemNet, &|| false, &|_| *again.lock().unwrap() += 1);
    assert_eq!(*again.lock().unwrap(), 0);
}
