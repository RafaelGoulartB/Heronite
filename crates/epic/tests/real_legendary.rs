//! Opt-in checks against the real legendary and the real Epic account
//! (READ-ONLY commands: `list`, `info`). Needs a data folder whose
//! `legendaryConfig` is a symlink to the real one (legendary rotates its login
//! token, so it must never run on a copy):
//!
//! `HERONITE_EPIC_FIXTURE=/tmp/heronite-fixture-epic cargo test -p heronite-epic --test real_legendary -- --nocapture`

use heronite_data::DataPaths;
use heronite_epic::cmd::Legendary;
use heronite_epic::{info, library, user};

fn fixture() -> Option<DataPaths> {
    std::env::var_os("HERONITE_EPIC_FIXTURE").map(DataPaths::at)
}

#[test]
fn account_library_refresh_and_info() {
    let Some(data_paths) = fixture() else {
        eprintln!("HERONITE_EPIC_FIXTURE not set; skipping");
        return;
    };
    let legendary = Legendary::locate(&data_paths).expect("legendary binary");
    assert!(user::is_logged_in(&data_paths), "the fixture's legendaryConfig must hold a login");
    let account = user::sync_user_info(&data_paths).unwrap().expect("user info");
    println!("logged in as {}", account.display_name);

    let r = library::refresh(&data_paths, &legendary).unwrap();
    println!("refresh: {} games, list error: {:?}", r.games, r.list_error);
    assert!(r.list_error.is_none(), "legendary list failed: {:?}", r.list_error);
    assert!(r.games > 0);
    println!("updates available: {:?}", library::updates_available(&data_paths));

    // An owned game that is not installed: sizes come from the manifest.
    let games = library::build_library(&data_paths);
    let game = games
        .iter()
        .find(|g| g["is_installed"] == false && g["install"]["is_dlc"] == false)
        .expect("a game that is not installed");
    let app = game["app_name"].as_str().unwrap();
    let i = info::install_info(&data_paths, &legendary, app, "Windows").unwrap();
    println!("{app}: download {} disk {} dlc {}", i.download_size(), i.disk_size(), i.owned_dlc().len());
    assert!(i.download_size() > 0.);

    // The legendary API overrides (and SDL of a game that has any).
    match info::game_override(&data_paths) {
        Ok(over) => {
            let with_sdl = over["sdl_config"].as_object().and_then(|m| m.keys().next().cloned());
            println!("api overrides ok; first sdl game: {with_sdl:?}");
            if let Some(app) = with_sdl {
                let sdl = info::game_sdl(&data_paths, &app);
                println!("sdl of {app}: {:?}", sdl.iter().map(|s| (&s.name, s.required)).collect::<Vec<_>>());
            }
        }
        Err(err) => println!("api overrides unavailable: {err:#}"),
    }
}
